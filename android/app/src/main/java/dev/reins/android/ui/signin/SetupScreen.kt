package dev.reins.android.ui.signin

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.BiasAlignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.AppContainer
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalLiveTimers
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.Spinner
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Cue
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.cueUnlessRecent
import dev.reins.android.feedback.play
import dev.reins.android.platform.NotificationState
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.ui.AppViewModel
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.IconTile
import dev.reins.android.ui.autopilot.ModelCard
import dev.reins.android.ui.autopilot.modeGlyph
import dev.reins.android.ui.autopilot.modeTint
import dev.reins.android.ui.nav.Route
import dev.reins.android.ui.pairing.ComputerHowTo
import dev.reins.android.ui.pairing.ConnectComputerPanel
import dev.reins.android.ui.pairing.DesktopAppLink
import dev.reins.core.ModelState
import dev.reins.core.ServiceView

/** The pages of the setup after signing in, in order. */
enum class SetupPage { Welcome, Notifications, Integrations, Autopilot, Computer, Ai, Done }

/**
 * Right after signing in or creating an account (once per account, and again from Settings > Take the tour): what
 * Reins is and how a request travels, notifications, the integrations an AI can use, the private on-device model,
 * connecting the computer and an AI app, and a summary. Every page can be skipped; "Skip setup" ends it. An
 * integration opens its own page over the setup and Back returns to the same page ([AppViewModel.setupPage]).
 */
@Composable
fun SetupScreen(app: AppViewModel, container: AppContainer, autopilot: AutopilotViewModel, serverUrl: String) {
    val feedback = LocalFeedback.current
    val notifications = rememberNotificationState()
    val page = app.setupPage
    val pages = SetupPage.entries
    val go = { next: SetupPage ->
        if (next.ordinal > page.ordinal) feedback.cue(Cue.Open) else feedback.cueUnlessRecent(Cue.Close)
        if (next == SetupPage.Ai) app.resetConnect()
        app.setupPage = next
    }
    val next = { go(pages[(page.ordinal + 1).coerceAtMost(pages.lastIndex)]) }
    val back = { go(pages[(page.ordinal - 1).coerceAtLeast(0)]) }
    BackHandler(enabled = page != SetupPage.Welcome, onBack = back)

    val c = LocalColors.current
    Column(
        Modifier
            .fillMaxSize()
            .background(c.background)
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
            Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                when (shown) {
                    SetupPage.Welcome -> WelcomePage()
                    SetupPage.Notifications -> NotificationsPage(notifications)
                    SetupPage.Integrations -> IntegrationsPage(container) { id ->
                        app.open(
                            when (id) {
                                "gmail" -> Route.Gmail
                                MCP -> Route.McpAdd
                                else -> Route.Service(id)
                            },
                        )
                    }
                    SetupPage.Autopilot -> AutopilotPage(autopilot)
                    SetupPage.Computer -> ComputerPage(app)
                    SetupPage.Ai -> AiPage(AccountRules.mcpUrl(serverUrl))
                    SetupPage.Done -> DonePage(container, autopilot, notifications)
                }
                Spacer(Modifier.height(16.dp))
            }
        }
        SetupButtons(page, notifications, onBack = back, onNext = next, onDone = app::finishSetup)
    }
}

private const val MCP = "mcp"

/** A segment per page (filled up to this one), and "Skip setup". */
@Composable
private fun SetupTopBar(page: SetupPage, onSkip: () -> Unit) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(start = 24.dp, end = 12.dp, top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        Row(Modifier.weight(1f).testTag("setupProgress"), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            SetupPage.entries.forEach { p ->
                val fill by animateColorAsState(if (p.ordinal <= page.ordinal) c.accent else c.controlFill.copy(alpha = if (c.dark) 0.16f else 0.1f), label = "segment")
                Box(Modifier.weight(1f).height(4.dp).clip(CircleShape).background(fill))
            }
        }
        Spacer(Modifier.width(8.dp))
        if (page != SetupPage.Done) {
            TextLink("Skip setup", "setupSkip", onClick = onSkip)
        } else {
            Spacer(Modifier.height(36.dp))
        }
    }
}

/** Back and the page's forward button; the notifications page's forward button asks for them first. */
@Composable
private fun SetupButtons(page: SetupPage, notifications: NotificationState, onBack: () -> Unit, onNext: () -> Unit, onDone: () -> Unit) {
    Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 8.dp, bottom = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        val asking = page == SetupPage.Notifications && !notifications.enabled
        if (asking) {
            CapsuleButton("Not now", Modifier.weight(1f).testTag("notificationsLater"), style = ButtonStyle.Ghost, onClick = onNext)
        } else if (page != SetupPage.Welcome && page != SetupPage.Done) {
            CapsuleButton("Back", Modifier.weight(1f).testTag("setupBack"), style = ButtonStyle.Ghost, onClick = onBack)
        }
        when {
            page == SetupPage.Welcome -> CapsuleButton("Show me around", Modifier.weight(1f).testTag("setupNext"), style = ButtonStyle.Primary, onClick = onNext)
            asking ->
                CapsuleButton("Allow notifications", Modifier.weight(1f).testTag("allowNotifications"), style = ButtonStyle.Accent, glyph = Glyph.Bell) {
                    notifications.request()
                }
            page == SetupPage.Done -> CapsuleButton("Start using Reins", Modifier.weight(1f).testTag("setupDone"), style = ButtonStyle.Primary, onClick = onDone)
            else -> CapsuleButton("Next", Modifier.weight(1f).testTag("setupNext"), style = ButtonStyle.Primary, onClick = onNext)
        }
    }
}

/** A page's heading: an eyebrow, a glyph on a soft tile, the title and a line under it. */
@Composable
private fun PageHeader(eyebrow: String, glyph: Glyph, title: String, body: String, tint: Color? = null) {
    val c = LocalColors.current
    val t = tint ?: c.accent
    Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 28.dp)) {
        RText(
            eyebrow.uppercase(),
            RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
            c.secondary,
            Modifier.testTag("setupStep"),
        )
        Spacer(Modifier.height(16.dp))
        Box(Modifier.size(56.dp).background(t.copy(alpha = 0.13f), RoundedCornerShape(18.dp)), contentAlignment = Alignment.Center) {
            GlyphIcon(glyph, t, size = 30.dp)
        }
        Spacer(Modifier.height(14.dp))
        RText(title, RType.sans(28f, FontWeight.SemiBold, lineHeight = 34f), c.text)
        Spacer(Modifier.height(8.dp))
        RText(body, RType.sans(15.5f, lineHeight = 22f), c.secondary)
    }
}

// ---- 1. Welcome -------------------------------------------------------------------------------------------------

@Composable
private fun WelcomePage() {
    val c = LocalColors.current
    Column(Modifier.testTag("setupWelcome")) {
        Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 28.dp)) {
            RText(
                "WELCOME TO REINS",
                RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
                c.secondary,
                Modifier.testTag("setupStep"),
            )
            Spacer(Modifier.height(12.dp))
            RText("Your AIs ask.\nYou decide.", RType.sans(34f, FontWeight.SemiBold, lineHeight = 40f), c.text)
            Spacer(Modifier.height(10.dp))
            RText(
                "Reins sits between your AI agents and your accounts. Whenever one wants to read, send or change " +
                    "something, this phone asks you first.",
                RType.sans(15.5f, lineHeight = 22f),
                c.secondary,
            )
        }
        RequestFlow(Modifier.padding(horizontal = 12.dp, vertical = 22.dp))
        Group {
            Promise(Glyph.Hand, c.accent, "Nothing happens behind your back", "Every read, send or change waits for a tap, with exactly what the AI wants to do.")
            Hairline(inset = 70.dp)
            Promise(Glyph.Key, c.success, "Your keys stay on this phone", "Access to your mail, chats and code is kept encrypted here. The server only passes requests along.")
            Hairline(inset = 70.dp)
            Promise(Glyph.Clock, c.search, "Trust on your terms", "Grant a few minutes of access, or let Autopilot learn what you always allow.")
            Hairline(inset = 70.dp)
            Promise(Glyph.Lock, c.warning, "One tap to stop everything", "Lockdown denies every request at once, until you lift it.")
        }
    }
}

@Composable
private fun Promise(glyph: Glyph, tint: Color, title: String, body: String) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.Top) {
        IconTile(glyph, tint, size = 40.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(16f, FontWeight.SemiBold), c.text)
            RText(body, RType.sans(13.5f, lineHeight = 19f), c.secondary, Modifier.padding(top = 2.dp))
        }
    }
}

/**
 * How a request travels: from the AI to this phone (blue), a pause while you decide (the phone's ring and a tick),
 * then on to your app (green). Still, mid-way, when clocks are frozen (screenshots).
 */
@Composable
private fun RequestFlow(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val t = if (LocalLiveTimers.current) {
        rememberInfiniteTransition(label = "flow").animateFloat(0f, 1f, infiniteRepeatable(tween(4200, easing = LinearEasing)), label = "flowT").value
    } else {
        0.5f
    }
    val toPhone = (t / 0.32f).coerceIn(0f, 1f)
    val deciding = t in 0.32f..0.62f
    val approved = t >= 0.5f
    val toApp = ((t - 0.62f) / 0.3f).coerceIn(0f, 1f)
    val tileCenter = 34.dp
    Box(modifier.fillMaxWidth().height(118.dp).testTag("requestFlow")) {
        Canvas(Modifier.fillMaxSize()) {
            val y = tileCenter.toPx()
            val x0 = size.width * 0.16f
            val x1 = size.width * 0.5f
            val x2 = size.width * 0.84f
            val dash = PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(), 6.dp.toPx()))
            val track = c.tertiary.copy(alpha = 0.5f)
            drawLine(track, Offset(x0 + 34.dp.toPx(), y), Offset(x1 - 40.dp.toPx(), y), 2.dp.toPx(), StrokeCap.Round, dash)
            drawLine(track, Offset(x1 + 40.dp.toPx(), y), Offset(x2 - 34.dp.toPx(), y), 2.dp.toPx(), StrokeCap.Round, dash)
            if (t < 0.32f) {
                val x = (x0 + 34.dp.toPx()) + (x1 - 40.dp.toPx() - x0 - 34.dp.toPx()) * toPhone
                drawCircle(c.search, 6.dp.toPx(), Offset(x, y))
            }
            if (deciding) {
                val p = (t - 0.32f) / 0.3f
                drawCircle(c.accent.copy(alpha = 0.35f * (1f - p)), 36.dp.toPx() + 14.dp.toPx() * p, Offset(x1, y), style = Stroke(2.dp.toPx()))
            }
            if (t > 0.62f && t < 0.92f) {
                val x = (x1 + 40.dp.toPx()) + (x2 - 34.dp.toPx() - x1 - 40.dp.toPx()) * toApp
                drawCircle(c.success, 6.dp.toPx(), Offset(x, y))
            }
        }
        FlowNode(Glyph.Sparkle, c.search, "Your AI", "asks", BiasAlignment(-0.68f, -1f), 52.dp)
        FlowNode(if (approved && t < 0.95f) Glyph.Check else Glyph.ShieldCheck, c.accent, "This phone", "you decide", BiasAlignment(0f, -1f), 64.dp, filled = deciding || (approved && t < 0.95f))
        FlowNode(Glyph.Mail, c.success, "Your apps", "only then", BiasAlignment(0.68f, -1f), 52.dp, filled = t in 0.9f..0.98f)
    }
}

@Composable
private fun androidx.compose.foundation.layout.BoxScope.FlowNode(
    glyph: Glyph,
    tint: Color,
    title: String,
    subtitle: String,
    alignment: Alignment,
    size: androidx.compose.ui.unit.Dp,
    filled: Boolean = false,
) {
    val c = LocalColors.current
    Column(Modifier.align(alignment).width(96.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Box(Modifier.height(68.dp), contentAlignment = Alignment.Center) { IconTile(glyph, tint, size = size, filled = filled) }
        RText(title, RType.sans(13.5f, FontWeight.SemiBold), c.text, maxLines = 1, align = TextAlign.Center)
        RText(subtitle, RType.sans(12f), c.secondary, maxLines = 1, align = TextAlign.Center)
    }
}

// ---- 2. Notifications -------------------------------------------------------------------------------------------

@Composable
private fun NotificationsPage(notifications: NotificationState) {
    val c = LocalColors.current
    Column(Modifier.testTag("setupNotifications")) {
        PageHeader(
            "Step 1 · Notifications",
            Glyph.Bell,
            "Requests come to you",
            "When an AI asks for something, this phone rings, even with Reins closed. You approve or deny right from the notification.",
        )
        // What one looks like.
        Column(
            Modifier
                .padding(start = 16.dp, end = 16.dp, top = 24.dp)
                .fillMaxWidth()
                .background(c.elevated, RoundedCornerShape(22.dp))
                .padding(16.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(22.dp).background(c.accent, CircleShape), contentAlignment = Alignment.Center) {
                    GlyphIcon(Glyph.ShieldCheck, Color.White, size = 13.dp, weight = 2f)
                }
                Spacer(Modifier.width(8.dp))
                RText("Reins · now", RType.sans(12.5f, FontWeight.Medium), c.secondary)
            }
            Spacer(Modifier.height(10.dp))
            RText("Claude wants to send an email", RType.sans(15.5f, FontWeight.SemiBold), c.text)
            RText("To anna@example.com · \"Draft for Friday\"", RType.sans(13.5f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
            Row(Modifier.padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Tag("Approve", tint = c.success)
                Tag("Deny", tint = c.danger)
            }
        }
        Spacer(Modifier.height(18.dp))
        if (notifications.enabled) {
            Row(Modifier.padding(horizontal = 24.dp).testTag("notificationsOn"), verticalAlignment = Alignment.CenterVertically) {
                IconTile(Glyph.Check, c.success, size = 30.dp, filled = true)
                Spacer(Modifier.width(10.dp))
                RText("Notifications are on. You're all set here.", RType.sans(15f, FontWeight.Medium), c.text)
            }
        } else {
            RText(
                "Without notifications an AI waits for an answer that never comes, and its request times out. " +
                    "Reins only notifies you about requests, grants and your own devices.",
                RType.sans(13.5f, lineHeight = 19f),
                c.tertiary,
                Modifier.padding(horizontal = 24.dp),
            )
        }
    }
}

// ---- 3. Integrations --------------------------------------------------------------------------------------------

@Composable
private fun IntegrationsPage(container: AppContainer, onOpen: (String) -> Unit) {
    val c = LocalColors.current
    val services by container.state.services.collectAsStateWithLifecycle()
    Column(Modifier.testTag("setupIntegrations")) {
        PageHeader(
            "Step 2 · Integrations",
            Glyph.Apps,
            "What your AIs can use",
            "Connect the accounts you want your AIs to work with. They can only reach them through this phone, one approved request at a time.",
            tint = c.read,
        )
        Spacer(Modifier.height(20.dp))
        if (services.isEmpty()) {
            Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { Spinner(c.accent, size = 36.dp) }
        } else {
            val tiles = services.map { it as ServiceView? } + null
            Column(Modifier.padding(horizontal = 16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                tiles.chunked(2).forEach { row ->
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        row.forEach { service ->
                            if (service == null) {
                                IntegrationTile(Modifier.weight(1f), "mcp", "Any MCP server", "Add your own tools", connected = false, available = true, glyph = Glyph.Link) { onOpen(MCP) }
                            } else {
                                val count = service.accounts.size
                                IntegrationTile(
                                    Modifier.weight(1f),
                                    service.service,
                                    service.name,
                                    when {
                                        !service.available -> "Not available"
                                        count == 0 -> "Tap to connect"
                                        count == 1 -> "1 account"
                                        else -> "$count accounts"
                                    },
                                    connected = count > 0,
                                    available = service.available,
                                ) { onOpen(service.service) }
                            }
                        }
                        if (row.size == 1) Spacer(Modifier.weight(1f))
                    }
                }
            }
        }
        RText(
            "Nothing to connect right now? Add integrations any time from Activity > Integrations.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 24.dp, end = 24.dp, top = 16.dp),
        )
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
    Column(
        modifier
            .background(c.elevated, RoundedCornerShape(18.dp))
            .pressable(enabled = available, highlight = c.controlFill, shape = RoundedCornerShape(18.dp), onClick = onClick)
            .padding(14.dp)
            .testTag("setupService:$id"),
    ) {
        Row(verticalAlignment = Alignment.Top) {
            if (glyph != null) IconTile(glyph, c.accent, size = 40.dp) else ServiceAvatar(id, size = 40.dp)
            Spacer(Modifier.weight(1f))
            if (connected) IconTile(Glyph.Check, c.success, size = 22.dp, filled = true)
        }
        Spacer(Modifier.height(10.dp))
        RText(name, RType.sans(15f, FontWeight.SemiBold), if (available) c.text else c.tertiary, maxLines = 1)
        RText(status, RType.sans(12.5f), if (connected) c.success else c.tertiary, Modifier.padding(top = 1.dp), maxLines = 1)
    }
}

// ---- 4. Autopilot -----------------------------------------------------------------------------------------------

@Composable
private fun AutopilotPage(viewModel: AutopilotViewModel) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    Column(Modifier.testTag("setupAutopilot")) {
        PageHeader(
            "Step 3 · Autopilot",
            Glyph.Chip,
            "A private model that learns your rules",
            "Optionally, a small model on this phone learns from your answers and suggests, or takes, the easy decisions. " +
                "Requests are judged right here; nothing is sent off to be decided.",
        )
        Group(Modifier.padding(top = 20.dp), header = "Modes", footer = "You start in Manual. Change modes any time with the round button next to the tabs.") {
            AutopilotText.modes.forEachIndexed { i, mode ->
                if (i > 0) Hairline(inset = 66.dp)
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 11.dp), verticalAlignment = Alignment.CenterVertically) {
                    IconTile(modeGlyph(mode), modeTint(mode, c), size = 36.dp)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        RText(AutopilotText.name(mode), RType.sans(15.5f, FontWeight.SemiBold), c.text)
                        RText(AutopilotText.line(mode), RType.sans(13f), c.secondary, maxLines = 2)
                    }
                    if (AutopilotText.needsModel(mode)) Tag("Model")
                }
            }
        }
        ModelCard(
            model = ui.model ?: settings?.model,
            job = ui.job,
            wifiOnly = settings?.wifiOnly ?: true,
            onDownload = viewModel::download,
            onCancel = viewModel::cancelDownload,
            onDelete = viewModel::deleteModel,
            onWifiOnly = viewModel::setWifiOnly,
        )
    }
}

// ---- 5. Your computer -------------------------------------------------------------------------------------------

@Composable
private fun ComputerPage(app: AppViewModel) {
    Column(Modifier.testTag("setupComputer")) {
        PageHeader(
            "Step 4 · Your computer",
            Glyph.Laptop,
            "Connect your computer",
            "Coding agents and desktop AI apps on your computer go through Reins too: they ask, this phone answers.",
        )
        ComputerHowTo(Modifier.padding(start = 24.dp, end = 24.dp, top = 10.dp))
        ConnectComputerPanel(app, Modifier.padding(horizontal = 16.dp, vertical = 20.dp))
        DesktopAppLink()
    }
}

// ---- 6. Your AI app ---------------------------------------------------------------------------------------------

@Composable
private fun AiPage(mcpUrl: String) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    var copied by rememberSaveable { mutableStateOf(false) }
    Column(Modifier.testTag("setupAi")) {
        PageHeader(
            "Step 5 · Your AI app",
            Glyph.Sparkle,
            "Connect Claude.ai or ChatGPT",
            "Add Reins to your AI app with this address. Whatever it then wants to read or send asks this phone first.",
            tint = c.search,
        )
        Group(Modifier.padding(top = 18.dp)) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 10.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(mcpUrl, RType.mono(15f, FontWeight.Medium), c.text, Modifier.weight(1f).testTag("mcpUrl"), ltr = true)
                Spacer(Modifier.width(10.dp))
                CapsuleButton(
                    if (copied && Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) "Copied" else "Copy",
                    Modifier.testTag("copyMcp"),
                    style = ButtonStyle.Secondary,
                    glyph = Glyph.Copy,
                    compact = true,
                ) {
                    copyText(context, "Reins MCP address", mcpUrl)
                    // Android 13 and later confirm a copy themselves.
                    copied = true
                    feedback.play(Event.Copied)
                }
            }
        }
        RText(
            "In Claude.ai or ChatGPT, open Settings > Connectors, add a custom connector and paste this address.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 10.dp).testTag("connectorHowTo"),
        )
    }
}

// ---- 7. Done ----------------------------------------------------------------------------------------------------

@Composable
private fun DonePage(container: AppContainer, autopilot: AutopilotViewModel, notifications: NotificationState) {
    val c = LocalColors.current
    val services by container.state.services.collectAsStateWithLifecycle()
    val connections by container.state.connections.collectAsStateWithLifecycle()
    val ui by autopilot.ui.collectAsStateWithLifecycle()
    val settings by autopilot.settings.collectAsStateWithLifecycle()
    val accounts = services.sumOf { it.accounts.size }
    val model = (ui.model ?: settings?.model)?.state
    Column(Modifier.testTag("setupFinished")) {
        PageHeader(
            "All set",
            Glyph.ShieldCheck,
            "You're in control",
            "Your AIs can now do real work for you, and nothing happens without your say. Here's where things stand:",
            tint = c.success,
        )
        Group(Modifier.padding(top = 20.dp)) {
            Checklist(Glyph.Bell, "Notifications", if (notifications.enabled) "On" else "Off: turn them on from Activity", notifications.enabled)
            Hairline(inset = 66.dp)
            Checklist(
                Glyph.Apps,
                "Integrations",
                when (accounts) {
                    0 -> "None yet: add them from Activity > Integrations"
                    1 -> "1 account connected"
                    else -> "$accounts accounts connected"
                },
                accounts > 0,
            )
            Hairline(inset = 66.dp)
            Checklist(
                Glyph.Chip,
                "Private model",
                when (model) {
                    ModelState.INSTALLED -> "Installed on this phone"
                    ModelState.DOWNLOADING -> "Downloading in the background"
                    else -> "Not downloaded: Manual mode needs none"
                },
                model == ModelState.INSTALLED || model == ModelState.DOWNLOADING,
            )
            Hairline(inset = 66.dp)
            Checklist(
                Glyph.Laptop,
                "Computers and AI apps",
                when (val n = connections.size) {
                    0 -> "None yet: connect one from Settings"
                    1 -> "1 connected"
                    else -> "$n connected"
                },
                connections.isNotEmpty(),
            )
        }
        RText(
            "Everything here can be changed later in Settings, where you can also take this tour again.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 24.dp, end = 24.dp, top = 14.dp),
        )
    }
}

@Composable
private fun Checklist(glyph: Glyph, title: String, detail: String, done: Boolean) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        IconTile(glyph, if (done) c.success else c.tertiary, size = 36.dp)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(15.5f, FontWeight.SemiBold), c.text)
            RText(detail, RType.sans(13f), if (done) c.secondary else c.tertiary, maxLines = 2)
        }
        if (done) IconTile(Glyph.Check, c.success, size = 24.dp, filled = true)
    }
}

private fun copyText(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}
