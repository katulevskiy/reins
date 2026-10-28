package dev.reins.android.push

import dev.reins.android.profiles.WorkProfiles
import dev.reins.core.CoreException
import dev.reins.core.ReinsCoreInterface
import kotlin.coroutines.cancellation.CancellationException

enum class PushOutcome { DONE, RETRY }

/** Which profile a push is for. */
sealed interface PushOwner {
    /** The running profile: its core handles it. */
    data object Here : PushOwner

    /** Another profile on this phone, or one this phone can't place: the running core never sees it. */
    data class Elsewhere(val profile: WorkProfiles.Active) : PushOwner
}

/** What a push does, independent of WorkManager so it can be tested on the JVM. */
class PushHandler(
    private val core: ReinsCoreInterface,
    private val onReplaced: suspend () -> Unit,
    /** Places a push by its tag ([PushPayload.profileTag]). */
    private val ownerOf: suspend (String?) -> PushOwner = { PushOwner.Here },
    /** A push for another profile: a plain "Open Reins" on that profile, nothing else. */
    private val onElsewhere: (WorkProfiles.Active) -> Unit = {},
    /** An org review waits: notify it for the profile it is for (null: this one), opening that profile's reviews. */
    private val onOrgReview: (WorkProfiles.Active?, String) -> Unit = { _, _ -> },
    /** A web passkey waits for this phone's confirmation: notified like a review, opening the same screen. */
    private val onOrgPasskey: (WorkProfiles.Active?, String) -> Unit = { _, _ -> },
    /** Held emails that went too late now ask "Still send?" ([n] of them): an ask, so it may be notified. */
    private val onStillSend: (Int) -> Unit = {},
) {
    suspend fun handle(payload: PushPayload): PushOutcome {
        val owner = ownerOf(payload.profileTag)
        if (payload.kind == PushPayload.ORG_REVIEW) {
            onOrgReview((owner as? PushOwner.Elsewhere)?.profile, payload.id)
            return PushOutcome.DONE
        }
        if (payload.kind == PushPayload.ORG_PASSKEY) {
            onOrgPasskey((owner as? PushOwner.Elsewhere)?.profile, payload.id)
            return PushOutcome.DONE
        }
        if (owner is PushOwner.Elsewhere) {
            // Not this profile's: no fetch, no "replaced", nothing on this core. A hold push is silent wherever it is
            // for: that profile releases its held email when it next runs (asking "Still send?" if that is too late).
            if (payload.kind != PushPayload.HOLD) onElsewhere(owner.profile)
            return PushOutcome.DONE
        }
        if (payload.kind == "replaced") onReplaced()
        return try {
            if (payload.kind == PushPayload.HOLD) {
                // Silent (08 R10): what is due goes now; a send held automatically is never notified. Only one that
                // waited too long and now asks the owner says so.
                val released = core.releaseHolds()
                if (released.late > 0u) onStillSend(released.late.toInt())
            } else {
                core.handlePush(payload.kind, payload.id)
            }
            PushOutcome.DONE
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            when (e) {
                // Transient: the network or the server had a hiccup. The foreground poll will also pick it up.
                is CoreException.Network -> PushOutcome.RETRY
                is CoreException.Server -> if (e.status.toInt() >= 500) PushOutcome.RETRY else PushOutcome.DONE
                else -> PushOutcome.DONE
            }
        }
    }

    companion object {
        /** The app's handler: a "replaced" push marks this phone as no longer approving and says so. */
        fun of(container: dev.reins.android.AppContainer) = PushHandler(
            container.core,
            onReplaced = {
                container.markReplaced()
                container.notifier.deviceReplaced()
            },
            ownerOf = { tag -> PushRouting.ownerOf(container, tag) },
            onElsewhere = container.notifier::otherProfile,
            onOrgReview = { target, id -> container.notifier.orgReview(target ?: container.profile, id) },
            onOrgPasskey = { target, id -> container.notifier.orgPasskey(target ?: container.profile, id) },
            onStillSend = container.notifier::stillSend,
        )
    }
}

/**
 * Each profile's push tag, kept on this phone (not secret: the server sends it in the clear), and the rule that places a
 * push: no tag, or a phone without a work profile, is the running profile's (as before profiles); the running profile's
 * own tag is its; another known profile's tag is that one's; a tag nobody here knows is never the running core's.
 */
object PushRouting {
    private const val PREFS = "push_tags"

    private fun prefs(context: android.content.Context) =
        context.applicationContext.getSharedPreferences(PREFS, android.content.Context.MODE_PRIVATE)

    fun remember(context: android.content.Context, profileId: String, tag: String) {
        val prefs = prefs(context)
        if (prefs.getString(profileId, null) != tag) prefs.edit().putString(profileId, tag).apply()
    }

    fun forget(context: android.content.Context, profileId: String) = prefs(context).edit().remove(profileId).apply()

    suspend fun ownerOf(container: dev.reins.android.AppContainer, tag: String?): PushOwner {
        if (tag == null) return PushOwner.Here
        val profiles = runCatching { container.profiles.list() }.getOrNull()
        if (profiles != null && profiles.none { it.kind == dev.reins.core.ProfileKind.MANAGED }) return PushOwner.Here
        val context = container.appContext
        val here = container.profile.id
        val mine = prefs(context).getString(here, null)
            ?: runCatching { container.core.pushTag() }.getOrNull()?.also { remember(context, here, it) }
        if (tag == mine) return PushOwner.Here
        val other = prefs(context).all.entries.firstOrNull { it.value == tag && it.key != here }?.key
        val profile = other?.let { id -> profiles?.firstOrNull { it.id == id }?.let(WorkProfiles::of) }
        return PushOwner.Elsewhere(profile ?: container.profile)
    }
}
