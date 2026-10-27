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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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
import dev.reins.android.ui.approval.ApprovalScreen
import dev.reins.android.ui.approval.ApprovalViewModel
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.ProfileScreen
import dev.reins.android.ui.autopilot.TryItScreen
import dev.reins.android.ui.common.LocalConnections
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.gmail.GmailScreen
import dev.reins.android.ui.gmail.GmailViewModel
import dev.reins.android.ui.grants.GrantDetailScreen
import dev.reins.android.ui.grants.deleteGrant
import dev.reins.android.ui.grants.resumeGrant
import dev.reins.android.ui.grants.NewGrantScreen
import dev.reins.android.ui.grants.NewGrantViewModel
import dev.reins.android.ui.join.JoinSheet
import dev.reins.android.ui.join.JoinViewModel
import dev.reins.android.ui.mcp.McpAddScreen
import dev.reins.android.ui.mcp.McpServerScreen
import dev.reins.android.ui.mcp.McpViewModel
import dev.reins.android.ui.upload.UploadSheet
import dev.reins.android.ui.upload.UploadViewModel
import dev.reins.android.ui.permissions.*
import dev.reins.android.ui.nav.Route
import dev.reins.android.ui.nav.SheetTarget
import dev.reins.android.ui.nav.Tab
import dev.reins.android.ui.pairing.ConnectComputerScreen
import dev.reins.android.ui.pairing.PairingSheet
import dev.reins.android.ui.pairing.PairingViewModel
import dev.reins.android.ui.settings.DevicesScreen
import dev.reins.android.ui.settings.DevicesViewModel
import dev.reins.android.ui.settings.SettingsScreen
import dev.reins.android.ui.settings.SettingsViewModel
import dev.reins.android.ui.signin.AccountRules
import dev.reins.android.ui.settings.SoundsScreen
import dev.reins.android.ui.settings.VaultPasskeysScreen
import dev.reins.android.ui.settings.VaultPasskeysViewModel
import dev.reins.android.ui.sheet.FullScreenHost
import dev.reins.android.ui.sheet.SheetHost
import dev.reins.android.ui.payments.PaymentsScreen
import dev.reins.android.ui.payments.PaymentsViewModel
import dev.reins.android.ui.payments.SpendingScreen
import dev.reins.android.ui.services.ServiceScreen
import dev.reins.android.ui.services.ServiceViewModel
import dev.reins.android.ui.signin.OnboardingScreen
import dev.reins.android.ui.signin.PasskeyOfferScreen
import dev.reins.android.ui.signin.SetupScreen
import dev.reins.android.ui.signin.SignInViewModel
import dev.reins.android.ui.signin.UnlockScreen
import dev.reins.android.ui.signin.UnlockViewModel
import dev.reins.android.ui.update.UpdatePromptHost
import dev.reins.android.ui.vault.VaultAddScreen
import dev.reins.android.ui.vault.VaultEditScreen
import dev.reins.android.ui.vault.VaultItemScreen
import dev.reins.android.ui.vault.VaultScreen
import dev.reins.android.ui.vault.VaultViewModel
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.launch
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.ui.autopilot.BypassSheets
import dev.reins.android.ui.autopilot.OvernightSheets
import dev.reins.android.ui.connections.ConnectionDetailScreen
import dev.reins.android.ui.connections.ConnectionsScreen
import dev.reins.android.ui.grants.EndedGrantsScreen
import dev.reins.android.ui.main.HeaderModeChip
import dev.reins.android.ui.main.ReinsTabBar
import dev.reins.android.ui.main.bypassRunning
import dev.reins.android.ui.settings.AppearanceScreen
import dev.reins.android.ui.trust.TrustScreen
import dev.reins.core.AutopilotMode

@Composable
fun ReinsRoot(container: AppContainer, app: AppViewModel, authenticator: Authenticator, passkeys: PasskeyPrompt) {
    ReinsTheme {
        ProvideFeedback(container.feedback) {
            Box(Modifier.fillMaxSize()) {
                RootContent(container, app, authenticator, passkeys)
                // What did not work in the background, for a moment, above the tab bar.
                container.updates?.let { dev.reins.android.ui.update.InstallPermissionHost(it) }
                dev.reins.android.ui.common.FlashHost(
                    container.state,
                    Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = 84.dp),
                )
            }
        }
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
    // The main screen is up and usable: what startup is measured to (`Fully drawn` in the log, Macrobenchmark).
    val host = androidx.compose.ui.platform.LocalContext.current as? androidx.activity.ComponentActivity
    androidx.compose.runtime.LaunchedEffect(Unit) { host?.reportFullyDrawn() }
    val setup by state.setupPending.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val sheet by app.sheet.collectAsStateWithLifecycle()
    val notice by app.notice.collectAsStateWithLifecycle()
    val gmail = viewModel(key = "gmail") { GmailViewModel(container) }
    val settings = viewModel(key = "settings") { SettingsViewModel(container) }
    val mcp = viewModel(key = "mcp") { McpViewModel(container) }
    val autopilot = viewModel(key = "autopilot") { AutopilotViewModel(container) }
    val vault = viewModel(key = "vault") { VaultViewModel(container) }
    val vaultItem by vault.item.collectAsStateWithLifecycle()
    val context = androidx.compose.ui.platform.LocalContext.current
    // The setup can open an integration's page over itself; Back returns to the same setup page.
    val route = app.current
    // A sheet of a tab covers the page: the tab bar steps aside.
    var covered by remember { mutableStateOf(false) }
    // The mode menu's Overnight and Bypass open their hold sheets over everything.
    var timedSheet by remember { mutableStateOf<AutopilotMode?>(null) }
    // The mode chip's one sheet.
    var modeSheet by remember { mutableStateOf(false) }
    LaunchedEffect(route, sheet) {
        timedSheet = null
        modeSheet = false
    }
    BackHandler(enabled = app.stack.isNotEmpty()) { app.back() }
    // Back on another tab's root goes to Activity first, then leaves the app.
    BackHandler(enabled = app.stack.isEmpty() && app.tab != Tab.Activity && !setup) { app.selectTab(Tab.Activity) }
    PageFeedback(app.stack.size)

    CompositionLocalProvider(LocalConnections provides connections) {
        Box(Modifier.fillMaxSize().background(LocalColors.current.paper)) {
            if (setup && route == null) {
                SetupScreen(app, container, autopilot, serverUrl)
            } else if (route == null) {
                val bar = !covered && timedSheet == null && !modeSheet
                Column(Modifier.fillMaxSize()) {
                    Box(
                        Modifier
                            .weight(1f)
                            .fillMaxWidth()
                            .then(if (bar) Modifier.consumeWindowInsets(WindowInsets.navigationBars) else Modifier),
                    ) {
                        TabContent(container, app, authenticator, serverUrl, autopilot, settings, mcp, notice, onCovered = { covered = it }, onModeSheet = { modeSheet = true })
                        // Above the tab bar, and never while an approval sheet is up.
                        container.updates?.let {
                            UpdatePromptHost(it, allowed = sheet == null && bar, modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 14.dp))
                        }
                    }
                    if (bar) NavBar(container, app)
                }
            } else {
                PageContent(route, container, app, authenticator, passkeys, autopilot, settings, mcp, gmail, vault, vaultItem)
            }
            val apSettings by state.autopilot.collectAsStateWithLifecycle()
            val apUi by autopilot.ui.collectAsStateWithLifecycle()
            // Picking Auto before its setup was finished opens the setup instead (from any mode control).
            LaunchedEffect(apUi.setupRequested) {
                if (apUi.setupRequested) {
                    modeSheet = false
                    app.open(Route.AutoSetup)
                }
            }
            dev.reins.android.ui.main.ModeSheet(
                open = modeSheet,
                settings = apSettings,
                modelReady = AutopilotText.modelUsable(apUi.model ?: apSettings?.model),
                // Leaving Lockdown from the sheet asks the owner first, as Trust's End does.
                onSet = { mode -> autopilot.setMode(mode, authenticator = authenticator) },
                onTimed = { timedSheet = it },
                onStop = { mode -> if (mode == AutopilotMode.BYPASS) autopilot.stopBypass() else autopilot.stopOvernight() },
                onSettings = { app.selectTab(Tab.Trust) },
                onClose = { modeSheet = false },
            )
            BypassSheets(
                timedSheet == AutopilotMode.BYPASS,
                who = null,
                onStart = { minutes -> autopilot.startBypass(minutes, authenticator = authenticator) },
                onClose = { timedSheet = null },
            )
            OvernightSheets(
                timedSheet == AutopilotMode.OVERNIGHT,
                who = null,
                risky = apSettings?.overnightWhenRisky ?: dev.reins.core.OvernightRisky.WAIT,
                ratesRisk = (apUi.model ?: apSettings?.model)?.risk == true,
                onRisky = { autopilot.setOvernightWhenRisky(it, authenticator) },
                onStart = { minutes -> autopilot.startOvernight(minutes, authenticator = authenticator) },
                onClose = { timedSheet = null },
            )
            // Every ask is a field over the whole app: a request, a pairing, a new phone, a file.
            FullScreenHost(sheet, onClose = app::closeSheet) { target ->
                when (target) {
                    is SheetTarget.Approval -> ApprovalScreen(
                        viewModel = viewModel(key = "approval:${target.id}") { ApprovalViewModel(container, target.id) },
                        authenticator = authenticator,
                        onDone = app::closeSheet,
                        onOpenAutopilot = {
                            app.closeSheet()
                            app.selectTab(Tab.Trust)
                        },
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
                    is SheetTarget.Join -> {
                        val onJoinDone: (String?) -> Unit = { notice ->
                            app.closeSheet()
                            notice?.let(app::showNotice)
                        }
                        // A second phone asking to view the account, or (without phone roles) one asking for its keys.
                        val kind = viewModel(key = "joinKind:${target.id}") { dev.reins.android.ui.enroll.JoinKindViewModel(container, target.id) }
                        val enrollment by kind.enrollment.collectAsStateWithLifecycle()
                        when (enrollment) {
                            null -> dev.reins.android.ui.approval.AskLoading(true, null, onClose = { onJoinDone(null) })
                            true -> dev.reins.android.ui.enroll.EnrollRequestSheet(
                                viewModel = viewModel(key = "enroll:${target.id}") { dev.reins.android.ui.enroll.EnrollRequestViewModel(container, target.id) },
                                authenticator = authenticator,
                                onDone = onJoinDone,
                            )
                            false -> JoinSheet(
                                viewModel = viewModel(key = "join:${target.id}") { JoinViewModel(container, target.id) },
                                authenticator = authenticator,
                                onDone = onJoinDone,
                            )
                        }
                    }
                }
            }
        }
    }
}

/** The root of the tab in front: Activity, Trust, Connections or Settings. */
@Composable
private fun TabContent(
    container: AppContainer,
    app: AppViewModel,
    authenticator: Authenticator,
    serverUrl: String,
    autopilot: AutopilotViewModel,
    settings: SettingsViewModel,
    mcp: McpViewModel,
    notice: String?,
    onCovered: (Boolean) -> Unit,
    onModeSheet: () -> Unit,
) {
    val state = container.state
    val context = androidx.compose.ui.platform.LocalContext.current
    val questionScope = rememberCoroutineScope()
    when (app.tab) {
        Tab.Activity -> ActivityScreen(
            container = container,
            notice = notice,
            onDismissNotice = app::dismissNotice,
            onOpenPending = { app.openSheet(it.toTarget()) },
            onOpenEntry = { app.open(Route.ActivityDetail(it)) },
            modeChip = {
                val apSettings by state.autopilot.collectAsStateWithLifecycle()
                HeaderModeChip(apSettings, onOpen = onModeSheet)
            },
            onAnswerBurst = { burst, approve ->
                dev.reins.android.ui.activity.answerBurst(container, authenticator, burst, approve)?.let(app::showNotice)
            },
            onAnswerQuestion = { q, zone ->
                questionScope.launch { dev.reins.android.ui.activity.answerQuestion(container, authenticator, q, zone)?.let(app::showNotice) }
            },
            onDismissQuestion = { q ->
                questionScope.launch { dev.reins.android.ui.activity.dismissQuestion(container, q)?.let(app::showNotice) }
            },
            onConnectComputer = { app.open(Route.ConnectComputer) },
            onOpenSettings = { app.selectTab(Tab.Settings) },
            onConnectAi = {
                val url = AccountRules.mcpUrl(serverUrl)
                context.getSystemService(android.content.ClipboardManager::class.java)
                    ?.setPrimaryClip(android.content.ClipData.newPlainText("Reins MCP address", url))
                app.showNotice("Copied $url. In Claude.ai or ChatGPT: Settings, Connectors, add a custom connector and paste it.")
            },
        )
        Tab.Trust -> TrustScreen(
            autopilot,
            state,
            authenticator,
            onProfile = { app.open(Route.AutopilotProfile(it)) },
            onTryIt = { app.open(Route.TryIt(null)) },
            onOpenGrant = { app.open(Route.GrantDetail(it)) },
            onNewGrant = { app.open(Route.NewGrant) },
            onEnded = { app.open(Route.EndedGrants) },
            onPermissions = { app.open(Route.Permissions) },
            onStartingPolicy = container::chooseStartingPolicy,
            onOpenEntry = { app.open(Route.ActivityDetail(it)) },
            onCovered = onCovered,
        )
        Tab.Connections -> ConnectionsScreen(
            state,
            serverUrl = serverUrl,
            onConnection = { app.open(Route.Connection(it)) },
            onService = { id ->
                app.open(
                    when (id) {
                        "gmail" -> Route.Gmail
                        "payments" -> Route.Payments
                        else -> Route.Service(id)
                    },
                )
            },
            onMcp = { id -> app.open(Route.McpServer(id)) },
            onAddMcp = { app.open(Route.McpAdd) },
            onScanComputer = {
                app.requestScan()
                app.open(Route.ConnectComputer)
            },
            onTypeComputerCode = {
                app.showCodeField()
                app.open(Route.ConnectComputer)
            },
            onShown = mcp::refreshList,
            onCovered = onCovered,
            viewerJoin = viewModel(key = "viewerJoin") { dev.reins.android.ui.enroll.ViewerJoinViewModel(container, deviceName = android.os.Build.MODEL) },
        )
        Tab.Settings -> SettingsScreen(
            state = state,
            viewModel = settings,
            onSounds = { app.open(Route.Sounds) },
            onAppearance = { app.open(Route.Appearance) },
            onWallpaper = { app.open(Route.Wallpaper) },
            onTour = app::replaySetup,
            onVaultPasskeys = { app.open(Route.VaultPasskeys) },
            onDevices = { app.open(Route.Devices) },
            onAddWorkAccount = { app.open(Route.AddWorkAccount) },
            onSignalGallery = { app.open(Route.SignalGallery) },
            authenticator = authenticator,
        )
    }
}

/** A page pushed over the tabs. */
@Composable
private fun PageContent(
    route: Route,
    container: AppContainer,
    app: AppViewModel,
    authenticator: Authenticator,
    passkeys: PasskeyPrompt,
    autopilot: AutopilotViewModel,
    settings: SettingsViewModel,
    mcp: McpViewModel,
    gmail: GmailViewModel,
    vault: VaultViewModel,
    vaultItem: dev.reins.core.VaultItemDetail?,
) {
    val state = container.state
    val accountEpoch by state.accountEpoch.collectAsStateWithLifecycle()
    when (route) {
        Route.AddWorkAccount -> dev.reins.android.ui.settings.AddWorkAccountScreen(
            viewModel(key = "addWorkAccount") { dev.reins.android.ui.settings.AddWorkAccountViewModel(container) },
            onBack = { app.back() },
        )
        Route.Devices -> DevicesScreen(
            viewModel(key = "devices") { DevicesViewModel(container) },
            state,
            onBack = { app.back() },
            onConnection = { app.open(Route.Connection(it)) },
        )
        Route.VaultPasskeys -> VaultPasskeysScreen(
            viewModel(key = "vaultPasskeys") { VaultPasskeysViewModel(container, deviceName = Build.MODEL) },
            passkeys,
            onBack = { app.back() },
        )
        Route.ConnectComputer -> ConnectComputerScreen(app, onBack = { app.back() })
        Route.Sounds -> SoundsScreen(container.feedback, onBack = { app.back() })
        Route.Appearance -> AppearanceScreen(onBack = { app.back() })
        Route.Wallpaper -> dev.reins.android.ui.settings.WallpaperScreen(onBack = { app.back() })
        Route.SignalGallery -> if (dev.reins.android.BuildConfig.DEBUG) dev.reins.android.ui.signal.SignalGallery(onBack = { app.back() })
        Route.EndedGrants -> EndedGrantsScreen(
            state,
            onBack = { app.back() },
            onOpen = { app.open(Route.GrantDetail(it)) },
            onResume = { id, seconds, standing -> resumeGrant(container, authenticator, id, seconds, standing) },
            onDelete = { id -> deleteGrant(container, id) },
        )
        is Route.AutopilotProfile -> ProfileScreen(
            route.id,
            autopilot,
            state,
            onBack = { app.back() },
            onTryIt = { app.open(Route.TryIt(it)) },
        )
        is Route.TryIt -> TryItScreen(autopilot, route.profileId, onBack = { app.back() })
        Route.AutoSetup -> dev.reins.android.ui.autopilot.AutoSetupScreen(
            autopilot,
            authenticator,
            onBack = { app.back() },
            // Auto is on: back to Autopilot, on Trust.
            onDone = { app.selectTab(Tab.Trust) },
        )
        is Route.Connection -> ConnectionDetailScreen(route.id, state, settings, autopilot, onBack = { app.back() }, authenticator = authenticator)
        Route.McpAdd -> McpAddScreen(
            mcp,
            onBack = { app.back() },
            onAdded = { id ->
                if (app.current == Route.McpAdd) {
                    app.back()
                    app.open(Route.McpServer(id))
                }
            },
        )
        is Route.McpServer -> McpServerScreen(mcp, state, route.id, onBack = { app.back() }, onRemoved = { app.back() })
        is Route.Service -> ServiceScreen(
            viewModel(key = "service:${route.id}") { ServiceViewModel(container, route.id) },
            state,
            onBack = { app.back() },
            onOpenVault = { app.open(Route.Vault) },
            onPermissions = { app.open(Route.AccountPermissions(route.id, it)) },
            permissionFinish = { identity, done, another ->
                AccountPermissionsScreen(viewModel(key = "permissions:$accountEpoch:${route.id}:$identity") { AccountPermissionsViewModel(container, route.id, identity) }, authenticator, true, { app.back() }, done, {}, another)
            },
        )
        Route.Vault -> VaultScreen(
            vault,
            onBack = { app.back() },
            onOpen = { app.open(Route.VaultItem(it)) },
            onAdd = { app.open(Route.VaultAdd) },
        )
        Route.VaultAdd -> VaultAddScreen(onBack = { app.back() }, onPick = { app.open(Route.VaultEdit(null, it)) })
        is Route.VaultItem -> VaultItemScreen(
            vault,
            route.id,
            authenticator,
            onBack = { app.back() },
            onEdit = { app.open(Route.VaultEdit(route.id, null)) },
        )
        is Route.VaultEdit -> VaultEditScreen(
            vault,
            newItem = route.newItem,
            existing = vaultItem?.takeIf { it.id == route.id },
            authenticator = authenticator,
            onBack = { app.back() },
            onSaved = { id ->
                // An edited item goes back to its page; a new one opens in place of "Add" and the form.
                app.back()
                if (route.id == null) {
                    app.back()
                    app.open(Route.VaultItem(id))
                }
            },
        )
        Route.Gmail -> GmailScreen(gmail, state, onBack = { app.back() },
            onPermissions = { app.open(Route.AccountPermissions("gmail", it)) },
            permissionFinish = { identity, done, another ->
                AccountPermissionsScreen(viewModel(key = "permissions:$accountEpoch:gmail:$identity") { AccountPermissionsViewModel(container, "gmail", identity) }, authenticator, true, { app.back() }, done, {}, another)
            })
        Route.Permissions -> PermissionsHomeScreen(container, authenticator, { app.back() }, { service, account -> app.open(Route.AccountPermissions(service, account)) })
        is Route.AccountPermissions -> AccountPermissionsScreen(
            viewModel(key = "permissions:$accountEpoch:${route.service}:${route.account}:${route.temporary}") { AccountPermissionsViewModel(container, route.service, route.account, route.temporary) },
            authenticator, false, { app.back() }, { app.back() }, { app.open(Route.AccountPermissions(route.service, route.account, true)) },
        )
        Route.Payments -> PaymentsScreen(
            viewModel(key = "payments") { PaymentsViewModel(container) },
            state,
            onBack = { app.back() },
            onSpending = { app.open(Route.Spending) },
        )
        Route.Spending -> SpendingScreen(viewModel(key = "payments") { PaymentsViewModel(container) }, onBack = { app.back() })
        is Route.ActivityDetail -> ActivityDetailScreen(
            entryId = route.id,
            state = state,
            container = container,
            authenticator = authenticator,
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
            core = container.core,
        )
        is Route.GrantDetail -> GrantDetailScreen(
            grantId = route.id,
            state = state,
            onBack = { app.back() },
            onOpenEntry = { app.open(Route.ActivityDetail(it)) },
            onResume = { id, seconds, standing -> resumeGrant(container, authenticator, id, seconds, standing) },
            onDelete = { id -> deleteGrant(container, id) },
            onExtend = { id, seconds -> dev.reins.android.ui.grants.extendGrant(container, authenticator, id, seconds) },
            onRevokeSecrets = { project, env -> dev.reins.android.ui.grants.revokeSecretsEverywhere(container, project, env) },
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
}

/**
 * The tab bar. It reads the lists for its badge and dot itself, so a refresh that changes them redraws the bar, not
 * the whole screen.
 */
@Composable
private fun NavBar(container: AppContainer, app: AppViewModel) {
    val entries by container.state.activity.collectAsStateWithLifecycle()
    val seen by container.state.seenActivityId.collectAsStateWithLifecycle()
    val autopilot by container.state.autopilot.collectAsStateWithLifecycle()
    ReinsTabBar(
        selected = app.tab,
        activityCount = entries.count { it.id > seen },
        bypassOn = bypassRunning(autopilot),
        onSelect = app::selectTab,
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
