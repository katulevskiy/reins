package dev.rewarden.android

import android.content.Intent
import android.os.Bundle
import android.view.MotionEvent
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.fragment.app.FragmentActivity
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.lifecycle.viewmodel.initializer
import dev.rewarden.android.platform.AppNotifier
import dev.rewarden.android.platform.Foreground
import dev.rewarden.android.platform.GrantReminders
import dev.rewarden.android.platform.McpRedirectActivity
import dev.rewarden.android.ui.mcp.isMcpRedirect
import dev.rewarden.android.platform.AuthenticatorProvider
import dev.rewarden.android.platform.update.UpdateNotifier
import dev.rewarden.android.platform.update.UpdateWorker
import dev.rewarden.android.state.SessionState
import dev.rewarden.android.sync.ForegroundSync
import dev.rewarden.android.ui.AppViewModel
import dev.rewarden.android.ui.RewardenRoot
import dev.rewarden.android.ui.nav.DeepLink
import dev.rewarden.android.ui.pairing.PairingCode
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** The only exported component. FragmentActivity because BiometricPrompt needs one. */
class MainActivity : FragmentActivity() {
    private val container get() = (application as RewardenApp).container
    private val app: AppViewModel by viewModels { viewModelFactory { initializer { AppViewModel(container) } } }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val authenticator = AuthenticatorProvider.factory(this)
        setContent {
            RewardenRoot(container, app, authenticator)
        }
        if (savedInstanceState == null) handleIntent(intent)
        if (container.updates != null) UpdateWorker.schedule(applicationContext)

        // Long-poll only while the screen is on and someone is signed in.
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                container.state.session.first { it is SessionState.SignedIn }
                container.refreshPending()
                ForegroundSync(container).run()
            }
        }
    }

    /** Sounds and haptics play while the activity is started (dialogs and the biometric prompt take focus, not that). */
    override fun onStart() {
        super.onStart()
        container.feedback.setForeground(true)
    }

    override fun onStop() {
        container.feedback.setForeground(false)
        super.onStop()
    }

    /** A finger going down anywhere wakes the sound output, so the click that sounds is not late. */
    override fun dispatchTouchEvent(ev: MotionEvent): Boolean {
        if (ev.actionMasked == MotionEvent.ACTION_DOWN) container.feedback.onTouchDown()
        return super.dispatchTouchEvent(ev)
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        Foreground.focused = hasFocus && lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
        if (Foreground.focused) {
            app.onForeground()
            lifecycleScope.launch { container.refreshPending() }
        }
    }

    /** Also the return from Android's "install unknown apps" screen, where an update may be waiting to install. */
    override fun onResume() {
        super.onResume()
        container.updates?.onForeground()
    }

    override fun onPause() {
        Foreground.focused = false
        super.onPause()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    /** Any app can send this activity an intent, so only a well-formed link to a really pending item opens anything. */
    private fun handleIntent(intent: Intent?) {
        if (intent?.action == UpdateNotifier.ACTION_OPEN_UPDATE) {
            val updates = container.updates ?: return
            // The prompt sits on the main screen; an open approval sheet stays and the prompt waits for it.
            app.home()
            updates.openFromNotification()
            return
        }
        if (intent?.action == GrantReminders.ACTION_OPEN_GRANT) {
            val id = intent.getStringExtra(GrantReminders.EXTRA_GRANT_ID)
            if (id != null && DeepLink.isId(id)) {
                lifecycleScope.launch {
                    container.state.session.first { it !is SessionState.Loading }
                    app.openGrant(id)
                }
            }
            return
        }
        if (intent?.action == AppNotifier.ACTION_OPEN_ACTIVITY) {
            val id = intent.getLongExtra(AppNotifier.EXTRA_ACTIVITY_ID, -1)
            // Only ever an id to look up in the phone's own history; an unknown one shows "not found".
            if (id > 0) {
                lifecycleScope.launch {
                    container.state.session.first { it is SessionState.SignedIn }
                    app.openActivityEntry(id)
                }
            }
            return
        }
        if (intent?.action == AppNotifier.ACTION_OPEN_AUTOPILOT) {
            lifecycleScope.launch {
                container.state.session.first { it is SessionState.SignedIn }
                app.openAutopilot()
            }
            return
        }
        if (intent?.action == Intent.ACTION_VIEW) {
            // A `/pair` App Link or `reins://pair`: only the code in it is used, and only when it is well-formed. The
            // pairing it names still has to be confirmed in the sheet like any other.
            val code = PairingCode.parse(intent?.dataString)
            if (code != null) {
                lifecycleScope.launch {
                    container.state.session.first { it !is SessionState.Loading }
                    app.openPairingLink(code)
                }
            }
            return
        }
        if (intent?.action == McpRedirectActivity.ACTION_SIGNED_IN) {
            val redirect = intent.dataString
            if (redirect != null && isMcpRedirect(redirect)) {
                lifecycleScope.launch {
                    container.state.session.first { it !is SessionState.Loading }
                    app.finishMcpSignIn(redirect)
                }
            }
            return
        }
        val link = DeepLink.parse(
            action = intent?.action,
            kind = intent?.getStringExtra(AppNotifier.EXTRA_KIND),
            id = intent?.getStringExtra(AppNotifier.EXTRA_ID),
            expectedAction = AppNotifier.ACTION_OPEN_ITEM,
        ) ?: return
        lifecycleScope.launch {
            container.state.session.first { it !is SessionState.Loading }
            app.handleDeepLink(link)
        }
    }
}
