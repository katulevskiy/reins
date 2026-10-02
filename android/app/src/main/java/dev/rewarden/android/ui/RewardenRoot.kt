package dev.rewarden.android.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
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
import dev.rewarden.android.AppContainer
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RewardenTheme
import dev.rewarden.android.design.Spinner
import dev.rewarden.android.feedback.Cue
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.LocalFeedback
import dev.rewarden.android.feedback.ProvideFeedback
import dev.rewarden.android.feedback.cueUnlessRecent
import dev.rewarden.android.feedback.play
import dev.rewarden.android.platform.Authenticator
import dev.rewarden.android.state.SessionState
import dev.rewarden.android.ui.activity.ActivityDetailScreen
import dev.rewarden.android.ui.activity.ActivityScreen
import dev.rewarden.android.ui.activity.EmailScreen
import dev.rewarden.android.ui.approval.ApprovalSheet
import dev.rewarden.android.ui.approval.ApprovalViewModel
import dev.rewarden.android.ui.autopilot.AutopilotScreen
import dev.rewarden.android.ui.autopilot.AutopilotViewModel
import dev.rewarden.android.ui.autopilot.ProfileScreen
import dev.rewarden.android.ui.autopilot.TryItScreen
import dev.rewarden.android.ui.common.LocalConnections
import dev.rewarden.android.ui.common.userMessage
import dev.rewarden.android.ui.gmail.GmailScreen
import dev.rewarden.android.ui.gmail.GmailViewModel
import dev.rewarden.android.ui.gmail.IntegrationsScreen
import dev.rewarden.android.ui.grants.GrantDetailScreen
import dev.rewarden.android.ui.grants.GrantsScreen
import dev.rewarden.android.ui.grants.deleteGrant
import dev.rewarden.android.ui.grants.resumeGrant
import dev.rewarden.android.ui.grants.NewGrantScreen
import dev.rewarden.android.ui.grants.NewGrantViewModel
import dev.rewarden.android.ui.main.FloatingNavBar
import dev.rewarden.android.ui.mcp.McpAddScreen
import dev.rewarden.android.ui.mcp.McpServerScreen
import dev.rewarden.android.ui.mcp.McpViewModel
import dev.rewarden.android.ui.upload.UploadSheet
import dev.rewarden.android.ui.upload.UploadViewModel
import dev.rewarden.android.ui.nav.Route
import dev.rewarden.android.ui.nav.SheetTarget
import dev.rewarden.android.ui.nav.Tab
import dev.rewarden.android.ui.pairing.PairingSheet
import dev.rewarden.android.ui.pairing.PairingViewModel
import dev.rewarden.android.ui.settings.ConnectionDetailScreen
import dev.rewarden.android.ui.settings.SettingsScreen
import dev.rewarden.android.ui.settings.SettingsViewModel
import dev.rewarden.android.ui.settings.SoundsScreen
import dev.rewarden.android.ui.sheet.SheetHost
import dev.rewarden.android.ui.services.ServiceScreen
import dev.rewarden.android.ui.services.ServiceViewModel
import dev.rewarden.android.ui.signin.SignInScreen
import dev.rewarden.android.ui.signin.SignInViewModel
import dev.rewarden.android.ui.update.UpdatePromptHost
import kotlin.coroutines.cancellation.CancellationException

@Composable
fun RewardenRoot(container: AppContainer, app: AppViewModel, authenticator: Authenticator) {
    RewardenTheme {
        ProvideFeedback(container.feedback) { RootContent(container, app, authenticator) }
    }
}

@Composable
private fun RootContent(container: AppContainer, app: AppViewModel, authenticator: Authenticator) {
    val session by container.state.session.collectAsStateWithLifecycle()
    when (session) {
        SessionState.Loading -> Box(Modifier.fillMaxSize().background(LocalColors.current.background), contentAlignment = Alignment.Center) {
            Spinner(LocalColors.current.accent, size = 48.dp)
        }
        SessionState.SignedOut -> Box(Modifier.fillMaxSize()) {
            SignInScreen(viewModel(key = "signin") { SignInViewModel(container) })
            container.updates?.let {
                UpdatePromptHost(
                    it,
                    allowed = true,
                    modifier = Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars).padding(bottom = 12.dp),
                )
            }
        }
        is SessionState.SignedIn -> SignedInContent(container, app, authenticator)
    }
}

@Composable
private fun SignedInContent(container: AppContainer, app: AppViewModel, authenticator: Authenticator) {
    val state = container.state
    val connections by state.connections.collectAsStateWithLifecycle()
    val entries by state.activity.collectAsStateWithLifecycle()
    val grants by state.grants.collectAsStateWithLifecycle()
    val seen by state.seenActivityId.collectAsStateWithLifecycle()
    val sheet by app.sheet.collectAsStateWithLifecycle()
    val notice by app.notice.collectAsStateWithLifecycle()
    val gmail = viewModel(key = "gmail") { GmailViewModel(container) }
    val settings = viewModel(key = "settings") { SettingsViewModel(container) }
    val mcp = viewModel(key = "mcp") { McpViewModel(container) }
    val autopilot = viewModel(key = "autopilot") { AutopilotViewModel(container) }
    val scope = rememberCoroutineScope()
    val route = app.current
    BackHandler(enabled = app.stack.isNotEmpty()) { app.back() }
    PageFeedback(app.stack.size)

    CompositionLocalProvider(LocalConnections provides connections) {
        Box(Modifier.fillMaxSize().background(LocalColors.current.background)) {
            when (route) {
                null -> when (app.tab) {
                    Tab.Activity -> ActivityScreen(
                        container = container,
                        notice = notice,
                        onDismissNotice = app::dismissNotice,
                        onOpenPending = { app.openSheet(it.toTarget()) },
                        onOpenEntry = { app.open(Route.ActivityDetail(it)) },
                        onIntegrations = { app.open(Route.Integrations) },
                        onAutopilot = { app.open(Route.Autopilot) },
                    )
                    Tab.Grants -> GrantsScreen(
                        state = state,
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
                    onAutopilot = { app.open(Route.Autopilot) },
                )
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
            if (route == null) {
                Column(Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.navigationBars)) {
                    // Above the tab bar, and never while an approval sheet is up.
                    container.updates?.let { UpdatePromptHost(it, allowed = sheet == null, modifier = Modifier.padding(top = 10.dp)) }
                    FloatingNavBar(
                        selected = app.tab,
                        activityCount = entries.count { it.id > seen },
                        grantCount = grants.count { it.active },
                        onSelect = app::selectTab,
                        onSettings = { app.open(Route.Settings) },
                    )
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
                }
            }
        }
    }
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
