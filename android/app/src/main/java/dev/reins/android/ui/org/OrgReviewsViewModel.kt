package dev.reins.android.ui.org

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dev.reins.android.profiles.DeviceKeys
import dev.reins.android.profiles.OrgSigning
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.profiles.orgSign
import dev.reins.android.ui.common.userMessage
import dev.reins.core.CoreException
import dev.reins.core.OrgPasskeyView
import dev.reins.core.OrgReviewView
import dev.reins.core.ReinsCoreInterface
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * [reviews] null while loading; [passkeys] the web passkeys that wait for this phone's confirmation; [busy] the review
 * (or passkey) whose answer is on its way.
 */
data class OrgReviewsUi(
    val reviews: List<OrgReviewView>? = null,
    val passkeys: List<OrgPasskeyView> = emptyList(),
    val busy: String? = null,
    val error: String? = null,
)

/**
 * A work profile's org reviews (O2): the core lists them, checked and described on the phone; an answer is signed with
 * the key this phone enrolled to the org, over the core's review message, then sent. A web passkey the member added in
 * the browser waits here too, until this phone confirms it (O3). The Personal profile has none and asks nothing.
 */
class OrgReviewsViewModel(
    private val core: ReinsCoreInterface,
    private val profile: WorkProfiles.Active,
    private val keys: DeviceKeys,
    private val now: () -> Long = { System.currentTimeMillis() / 1000 },
) : ViewModel() {
    private val _ui = MutableStateFlow(OrgReviewsUi())
    val ui: StateFlow<OrgReviewsUi> = _ui.asStateFlow()

    private var orgId: String? = null

    fun load() {
        if (!profile.managed) {
            _ui.value = OrgReviewsUi(reviews = emptyList())
            return
        }
        viewModelScope.launch {
            try {
                val reviews = core.orgReviews()
                val passkeys = waitingPasskeys()
                _ui.update { it.copy(reviews = reviews, passkeys = passkeys, error = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException) {
                _ui.update { it.copy(reviews = it.reviews ?: emptyList(), error = e.userMessage()) }
            }
        }
    }

    /** The member's web passkeys not yet confirmed; none when the server has no web approvals. */
    private suspend fun waitingPasskeys(): List<OrgPasskeyView> = try {
        core.orgPasskeys().filterNot { it.confirmed }
    } catch (e: CancellationException) {
        throw e
    } catch (e: CoreException) {
        android.util.Log.w("Reins", "Could not list the org passkeys", e)
        emptyList()
    }

    private suspend fun org(): String =
        orgId ?: core.orgMembership()?.orgId?.also { orgId = it } ?: throw CoreException.NotFound()

    /**
     * Confirm (this passkey is the member's own) or refuse [key], signed with the enrolled key over the core's message of
     * the key hash and registration time the card shows the fingerprint of. Refusing revokes it.
     */
    fun confirmPasskey(key: OrgPasskeyView, approve: Boolean) {
        if (!profile.managed || _ui.value.busy != null) return
        _ui.update { it.copy(busy = key.credentialId, error = null) }
        val gone = { s: OrgReviewsUi -> s.copy(passkeys = s.passkeys.filterNot { it.credentialId == key.credentialId }, busy = null) }
        viewModelScope.launch {
            try {
                val org = org()
                val at = now()
                val signature = withContext(Dispatchers.IO) {
                    orgSign(keys, profile.id, OrgSigning.passkeyMessage(org, key, at))
                }
                core.orgConfirmPasskey(key.credentialId, approve, at, signature)
                _ui.update(gone)
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException.NotFound) {
                _ui.update(gone)
            } catch (e: CoreException) {
                _ui.update { it.copy(busy = null, error = e.userMessage()) }
            } catch (e: Exception) {
                android.util.Log.w("Reins", "Could not sign a passkey confirmation", e)
                _ui.update { it.copy(busy = null, error = "This phone can't sign for your organization.") }
            }
        }
    }

    /** Approve (or, for your own change, confirm) or deny [review]. */
    fun answer(review: OrgReviewView, approve: Boolean) {
        if (!profile.managed || _ui.value.busy != null) return
        _ui.update { it.copy(busy = review.id, error = null) }
        viewModelScope.launch {
            try {
                val org = org()
                val at = now()
                val signature = withContext(Dispatchers.IO) {
                    orgSign(keys, profile.id, OrgSigning.message(org, review, approve, at))
                }
                core.orgDecide(review.id, approve, at, signature)
                _ui.update { s -> s.copy(reviews = s.reviews?.filterNot { it.id == review.id }, busy = null) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException.NotFound) {
                // Answered elsewhere, expired, or no longer checks out: it is gone.
                _ui.update { s -> s.copy(reviews = s.reviews?.filterNot { it.id == review.id }, busy = null) }
            } catch (e: CoreException) {
                _ui.update { it.copy(busy = null, error = e.userMessage()) }
            } catch (e: Exception) {
                // The Keystore refused (the key is gone: this phone is no longer enrolled).
                android.util.Log.w("Reins", "Could not sign an org review", e)
                _ui.update { it.copy(busy = null, error = "This phone can't sign for your organization.") }
            }
        }
    }
}
