package dev.reins.android.platform

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.reins.android.ReinsApp
import dev.reins.android.ui.common.userMessage
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch

/**
 * Approve and Deny on a request's notification. Approve is only offered for what the core marks `quick` (never the hard
 * floor, and the core refuses it there anyway) and only fires once the phone is unlocked (`setAuthenticationRequired`).
 * Not exported; only the notification's intents reach it.
 */
class ApprovalActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val id = intent.getStringExtra(AppNotifier.EXTRA_ID) ?: return
        val approve = when (intent.action) {
            ACTION_APPROVE -> true
            ACTION_DENY -> false
            else -> return
        }
        val container = (context.applicationContext as ReinsApp).container
        val pending = goAsync()
        container.appScope.launch {
            try {
                if (approve) container.core.approveQuick(id) else container.core.deny(id)
                container.refreshAfterAnswer(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                container.notifier.answerFailed(id, approve, e.userMessage())
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_APPROVE = "dev.reins.android.APPROVE_REQUEST"
        const val ACTION_DENY = "dev.reins.android.DENY_REQUEST"
    }
}
