package dev.reins.android.ui.settings

import androidx.compose.runtime.Composable
import dev.reins.android.AppContainer
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.userMessage
import kotlin.coroutines.cancellation.CancellationException

/**
 * "Make a new recovery code": after the screen lock, the core replaces the account secret, and the new code goes to the
 * same must-write-it-down sheet as a new account's. Null when it worked (or the user backed out); else what went wrong.
 */
suspend fun rotateRecoveryCode(container: AppContainer, authenticator: Authenticator): String? = try {
    when (authenticator.authenticate("Make a new recovery code", "The current one stops working")) {
        AuthResult.Success -> {
            val code = container.core.rotateRecoveryCode()
            container.feedback.play(Event.GrantCreated)
            container.state.setRecoveryToRecord(code)
            null
        }
        AuthResult.Cancelled -> null
        AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone to make a new recovery code."
    }
} catch (e: CancellationException) {
    throw e
} catch (e: Exception) {
    container.feedback.play(Event.Error)
    e.userMessage()
}

/** What making a new code does, asked before it happens. */
@Composable
fun RotateRecoveryDialog(onConfirm: () -> Unit, onDismiss: () -> Unit) {
    ConfirmDialog(
        title = "Make a new recovery code?",
        text = "The current code stops working everywhere: on a lost phone that kept it, on your other phones (add them " +
            "again from this one to give them the new code), and for opening the vault on a new phone. Your vault " +
            "passkeys are removed; add them again. You write the new code down next.",
        confirmLabel = "Make a new code",
        onConfirm = onConfirm,
        onDismiss = onDismiss,
        destructive = false,
    )
}
