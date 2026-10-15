package dev.reins.android.ui.approval

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.ArrowBack
import androidx.compose.material.icons.automirrored.rounded.HelpOutline
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.LocalConnections
import dev.reins.android.ui.common.findConnection
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalView
import kotlin.math.roundToInt

/** A page pushed inside the approval screen; the dock stays under every one. */
sealed interface ApprovalPage {
    data class Commits(val ref: Int) : ApprovalPage

    data class Commit(val ref: Int, val index: Int) : ApprovalPage

    data class Files(val ref: Int) : ApprovalPage

    /** Long text folded out of the main page (a search, a tool's arguments). */
    data class Text(val title: String, val body: String, val mono: Boolean, val tag: String) : ApprovalPage
}

/** What slides up over the approval screen. */
enum class ApprovalOverlay { Remember, Autopilot, Grant }

/** A (?) explanation, shown in a bubble under the button at [y] (root coordinates, px). */
data class HelpTip(val key: String, val text: String, val y: Float)

/** The approval screen's navigation: pages pushed, the sheet up, the (?) bubble open. */
@Stable
class ApprovalNav {
    val pages = mutableStateListOf<ApprovalPage>()
    var overlay by mutableStateOf<ApprovalOverlay?>(null)
    var help by mutableStateOf<HelpTip?>(null)

    /** The risk did not fit on the field in two lines: the field says its word, the first fact row its reason. */
    var riskInFacts by mutableStateOf(false)

    fun push(page: ApprovalPage) {
        help = null
        pages.add(page)
    }

    /** Closes what is on top (the bubble, a sheet, a page); false when on the main page with nothing open. */
    fun back(): Boolean = when {
        help != null -> {
            help = null
            true
        }
        overlay != null -> {
            overlay = null
            true
        }
        pages.isNotEmpty() -> {
            pages.removeAt(pages.lastIndex)
            true
        }
        else -> false
    }
}

val LocalApprovalNav = staticCompositionLocalOf { ApprovalNav() }

/**
 * An Activity entry shown with the approval's own screen, read-only: how it ended (the hero's pill, in [tint]), what
 * goes [below] the facts, and the [footer] in place of the decision dock.
 */
class ApprovalRecord(
    val outcome: String,
    val icon: androidx.compose.ui.graphics.vector.ImageVector,
    val tint: (RColors) -> Color,
    val below: @Composable () -> Unit = {},
    val footer: (@Composable () -> Unit)? = null,
    /** How risky the model rated it, and whether Overnight held it; null when it was not rated. */
    val risk: dev.reins.core.RiskView? = null,
    /** How the ask is told: done, or (denied, failed) refused. */
    val tense: dev.reins.android.ui.common.Tense = dev.reins.android.ui.common.Tense.Done,
    /** The core's reading of the request (its label) and where a command ran, as the entry kept them. */
    val label: dev.reins.core.RequestLabel? = null,
    val cwd: String? = null,
)

/** The entry the approval screen shows again, or null for a request that waits. */
val LocalApprovalRecord = staticCompositionLocalOf<ApprovalRecord?> { null }

/** The surfaces of the approval screen, from the palette (the design is dark first; light mode follows it). */
internal object Surfaces {
    fun dock(c: RColors) = if (c.dark) Color(0xFF101012) else Color(0xFFFFFFFF)
    fun dockEdge(c: RColors) = if (c.dark) Color(0xFF1E1E22) else c.hairline
    fun group(c: RColors) = c.elevated
    fun groupEdge(c: RColors) = if (c.dark) Color(0xFF1B1B1F) else c.hairline
    fun raised(c: RColors) = if (c.dark) Color(0xFF232327) else Color(0xFFE7E7EB)
    fun sunken(c: RColors) = c.paperSunken
    fun segment(c: RColors) = if (c.dark) Color(0xFF1C1C20) else Color(0xFFE7E7EB)
    fun sheet(c: RColors) = if (c.dark) Color(0xFF17171A) else Color(0xFFFFFFFF)
    fun bubble(c: RColors) = if (c.dark) Color(0xFF26262B) else Color(0xFF2A2A30)
}

/**
 * A request that waits for the user, full screen: who asks and how long it waits at the top, a hero that says what in
 * one sentence with Autopilot's score, the facts below (long lists folded into pages), and the dock at the bottom with
 * how long to remember it and Deny / Approve. Approving asks for the screen lock or biometrics first.
 */
@Composable
fun ApprovalScreen(
    viewModel: ApprovalViewModel,
    authenticator: Authenticator,
    onDone: () -> Unit,
    onOpenAutopilot: () -> Unit = {},
    /** An Activity entry: read-only, with its outcome, more below the facts and its own footer. */
    record: ApprovalRecord? = null,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val nav = remember { ApprovalNav() }
    val entered = remember { androidx.compose.animation.core.MutableTransitionState(false).apply { targetState = true } }
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    BackHandler(enabled = nav.help != null || nav.overlay != null || nav.pages.isNotEmpty()) {
        if (nav.overlay != null) feedback.play(Event.Close)
        nav.back()
    }
    val close = {
        feedback.play(Event.Close)
        onDone()
    }
    CompositionLocalProvider(LocalApprovalNav provides nav, LocalApprovalRecord provides record) {
        Box(Modifier.fillMaxSize().background(c.background).testTag(if (record == null) "approvalScreen" else "entryApproval")) {
            val view = ui.view
            if (view == null) {
                Column(Modifier.fillMaxSize()) {
                    Row(Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).padding(start = 8.dp, top = 8.dp)) {
                        CloseButton(close)
                    }
                    Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                        if (ui.loading) Spinner(c.secondary, size = 32.dp)
                        ui.error?.let {
                            Banner(it, Modifier.padding(top = 32.dp), BannerKind.Error)
                            CapsuleButton("Close", Modifier.padding(top = 16.dp), style = ButtonStyle.Secondary, onClick = close)
                        }
                    }
                }
                return@Box
            }
            Column(Modifier.fillMaxSize()) {
                val page = nav.pages.lastOrNull()
                if (page == null) {
                    Hero(view, ui, viewModel, close, back = record != null, entered = entered)
                } else {
                    // Drilled in: the field becomes a band with the ask and the timer.
                    val (ask, target) = askWords(view)
                    AskBand(askField(view), ask, target, onBack = {
                        feedback.play(Event.Close)
                        nav.back()
                    }) { TimerPill(view) }
                }
                AnimatedContent(
                    targetState = nav.pages.size to page,
                    modifier = Modifier.weight(1f).fillMaxWidth(),
                    transitionSpec = {
                        val forward = targetState.first >= initialState.first
                        (slideInHorizontally(tween(260)) { if (forward) it / 3 else -it / 3 } + fadeIn(tween(200)))
                            .togetherWith(slideOutHorizontally(tween(220)) { if (forward) -it / 4 else it / 4 } + fadeOut(tween(160)))
                    },
                    label = "page",
                ) { (_, shown) ->
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                        if (shown != null) {
                            RText(
                                pageTitle(view, shown),
                                RType.serif(28f, 32f, italic = shown !is ApprovalPage.Text),
                                c.ink,
                                Modifier.padding(start = 22.dp, end = 22.dp, top = 18.dp, bottom = 4.dp).testTag("pageTitle"),
                                maxLines = 2,
                            )
                        }
                        when (shown) {
                            null -> {
                                MainPage(view, ui, viewModel)
                                record?.below?.invoke()
                            }
                            is ApprovalPage.Commits -> CommitsPage(view, shown.ref)
                            is ApprovalPage.Commit -> CommitPage(view, shown.ref, shown.index)
                            is ApprovalPage.Files -> FilesPage(view, shown.ref)
                            is ApprovalPage.Text -> TextPage(shown)
                        }
                        Spacer(Modifier.height(20.dp))
                    }
                }
                if (record == null) Dock(view, ui, viewModel, authenticator) else record.footer?.invoke()
            }
            HelpBubble(nav)
            if (record == null) ApprovalOverlays(view, ui, viewModel, onOpenAutopilot)
        }
    }
}

/** The page's own title in the bar: "8 commits", a commit's short id, "12 files". */
private fun pageTitle(view: ApprovalView, page: ApprovalPage): String = when (page) {
    is ApprovalPage.Commits -> view.git?.refs?.getOrNull(page.ref)?.let { commitsWord(it.commitCount.toInt()) }.orEmpty()
    is ApprovalPage.Commit -> view.git?.refs?.getOrNull(page.ref)?.commits?.getOrNull(page.index)?.shortSha.orEmpty()
    is ApprovalPage.Files -> view.git?.refs?.getOrNull(page.ref)?.let { filesWord(it.filesChanged.toInt()) }.orEmpty()
    is ApprovalPage.Text -> page.title
}

internal fun commitsWord(n: Int) = if (n == 1) "1 commit" else "$n commits"

internal fun filesWord(n: Int) = if (n == 1) "1 file" else "$n files"

@Composable
private fun CloseButton(onClick: () -> Unit, back: Boolean = false) {
    val c = LocalColors.current
    val label = if (back) "Back" else "Close"
    Box(
        Modifier
            .size(44.dp)
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label }
            .testTag(if (back) "back" else "closeSheet"),
        contentAlignment = Alignment.Center,
    ) { Icon(if (back) Icons.AutoMirrored.Rounded.ArrowBack else Icons.Rounded.Close, null, Modifier.size(24.dp), tint = c.text) }
}

/** The (?) bubble: one short explanation under the button that opened it; a tap anywhere closes it. */
@Composable
private fun HelpBubble(nav: ApprovalNav) {
    val c = LocalColors.current
    val tip = nav.help
    var rootY by remember { mutableStateOf(0f) }
    Box(Modifier.fillMaxSize().onGloballyPositioned { rootY = it.positionInRoot().y }) {
        AnimatedVisibility(tip != null, enter = fadeIn(tween(140)), exit = fadeOut(tween(120))) {
            Box(
                Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures { nav.help = null } },
            ) {
                val shown = tip ?: return@Box
                val density = LocalDensity.current
                RText(
                    shown.text,
                    RType.sans(13.5f, lineHeight = 19.5f),
                    Color(0xFFE8E8EA),
                    Modifier
                        .offset { IntOffset(0, (shown.y - rootY + with(density) { 10.dp.toPx() }).roundToInt()) }
                        .padding(horizontal = 40.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(16.dp))
                        .background(Surfaces.bubble(c))
                        .padding(horizontal = 15.dp, vertical = 13.dp)
                        .testTag("helpBubble"),
                )
            }
        }
    }
}

/** A round (?) of its own, never inside another tappable thing; it opens [text] in the bubble. */
@Composable
internal fun HelpDot(key: String, text: String, modifier: Modifier = Modifier, size: androidx.compose.ui.unit.Dp = 32.dp, onField: Boolean = false) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    var bottom by remember { mutableStateOf(0f) }
    val open = nav.help?.key == key
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(if (onField) (if (open) Color.White.copy(alpha = 0.32f) else c.onFieldWash) else if (open) c.text.copy(alpha = 0.18f) else c.text.copy(alpha = 0.08f))
            .onGloballyPositioned { bottom = it.positionInRoot().y + it.size.height }
            .pressable(shape = CircleShape, label = "More about this") {
                nav.help = if (open) null else HelpTip(key, text, bottom)
                feedback.play(Event.expand(!open))
            }
            .semantics { contentDescription = "More about this" }
            .testTag("help:$key"),
        contentAlignment = Alignment.Center,
    ) {
        dev.reins.android.design.GlyphIcon(
            dev.reins.android.design.Glyph.Help,
            if (onField) c.onField else if (open) c.text else c.secondary,
            size = size * 0.62f,
        )
    }
}

/** Gaps used across the screen. */
internal val Gap = Arrangement.spacedBy(10.dp)
