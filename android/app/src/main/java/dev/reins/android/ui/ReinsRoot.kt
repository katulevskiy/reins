package dev.reins.android.ui

import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.DisposableEffect
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.ViewModelStoreOwner
import androidx.lifecycle.viewmodel.compose.LocalViewModelStoreOwner
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.reins.android.AppContainer
import dev.reins.android.design.LocalColors
import dev.reins.android.design.ReinsTheme
import dev.reins.android.design.Spinner
import dev.reins.android.feedback.Cue
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.ProvideFeedback
import dev.reins.android.feedback.cueUnlessRecent
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.PasskeyPrompt
import dev.reins.android.state.SessionState
import dev.reins.android.ui.activity.ActivityDetailScreen
import dev.reins.android.ui.activity.ActivityScreen
import dev.reins.android.ui.activity.EmailScreen
import dev.reins.android.ui.approval.ApprovalSheet
import dev.reins.android.ui.approval.ApprovalViewModel
import dev.reins.android.ui.autopilot.AutopilotScreen
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.ProfileScreen
import dev.reins.android.ui.autopilot.TryItScreen
import dev.reins.android.ui.common.LocalConnections
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.gmail.GmailScreen
import dev.reins.android.ui.gmail.GmailViewModel
import dev.reins.android.ui.gmail.IntegrationsScreen
import dev.reins.android.ui.grants.GrantDetailScreen
import dev.reins.android.ui.grants.GrantsScreen
import dev.reins.android.ui.grants.deleteGrant
import dev.reins.android.ui.grants.resumeGrant
import dev.reins.android.ui.grants.NewGrantScreen
import dev.reins.android.ui.grants.NewGrantViewModel
import dev.reins.android.ui.join.JoinSheet
import dev.reins.android.ui.join.JoinViewModel
import dev.reins.android.ui.main.FloatingNavBar
import dev.reins.android.ui.mcp.McpAddScreen
import dev.reins.android.ui.mcp.McpServerScreen
import dev.reins.android.ui.mcp.McpViewModel
import dev.reins.android.ui.upload.UploadSheet
import dev.reins.android.ui.upload.UploadViewModel
import dev.reins.android.ui.nav.Route
import dev.reins.android.ui.nav.SheetTarget
import dev.reins.android.ui.nav.Tab
import dev.reins.android.ui.pairing.ConnectComputerScreen
import dev.reins.android.ui.pairing.PairingSheet
import dev.reins.android.ui.pairing.PairingViewModel
import dev.reins.android.ui.settings.ConnectionDetailScreen
import dev.reins.android.ui.settings.SettingsScreen
import dev.reins.android.ui.settings.SettingsViewModel
import dev.reins.android.ui.settings.SoundsScreen
import dev.reins.android.ui.settings.VaultPasskeysScreen
import dev.reins.android.ui.settings.VaultPasskeysViewModel
import dev.reins.android.ui.sheet.SheetHost
import dev.reins.android.ui.services.ServiceScreen
import dev.reins.android.ui.services.ServiceViewModel
import dev.reins.android.ui.signin.OnboardingScreen
import dev.reins.android.ui.signin.PasskeyOfferScreen
import dev.reins.android.ui.signin.SetupScreen
import dev.reins.android.ui.signin.SignInViewModel
import dev.reins.android.ui.signin.UnlockScreen
import dev.reins.android.ui.signin.UnlockViewModel
import dev.reins.android.ui.update.UpdatePromptHost
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch

@Composable
fun ReinsRoot(container: AppContainer, app: AppViewModel, authenticator: Authenticator, passkeys: PasskeyPrompt) {
    ReinsTheme {
        ProvideFeedback(container.feedback) { RootContent(container, app, authenticator, passkeys) }
    }
}

@Composable
private fun RootContent(container: AppContainer, app: AppViewModel, authenticator: Authenticator, passkeys: PasskeyPrompt) {
    val epoch by container.state.accountEpoch.collectAsStateWithLifecycle()
    val owner = remember(epoch) { object : ViewModelStoreOwner { override val viewModelStore = ViewModelStore() } }
    DisposableEffect(owner) { onDispose { owner.viewModelStore.clear() } }
    CompositionLocalProvider(LocalViewModelStoreOwner provides owner) {
        AccountContent(container, app, authenticator, passkeys)
    }
}

@Composable
private fun AccountContent(container: AppContainer, app: AppViewModel, authenticator: Authenticator, passkeys: PasskeyPrompt) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val session by container.state.session.collectAsStateWithLifecycle()
    when (session) {
        SessionState.Loading -> Box(Modifier.fillMaxSize().background(LocalColors.current.background), contentAlignment = Alignment.Center) {
            Spinner(LocalColors.current.accent, size = 48.dp)
        }
        SessionState.SignedOut -> Box(Modifier.fillMaxSize()) {
            val waitingCode by app.waitingCode.collectAsStateWithLifecycle()
            OnboardingScreen(viewModel(key = "signin") { SignInViewModel(container) }, linkWaiting = waitingCode != null)
            container.updates?.let {
                UpdatePromptHost(
                    it,
                    allowed = true,
                    modifier = Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = 12.dp),
                )
            }
        }
        is SessionState.SignedIn -> {
            val info = (session as SessionState.SignedIn).info
            val locked by container.state.keysLocked.collectAsStateWithLifecycle()
            val takeover by container.state.approvalTakeover.collectAsStateWithLifecycle()
            val recovery by container.state.recoveryToRecord.collectAsStateWithLifecycle()
            val passkeyOffer by container.state.vaultPasskeyOffer.collectAsStateWithLifecycle()
            val recoveryError by container.state.recoveryLoadError.collectAsStateWithLifecycle()
            val recoveryScope = rememberCoroutineScope()
            if (locked || takeover) {
                UnlockScreen(
                    viewModel(key = "unlock") { UnlockViewModel(container, deviceName = Build.MODEL) },
                    info.email,
                    passkeys,
                    takeover = takeover && !locked,
                )
            } else if (recoveryError != null) {
                dev.reins.android.design.Screen(title = "Recovery setup") {
                    androidx.compose.foundation.layout.Column(Modifier.padding(24.dp)) {
                        dev.reins.android.design.RText(recoveryError!!, dev.reins.android.design.RType.sans(16f), LocalColors.current.secondary)
                        dev.reins.android.design.CapsuleButton("Try again", onClick = { recoveryScope.launch { container.refreshSession() } })
                    }
                }
            } else if (recovery != null && passkeyOffer) {
                BackHandler { /* The offer is answered with its own buttons, then the recovery code follows. */ }
                PasskeyOfferScreen(viewModel(key = "passkeyOffer") { VaultPasskeysViewModel(container, deviceName = Build.MODEL) }, passkeys)
            } else if (recovery != null) {
                BackHandler { /* Recording recovery is required; Back cannot skip it. */ }
                dev.reins.android.ui.settings.RecoveryCodeSheet(
                    code = recovery!!,
                    required = true,
                    onCopy = { dev.reins.android.ui.settings.copySecret(context, "Reins recovery code", recovery!!) },
                    onDone = { recoveryScope.launch { container.confirmRecoveryRecord() } },
                )
            } else {
                SignedInContent(container, app, authenticator, passkeys, info.serverUrl)
            }
        }
    }
}

@Composable
private fun SignedInContent(
    container: AppContainer,
    app: AppViewModel,
    authenticator: Authenticator,
    passkeys: PasskeyPrompt,
    serverUrl: String,
) {
    val state = container.state
    val setup by state.setupPending.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val sheet by app.sheet.collectAsStateWithLifecycle()
    val notice by app.notice.collectAsStateWithLifecycle()
    val gmail = viewModel(key = "gmail") { GmailViewModel(container) }
    val settings = viewModel(key = "settings") { SettingsViewModel(container) }
    val mcp = viewModel(key = "mcp") { McpViewModel(container) }
    val autopilot = viewModel(key = "autopilot") { AutopilotViewModel(container) }
    val scope = rememberCoroutineScope()
    // The setup can open an integration's page over itself; Back returns to the same setup page.
    val route = app.current
    BackHandler(enabled = app.stack.isNotEmpty()) { app.back() }
    PageFeedback(app.stack.size)

    CompositionLocalProvider(LocalConnections provides connections) {
        Box(Modifier.fillMaxSize().background(LocalColors.current.background)) {
            if (setup && route == null) SetupScreen(app, container, autopilot, serverUrl) else when (route) {
                null -> when (app.tab) {
                    Tab.Activity -> ActivityScreen(
                        container = container,
                        notice = notice,
                        onDismissNotice = app::dismissNotice,
                        onOpenPending = { app.openSheet(it.toTarget()) },
                        onOpenEntry = { app.open(Route.ActivityDetail(it)) },
                        onIntegrations = { app.open(Route.Integrations) },
                        onSettings = { app.open(Route.Settings) },
                        onAnswerBurst = { burst, approve ->
                            scope.launch {
                                dev.reins.android.ui.activity.answerBurst(container, authenticator, burst, approve)?.let(app::showNotice)
                            }
                        },
                    )
                    Tab.Grants -> GrantsScreen(
                        state = state,
                        onSettings = { app.open(Route.Settings) },
                        expiredOpen = app.expiredOpen,
                        onToggleExpired = app::toggleExpired,
                        onOpen = { app.open(Route.GrantDetail(it)) },
                        onNew = { app.open(Route.NewGrant) },
                        onResume = { id, seconds, standing -> resumeGrant(container, authenticator, id, seconds, standing) },
                        onDelete = { id -> deleteGrant(container, id) },
                    )
                }
                Route.Settings -> SettingsScreen(
                    state = state,
                    viewModel = settings,
                    onBack = { app.back() },
                    onConnection = { app.open(Route.Connection(it)) },
                    onIntegrations = { app.open(Route.Integrations) },
                    onSounds = { app.open(Route.Sounds) },
                    onTour = app::replaySetup,
                    onAutopilot = { app.open(Route.Autopilot) },
                    onConnectComputer = { app.open(Route.ConnectComputer) },
                    onVaultPasskeys = { app.open(Route.VaultPasskeys) },
                    authenticator = authenticator,
                )
                Route.VaultPasskeys -> VaultPasskeysScreen(
                    viewModel(key = "vaultPasskeys") { VaultPasskeysViewModel(container, deviceName = Build.MODEL) },
                    passkeys,
                    onBack = { app.back() },
                )
                Route.ConnectComputer -> ConnectComputerScreen(app, onBack = { app.back() })
                Route.Sounds -> SoundsScreen(container.feedback, onBack = { app.back() })
                Route.Autopilot -> AutopilotScreen(
                    autopilot,
                    onBack = { app.back() },
                    onProfile = { app.open(Route.AutopilotProfile(it)) },
                    onTryIt = { app.open(Route.TryIt(null)) },
                )
                is Route.AutopilotProfile -> ProfileScreen(
                    route.id,
                    autopilot,
                    state,
                    onBack = { app.back() },
                    onTryIt = { app.open(Route.TryIt(it)) },
                )
                is Route.TryIt -> TryItScreen(autopilot, route.profileId, onBack = { app.back() })
                is Route.Connection -> ConnectionDetailScreen(route.id, state, settings, autopilot, onBack = { app.back() })
                Route.Integrations -> IntegrationsScreen(
                    state,
                    onBack = { app.back() },
                    onOpen = { id -> app.open(if (id == "gmail") Route.Gmail else Route.Service(id)) },
                    onOpenMcp = { id -> app.open(Route.McpServer(id)) },
                    onAddMcp = { app.open(Route.McpAdd) },
                    onShown = mcp::refreshList,
                )
                Route.McpAdd -> McpAddScreen(
                    mcp,
                    onBack = { app.back() },
                    onAdded = { id ->
                        app.back()
                        app.open(Route.McpServer(id))
                    },
                )
                is Route.McpServer -> McpServerScreen(mcp, state, route.id, onBack = { app.back() }, onRemoved = { app.back() })
                is Route.Service -> ServiceScreen(
                    viewModel(key = "service:${route.id}") { ServiceViewModel(container, route.id) },
                    state,
                    onBack = { app.back() },
                )
                Route.Gmail -> GmailScreen(gmail, state, onBack = { app.back() })
                is Route.ActivityDetail -> ActivityDetailScreen(
                    entryId = route.id,
                    state = state,
                    onBack = { app.back() },
                    onOpenGrant = { app.open(Route.GrantDetail(it)) },
                    onOpenEmail = { entry, index -> app.open(Route.Email(entry, index)) },
                    onCorrect = { id, verdict ->
                        try {
                            container.feedback.play(Event.Undo)
                            container.core.correctDecision(id, verdict)
                            container.refreshPending()
                            null
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            container.feedback.play(Event.Error)
                            e.userMessage()
                        }
                    },
                )
                is Route.Email -> EmailScreen(
                    entryId = route.entryId,
                    index = route.index,
                    state = state,
                    load = { account, id -> container.core.fetchEmail(account, id) },
                    onBack = { app.back() },
                )
                is Route.GrantDetail -> GrantDetailScreen(
                    grantId = route.id,
                    state = state,
                    onBack = { app.back() },
                    onOpenEntry = { app.open(Route.ActivityDetail(it)) },
                    onResume = { id, seconds, standing -> resumeGrant(container, authenticator, id, seconds, standing) },
                    onDelete = { id -> deleteGrant(container, id) },
                    onRevoke = { id ->
                        try {
                            container.feedback.play(Event.Revoked)
                            container.core.revokeGrant(id)
                            container.refreshPending()
                            null
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            container.feedback.play(Event.Error)
                            e.userMessage()
                        }
                    },
                )
                Route.NewGrant -> NewGrantScreen(
                    viewModel = viewModel(key = "newgrant:${app.newGrantToken}") { NewGrantViewModel(container) },
                    state = state,
                    authenticator = authenticator,
                    onBack = { app.back() },
                    onDone = { app.back() },
                )
            }
            if (!setup && route == null) {
                Column(Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars)) {
                    // Above the tab bar, and never while an approval sheet is up.
                    container.updates?.let { UpdatePromptHost(it, allowed = sheet == null, modifier = Modifier.padding(top = 10.dp)) }
                    NavBar(container, app)
                }
            }
            SheetHost(sheet, onClose = app::closeSheet) { target ->
                when (target) {
                    is SheetTarget.Approval -> ApprovalSheet(
                        viewModel = viewModel(key = "approval:${target.id}") { ApprovalViewModel(container, target.id) },
                        authenticator = authenticator,
                        onDone = app::closeSheet,
                    )
                    is SheetTarget.Pairing -> PairingSheet(
                        viewModel = viewModel(key = "pairing:${target.id}") { PairingViewModel(container, target.id) },
                        authenticator = authenticator,
                        onDone = app::closeSheet,
                    )
                    is SheetTarget.Upload -> UploadSheet(
                        viewModel = viewModel(key = "upload:${target.id}") { UploadViewModel(container, target.id) },
                        authenticator = authenticator,
                        onDone = app::closeSheet,
                    )
                    is SheetTarget.Join -> JoinSheet(
                        viewModel = viewModel(key = "join:${target.id}") { JoinViewModel(container, target.id) },
                        authenticator = authenticator,
                        onDone = { notice ->
                            app.closeSheet()
                            notice?.let(app::showNotice)
                        },
                    )
                }
            }
        }
    }
}

/**
 * The tab bar. It reads the lists for its badges itself, so a refresh that changes them redraws the bar, not the whole
 * screen.
 */
@Composable
private fun NavBar(container: AppContainer, app: AppViewModel) {
    val entries by container.state.activity.collectAsStateWithLifecycle()
    val grants by container.state.grants.collectAsStateWithLifecycle()
    val seen by container.state.seenActivityId.collectAsStateWithLifecycle()
    val autopilot by container.state.autopilot.collectAsStateWithLifecycle()
    FloatingNavBar(
        selected = app.tab,
        activityCount = entries.count { it.id > seen },
        grantCount = grants.count { it.active },
        autopilot = autopilot,
        onSelect = app::selectTab,
        onAutopilot = { app.open(Route.Autopilot) },
    )
}

/** A page pushed opens with [Cue.Open], going back closes with [Cue.Close] (quiet when the action that left just sounded). */
@Composable
private fun PageFeedback(depth: Int) {
    val feedback = LocalFeedback.current
    val last = remember { mutableIntStateOf(depth) }
    LaunchedEffect(depth) {
        when {
            depth > last.intValue -> feedback.cue(Cue.Open)
            depth < last.intValue -> feedback.cueUnlessRecent(Cue.Close)
        }
        last.intValue = depth
    }
}
