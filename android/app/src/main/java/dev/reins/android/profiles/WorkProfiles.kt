package dev.reins.android.profiles

import android.app.Activity
import android.content.Context
import android.content.Intent
import dev.reins.android.MainActivity
import dev.reins.android.core.MainSafeCore
import dev.reins.android.platform.KeystoreKeyWrapper
import dev.reins.core.AccountProfile
import dev.reins.core.ProfileKind
import dev.reins.core.ProfileRegistry
import dev.reins.core.ProfileRegistryInterface
import dev.reins.core.ReinsCoreInterface

/**
 * The profiles on this phone (Decisions v2 O1, 06 §2): the Personal account and a Managed work profile per
 * organization. Each profile runs its own core on its own data directory, its data key wrapped by its own Keystore
 * alias, and posts to its own notification channels; the app shows one profile at a time and switches in Settings.
 */
object WorkProfiles {
    /** The Personal profile's data-key alias (today's: nothing moves for anyone without a work profile). */
    const val DEK_ALIAS = "reins_dek_wrap_v1"

    /** What the running app is showing. */
    data class Active(
        val id: String,
        val label: String,
        val managed: Boolean,
        val channelPrefix: String,
        val keyAlias: String,
    )

    val PERSONAL = Active("personal", "Personal", managed = false, channelPrefix = "", keyAlias = "")

    /** Set once the app knows (before any notification is posted); Personal until then. */
    @Volatile var active: Active = PERSONAL

    /** Whether this phone has a work profile (then pairing says which profile an agent joins). */
    @Volatile var hasWork: Boolean = false

    fun of(p: AccountProfile) = Active(p.id, label(p), p.kind == ProfileKind.MANAGED, p.channelPrefix, p.keyAlias)

    /** A profile's label as shown anywhere: the org typed it, so it goes through `untrustedName` (the core already
     *  stripped control, bidi and invisible characters and kept it to 40). */
    fun label(p: AccountProfile): String = dev.reins.android.ui.common.untrustedName(p.label, 40)

    /** The Keystore alias that wraps a profile's data key: today's for Personal, its own for each work profile. */
    fun dekAlias(keyAlias: String): String = if (keyAlias.isEmpty()) DEK_ALIAS else "$DEK_ALIAS.$keyAlias"

    /** The Keystore alias of a work profile's org device key (made at enrollment). */
    fun orgKeyAlias(profileId: String): String = "reins_org_device.$profileId"

    /** The badge on cards and rows: the org's name in a work profile, none in the Personal one. */
    fun badge(active: Active = this.active): String? = active.label.takeIf { active.managed }

    /** What the device key signs to enroll (`reins_proto::org::enrollment_message`). */
    fun enrollmentMessage(challenge: String): ByteArray = "reins/org-enroll/v1\n".toByteArray() + challenge.toByteArray()
}

/** Makes the profile registry (tests: a fake; the real one needs the native core). */
fun interface RegistryFactory {
    fun create(context: Context): ProfileRegistryInterface
}

/** Makes a core for a work profile while it signs in (tests: a fake). */
fun interface WorkCoreFactory {
    fun create(context: Context, profile: AccountProfile): ReinsCoreInterface
}

object ProfilesProvider {
    /** Setting it (tests) drops the registry made before. */
    @Volatile var registry: RegistryFactory = RegistryFactory { ProfileRegistry(it.noBackupFilesDir.absolutePath) }
        set(value) {
            field = value
            cached = null
        }

    @Volatile private var cached: ProfileRegistryInterface? = null

    /** The one registry of this process. */
    fun registryFor(context: Context): ProfileRegistryInterface =
        cached ?: synchronized(this) { cached ?: registry.create(context).also { cached = it } }

    @Volatile var workCore: WorkCoreFactory = WorkCoreFactory { context, profile -> realCore(context, profile) }

    @Volatile var deviceKeys: DeviceKeys = KeystoreDeviceKeys()

    private fun realCore(context: Context, profile: AccountProfile): ReinsCoreInterface = MainSafeCore {
        java.io.File(profile.dataDir).mkdirs()
        val app = context.applicationContext as dev.reins.android.ReinsApp
        dev.reins.core.ReinsCore(
            profile.dataDir,
            KeystoreKeyWrapper(WorkProfiles.dekAlias(profile.keyAlias)),
            app.container.google,
            app.container.notifier,
            null,
            dev.reins.android.BuildConfig.TELEGRAM_API_ID,
            dev.reins.android.BuildConfig.TELEGRAM_API_HASH,
        )
    }
}

/** Switching profiles starts the app again on the other profile's core (one core per process). */
object ProfileSwitch {
    /** Tests replace the restart. */
    @Volatile var restart: (Activity) -> Unit = { activity ->
        activity.startActivity(
            Intent(activity, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK),
        )
        Runtime.getRuntime().exit(0)
    }

    fun switchTo(activity: Activity, registry: ProfileRegistryInterface, id: String) {
        registry.setActive(id)
        restart(activity)
    }
}
