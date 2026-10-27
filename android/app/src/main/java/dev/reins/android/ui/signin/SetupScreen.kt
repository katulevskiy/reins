package dev.reins.android.ui.signin

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.R
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.autopilot.DownloadJob
import dev.reins.android.design.AskText
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Calm
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.InlineHelp
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Spinner
import dev.reins.android.design.Toggle
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Cue
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.cueUnlessRecent
import dev.reins.android.feedback.play
import dev.reins.android.platform.NotificationAccess
import dev.reins.android.platform.NotificationState
import dev.reins.android.platform.ScreenLockState
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.platform.rememberScreenLockState
import dev.reins.android.ui.AppViewModel
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.PlainTile
import dev.reins.android.ui.common.ScreenLockCard
import dev.reins.android.ui.nav.Route
import dev.reins.android.ui.pairing.ComputerHowTo
import dev.reins.android.ui.pairing.ConnectComputerPanel
import dev.reins.android.ui.pairing.DesktopAppLink
import dev.reins.core.AutopilotMode
import dev.reins.core.ModelState
import dev.reins.core.ServiceView
/** The pages of the setup after signing in, in order. */

/** The tour's five pages (iOS-Tour: eight steps became five, always skippable). */
enum class SetupPage { Welcome, Notifications, Connect, Ask, Done }

/**
 * Right after signing in or creating an account (once per account, and again from Settings > Take the tour), in five
 * pages: what Reins promises; requests come to you (notifications and the screen lock); what your AIs can use (the
 * services, the computer and an AI app, all connections); how much to ask (how a new AI starts and Autopilot's
 * model); and a quiet finish with the pond. Skip ends it from any page. A service opens its own page over the tour and
 * Back returns to the same page ([AppViewModel.setupPage]).
 */
@Composable
fun SetupScreen(app: AppViewModel, container: AppContainer, autopilot: AutopilotViewModel, serverUrl: String) {
    val feedback = LocalFeedback.current
    val notifications = rememberNotificationState()
    val screenLock = rememberScreenLockState()
    val page = app.setupPage
    val pages = SetupPage.entries
    val go = { next: SetupPage ->
        if (next.ordinal > page.ordinal) feedback.cue(Cue.Open) else feedback.cueUnlessRecent(Cue.Close)
        if (next == SetupPage.Connect) app.resetConnect()
        app.setupPage = next
    }
    val next = { go(pages[(page.ordinal + 1).coerceAtMost(pages.lastIndex)]) }
    val back = { go(pages[(page.ordinal - 1).coerceAtLeast(0)]) }
    BackHandler(enabled = page != SetupPage.Welcome, onBack = back)

    val c = LocalColors.current
    Column(
        Modifier
            .fillMaxSize()
            .background(c.paper)
            .windowInsetsPadding(WindowInsets.statusBars)
            .windowInsetsPadding(WindowInsets.navigationBars)
            .imePadding()
            .testTag("setup"),
    ) {
        SetupTopBar(page, onSkip = app::finishSetup)
        AnimatedContent(
            targetState = page,
            modifier = Modifier.weight(1f).fillMaxWidth(),
            transitionSpec = {
                val forward = targetState.ordinal > initialState.ordinal
                (slideInHorizontally(tween(320)) { w -> if (forward) w / 4 else -w / 4 } + fadeIn(tween(260)))
                    .togetherWith(slideOutHorizontally(tween(320)) { w -> if (forward) -w / 4 else w / 4 } + fadeOut(tween(180)))
            },
            label = "setupPage",
        ) { shown ->
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 24.dp)) {
                when (shown) {
                    SetupPage.Welcome -> WelcomePage()
                    SetupPage.Notifications -> NotificationsPage(notifications, screenLock)
                    SetupPage.Connect -> ConnectPage(app, container, AccountRules.mcpUrl(serverUrl)) { id ->
                        app.open(
                            when (id) {
                                "gmail" -> Route.Gmail
                                MCP -> Route.McpAdd
                                else -> Route.Service(id)
                            },
                        )
                    }
                    SetupPage.Ask -> AskPage(container, autopilot)
                    SetupPage.Done -> DonePage(container, autopilot, notifications, screenLock)
                }
                Spacer(Modifier.height(16.dp))
            }
        }
        SetupButtons(page, notifications, onBack = back, onNext = next, onDone = app::finishSetup)
    }
}

private const val MCP = "mcp"

/** A bar per page (filled up to this one), and Skip. */
@Composable
private fun SetupTopBar(page: SetupPage, onSkip: () -> Unit) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.width(120.dp).testTag("setupProgress"), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            SetupPage.entries.forEach { p ->
                val fill by animateColorAsState(if (p.ordinal <= page.ordinal) c.ink else c.rule, label = "segment")
                Box(Modifier.weight(1f).height(3.dp).clip(CircleShape).background(fill))
            }
        }
        Spacer(Modifier.weight(1f))
        if (page != SetupPage.Done) {
            SignalButton("Skip", Modifier.testTag("setupSkip"), style = ButtonStyle.Ghost, size = ButtonSize.Small, onClick = onSkip)
        } else {
            Spacer(Modifier.height(40.dp))
        }
    }
}

/** Back and the page's forward button; on the notifications page, Not now and Allow until they are on. */
@Composable
private fun SetupButtons(page: SetupPage, notifications: NotificationState, onBack: () -> Unit, onNext: () -> Unit, onDone: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        val asking = page == SetupPage.Notifications && !notifications.enabled
        if (asking) {
            val context = LocalContext.current
            SignalButton("Not now", Modifier.weight(1f).testTag("notificationsLater"), style = ButtonStyle.Outline) {
                // An answer: Activity does not ask again right after the tour (its card still offers them).
                NotificationAccess.markAsked(context)
                onNext()
            }
        } else if (page != SetupPage.Welcome && page != SetupPage.Done) {
            SignalButton("Back", Modifier.weight(1f).testTag("setupBack"), style = ButtonStyle.Outline, onClick = onBack)
        }
        when {
            page == SetupPage.Welcome -> SignalButton("Show me around", Modifier.weight(1f).testTag("setupNext"), onClick = onNext)
            asking -> SignalButton("Allow", Modifier.weight(1f).testTag("allowNotifications"), glyph = Glyph.Bell) { notifications.request() }
            page == SetupPage.Done -> SignalButton("Start using Reins", Modifier.weight(1f).testTag("setupDone"), onClick = onDone)
            else -> SignalButton("Next", Modifier.weight(1f).testTag("setupNext"), onClick = onNext)
        }
    }
}

/** A page's title in the serif with its italic part, and a line under it. */
@Composable
private fun PageTitle(title: String, italic: String, body: String? = null, eyebrow: String? = null) {
    val c = LocalColors.current
    Column(Modifier.padding(top = 26.dp)) {
        if (eyebrow != null) LabelText(eyebrow, c.inkMeta, Modifier.padding(bottom = 14.dp).testTag("setupStep"))
        AskText(title, c.ink, Modifier.semantics { heading() }, target = italic, style = RType.serif(38f, 42f), minSize = 30.sp, maxLines = 4)
        if (body != null) RText(body, RType.sans(15.5f, lineHeight = 22f), c.ink2, Modifier.padding(top = 12.dp))
    }
}

// ---- 1. Welcome -------------------------------------------------------------------------------------------------

@Composable
private fun WelcomePage() {
    Column(Modifier.testTag("setupWelcome")) {
        PageTitle("Nothing happens behind your back", "behind your back", eyebrow = "Welcome to Reins")
        Column(Modifier.padding(top = 18.dp)) {
            Promise(Glyph.Activity, "Every ask waits for you", "Each read, send or change shows exactly what the AI wants.")
            Promise(Glyph.Key, "Your keys stay on this phone", "The server passes requests along and sees nothing inside.")
            Promise(Glyph.Shield, "Trust on your terms", "Allow for an hour, or let Autopilot learn what you always allow.")
            Promise(Glyph.Lock, "One tap stops everything", "Lockdown denies every request at once, until you lift it.", last = true)
        }
    }
}

@Composable
private fun Promise(glyph: Glyph, title: String, detail: String, last: Boolean = false) {
    val c = LocalColors.current
    Row(
        Modifier.fillMaxWidth().then(if (last) Modifier else Modifier.hairlineBelow(c.rule)).padding(vertical = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PlainTile(glyph, 34.dp)
        Column(Modifier.weight(1f)) {
            RText(title, RType.row, c.ink)
            RText(detail, RType.meta, c.inkMeta, Modifier.padding(top = 2.dp))
        }
    }
}

// ---- 2. Requests come to you ---------------------------------------------------------------------------------------

@Composable
private fun NotificationsPage(notifications: NotificationState, screenLock: ScreenLockState) {
    val c = LocalColors.current
    Column(Modifier.testTag("setupNotifications")) {
        PageTitle(
            "Requests come to you",
            "come to you",
            "When an AI asks, this phone rings, even with Reins closed. Answer right from the notification.",
        )
        // The notification they will get, in their theme.
        Column(
            Modifier
                .padding(top = 22.dp)
                .fillMaxWidth()
                .clip(RoundedCornerShape(22.dp))
                .background(c.paperRaised)
                .border(1.dp, c.rule, RoundedCornerShape(22.dp))
                .padding(16.dp)
                .testTag("sampleNotification"),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(20.dp).background(c.inverse, RoundedCornerShape(6.dp)), contentAlignment = Alignment.Center) {
                    // The notification's small icon (the R), as the shade draws it.
                    Image(
                        painterResource(R.drawable.ic_notification),
                        null,
                        Modifier.size(13.dp),
                        colorFilter = ColorFilter.tint(c.onInverse),
                    )
                }
                Spacer(Modifier.width(8.dp))
                RText("Reins", RType.sans(13f, FontWeight.Medium), c.ink, Modifier.weight(1f))
                RText("now", RType.meta, c.inkMeta)
            }
            RText("Claude wants to send an email", RType.row, c.ink, Modifier.padding(top = 10.dp))
            RText("To anna@example.com · “Draft for Friday”", RType.meta, c.inkMeta, Modifier.padding(top = 2.dp), maxLines = 1)
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                SignalButton("Deny", Modifier.weight(1f), style = ButtonStyle.Soft, size = ButtonSize.Small) {}
                SignalButton("Review", Modifier.weight(1f), style = ButtonStyle.Outline, size = ButtonSize.Small) {}
            }
        }
        Column(Modifier.padding(top = 18.dp)) {
            StatusLine(
                Glyph.Bell,
                "Notifications",
                "Requests, grants and your devices only",
                if (notifications.enabled) "On" else "Not yet",
                notifications.enabled,
                Modifier.then(if (notifications.enabled) Modifier.testTag("notificationsOn") else Modifier),
            )
            StatusLine(Glyph.Fingerprint, "Screen lock", "Every approval asks for it", if (screenLock.set) "On" else "Not yet", screenLock.set, last = true)
        }
        // Approving needs the screen lock: better found out here than at the first request.
        if (!screenLock.set) ScreenLockCard(Modifier.padding(top = 14.dp), screenLock::openSettings)
    }
}

/** "Notifications · Requests, grants and your devices only · On": a glyph, what it is, and whether it is. */
@Composable
private fun StatusLine(glyph: Glyph, title: String, detail: String, value: String, done: Boolean, modifier: Modifier = Modifier, last: Boolean = false) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().then(if (last) Modifier else Modifier.hairlineBelow(c.rule)).padding(vertical = 12.dp).semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        PlainTile(glyph, 34.dp)
        Column(Modifier.weight(1f)) {
            RText(title, RType.row, c.ink, maxLines = 1)
            RText(detail, RType.meta, c.inkMeta, maxLines = 2)
        }
        RText(value, RType.sans(13.5f, FontWeight.Medium), if (done) c.success else c.inkMeta, maxLines = 1)
    }
}

// ---- 3. What your AIs can use ----------------------------------------------------------------------------------------

/** The services, the computer and an AI app: all of them connections. */
@Composable
private fun ConnectPage(app: AppViewModel, container: AppContainer, mcpUrl: String, onOpen: (String) -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    val services by container.state.services.collectAsStateWithLifecycle()
    var copied by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.testTag("setupIntegrations")) {
        PageTitle("What your AIs can use", "can use", "They reach these only through this phone, one approved request at a time.")
        Spacer(Modifier.height(20.dp))
        if (services.isEmpty()) {
            Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(c.ink2, size = 36.dp) }
        } else {
            val tiles = services.map { it as ServiceView? } + null
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                tiles.chunked(3).forEach { row ->
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        row.forEach { service ->
                            if (service == null) {
                                IntegrationTile(Modifier.weight(1f), MCP, "Any MCP", "Add yours", connected = false, available = true, glyph = Glyph.Link) { onOpen(MCP) }
                            } else {
                                val count = service.accounts.size
                                IntegrationTile(
                                    Modifier.weight(1f),
                                    service.service,
                                    service.name,
                                    when {
                                        !service.available -> "Not available"
                                        count == 0 -> "Connect"
                                        count == 1 -> "1 account"
                                        else -> "$count accounts"
                                    },
                                    connected = count > 0,
                                    available = service.available,
                                ) { onOpen(service.service) }
                            }
                        }
                        repeat(3 - row.size) { Spacer(Modifier.weight(1f)) }
                    }
                }
            }
        }
        // The computer: the desktop app or `reins login`, scanned or typed in here.
        Column(Modifier.padding(top = 22.dp).testTag("setupComputer")) {
            LabelText("Your computer", c.inkMeta)
            ComputerHowTo(Modifier.padding(top = 8.dp))
            ConnectComputerPanel(app, Modifier.padding(top = 12.dp))
            DesktopAppLink()
        }
        // An AI app: Claude.ai or ChatGPT, through the address it pastes as a custom connector.
        Column(Modifier.padding(top = 18.dp).testTag("setupAi")) {
            LabelText("Claude.ai or ChatGPT", c.inkMeta)
            Row(Modifier.fillMaxWidth().padding(top = 8.dp).hairlineBelow(c.rule).padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(mcpUrl, RType.value, c.ink, Modifier.weight(1f).testTag("mcpUrl"), ltr = true)
                Spacer(Modifier.width(10.dp))
                SignalButton(
                    if (copied && Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) "Copied" else "Copy",
                    Modifier.testTag("copyMcp"),
                    style = ButtonStyle.Soft,
                    size = ButtonSize.Compact,
                ) {
                    copyText(context, "Reins MCP address", mcpUrl)
                    // Android 13 and later confirm a copy themselves.
                    copied = true
                    feedback.play(Event.Copied)
                }
            }
            InlineHelp(
                "custom connectors",
                "In Claude.ai or ChatGPT, open Settings > Connectors, add a custom connector and paste this address.",
                Modifier.padding(top = 4.dp).testTag("connectorHowTo"),
                label = "Paste as a custom connector",
            )
        }
    }
}

@Composable
private fun IntegrationTile(
    modifier: Modifier,
    id: String,
    name: String,
    status: String,
    connected: Boolean,
    available: Boolean,
    glyph: Glyph? = null,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(16.dp)
    Column(
        modifier
            .clip(shape)
            .background(c.paperRaised)
            .border(1.dp, c.rule, shape)
            .pressable(enabled = available, highlight = c.paperSunken, shape = shape, onClick = onClick)
            .padding(12.dp)
            .testTag("setupService:$id"),
    ) {
        Row(verticalAlignment = Alignment.Top) {
            if (glyph != null) PlainTile(glyph, 30.dp) else ServiceAvatar(id, size = 30.dp)
            Spacer(Modifier.weight(1f))
            if (connected) GlyphIcon(Glyph.Check, c.success, size = 16.dp)
        }
        Spacer(Modifier.height(10.dp))
        RText(name, RType.sans(14f, FontWeight.Medium), if (available) c.ink else c.inkMeta, maxLines = 1)
        RText(status, RType.meta, if (connected) c.success else c.inkMeta, maxLines = 1)
    }
}

// ---- 4. How much to ask --------------------------------------------------------------------------------------------

/** How a new AI starts, and whether a model on this phone helps: two decisions, one page. */
@Composable
private fun AskPage(container: AppContainer, viewModel: AutopilotViewModel) {
    val c = LocalColors.current
    val policy by container.state.startingPolicy.collectAsStateWithLifecycle()
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    // Someone who just installed Reins starts with the recommended rule (ask every time); a choice made before stays.
    LaunchedEffect(Unit) {
        if (container.core.startingPolicy() == null) container.chooseStartingPolicy(dev.reins.core.StartingPolicy.ASK_EVERY_TIME)
    }
    Column {
        PageTitle("How much to ask", "to ask")
        Column(Modifier.padding(top = 20.dp).testTag("setupRules")) {
            LabelText("A new AI starts with", c.inkMeta, Modifier.padding(bottom = 4.dp))
            dev.reins.android.ui.grants.StartingRuleChooser(policy, onChoose = container::chooseStartingPolicy)
        }
        Column(Modifier.padding(top = 22.dp).testTag("setupAutopilot")) {
            LabelText("Autopilot · a private model", c.inkMeta, Modifier.padding(bottom = 10.dp))
            val s = settings
            val model = ui.model ?: s?.model
            Row(Modifier.height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(AutopilotMode.MANUAL, AutopilotMode.AUTO).forEach { mode ->
                    TourModeTile(mode, selected = (s?.mode ?: AutopilotMode.MANUAL) == mode, Modifier.weight(1f).fillMaxHeight()) { viewModel.setMode(mode) }
                }
            }
            // Getting the model: a switch, quiet in the background.
            val getting = model != null && (model.state == ModelState.INSTALLED || model.state == ModelState.DOWNLOADING || ui.job != DownloadJob.Idle)
            Row(
                Modifier.fillMaxWidth().padding(top = 14.dp).testTag("modelCard"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    RText(if (model?.state == ModelState.INSTALLED) "The model is here" else "Get the model now", RType.row, c.ink, maxLines = 1)
                    RText(
                        model?.let { "${AutopilotText.modelSize(it)}, on Wi-Fi, runs on this phone" } ?: "Runs on this phone",
                        RType.meta,
                        c.inkMeta,
                        maxLines = 2,
                    )
                }
                Spacer(Modifier.width(10.dp))
                Toggle(getting, Modifier.testTag("getModel"), enabled = model != null && model.state != ModelState.INSTALLED) { on ->
                    if (on) viewModel.download() else viewModel.cancelDownload()
                }
            }
        }
    }
}

/** Manual or Auto as on Trust: its glyph, its name in the serif, what it does; the one picked ringed. */
@Composable
private fun TourModeTile(mode: AutopilotMode, selected: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(20.dp)
    Column(
        modifier
            .clip(shape)
            .background(c.paperRaised)
            .border(if (selected) 2.dp else 1.dp, if (selected) c.ink else c.rule, shape)
            .pressable(shape = shape, onClick = onClick)
            .semantics(mergeDescendants = true) { this.selected = selected }
            .testTag("tourMode:${mode.name}")
            .padding(14.dp),
    ) {
        GlyphIcon(if (mode == AutopilotMode.AUTO) Glyph.Autopilot else Glyph.Person, if (mode == AutopilotMode.AUTO) c.violet else c.ink, size = 18.dp)
        RText(AutopilotText.name(mode), RType.serif(24f, 28f), c.ink, Modifier.padding(top = 8.dp), maxLines = 1)
        RText(if (mode == AutopilotMode.MANUAL) "You decide, with a hint" else AutopilotText.line(mode), RType.meta, c.inkMeta, maxLines = 3)
    }
}

// ---- 5. You're in control -------------------------------------------------------------------------------------------

@Composable
private fun DonePage(container: AppContainer, autopilot: AutopilotViewModel, notifications: NotificationState, screenLock: ScreenLockState) {
    val services by container.state.services.collectAsStateWithLifecycle()
    val connections by container.state.connections.collectAsStateWithLifecycle()
    val ui by autopilot.ui.collectAsStateWithLifecycle()
    val settings by autopilot.settings.collectAsStateWithLifecycle()
    // The vault comes with the account: not something connected here.
    val accounts = services.filter { it.service != "vault" }.sumOf { it.accounts.size }
    val model = (ui.model ?: settings?.model)?.state
    Column(Modifier.testTag("setupFinished")) {
        // A quiet finish: the pond the empty Activity shows next.
        Calm("", Modifier.padding(top = 18.dp), drawingWidth = 260.dp, gap = 0.dp)
        PageTitle("You're in control", "in control")
        Column(Modifier.padding(top = 18.dp)) {
            StatusLine(Glyph.Bell, "Notifications", if (notifications.enabled) "Requests reach you" else "Turn them on in Settings", if (notifications.enabled) "On" else "Off", notifications.enabled)
            StatusLine(Glyph.Fingerprint, "Screen lock", "Every approval asks for it", if (screenLock.set) "On" else "Off", screenLock.set)
            StatusLine(
                Glyph.Apps,
                "Services",
                when (accounts) {
                    0 -> "None yet"
                    1 -> "1 account"
                    else -> "$accounts accounts"
                },
                if (accounts > 0) "$accounts" else "Later",
                accounts > 0,
            )
            StatusLine(
                Glyph.Chip,
                "Private model",
                when (model) {
                    ModelState.INSTALLED -> "On this phone"
                    ModelState.DOWNLOADING -> "Downloading in the background"
                    else -> "Not downloaded"
                },
                when (model) {
                    ModelState.INSTALLED -> "On"
                    ModelState.DOWNLOADING -> "Soon"
                    else -> "Later"
                },
                model == ModelState.INSTALLED || model == ModelState.DOWNLOADING,
            )
            StatusLine(
                Glyph.Laptop,
                "Computers and AI apps",
                if (connections.isEmpty()) "None yet · connect from Connections" else "${connections.size} connected",
                if (connections.isEmpty()) "Later" else "${connections.size}",
                connections.isNotEmpty(),
                last = true,
            )
        }
    }
}

private fun copyText(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}
