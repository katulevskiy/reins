package dev.reins.android.design

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.MutableTransitionState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.union
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInParent
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play

// Reins Signal components (HANDOFF §8, screens/png/Components.png, values from screens/source/Components.dc.html).
// Built for phase 2; screens switch to them board by board.

// ---- Buttons ---------------------------------------------------------------------------------------------------

/** The two button sizes of the boards: 54 dp (16.5 sp) and small, 40 dp (14.5 sp). */
enum class ButtonSize(val height: Dp, val padding: Dp, val text: Float, val glyph: Dp) {
    Large(54.dp, 22.dp, 16.5f, 20.dp),
    Small(40.dp, 16.dp, 14.5f, 18.dp),

    /** The boards' 34 dp buttons inside rows and cards (Later · Install, Resume · Delete). */
    Compact(34.dp, 12.dp, 13.5f, 16.dp),
}

/**
 * A Signal button at the boards' sizes. Pressed dims to 55 %, disabled draws at 40 %, focus is a 2 dp ring 3 dp out
 * (violet on paper, white on a field). [showFocus] draws the ring regardless (the gallery's "Focused").
 */
@Composable
fun SignalButton(
    title: String,
    modifier: Modifier = Modifier,
    style: ButtonStyle = ButtonStyle.Inverse,
    size: ButtonSize = ButtonSize.Large,
    glyph: Glyph? = null,
    leading: (@Composable () -> Unit)? = null,
    enabled: Boolean = true,
    showFocus: Boolean = false,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val (bg, fg) = style.colors(c)
    val source = remember { MutableInteractionSource() }
    val focused by source.collectIsFocusedAsState()
    Row(
        modifier
            .height(size.height)
            .graphicsLayer { alpha = if (enabled) 1f else DISABLED_ALPHA }
            .focusRing(focused || showFocus, CircleShape, if (style.onField) c.onField else c.violet)
            .clip(CircleShape)
            .background(bg)
            .then(if (style == ButtonStyle.Outline) Modifier.border(1.5.dp, c.rule2, CircleShape) else Modifier)
            .pressable(enabled = enabled, shape = CircleShape, interactionSource = source, onClick = onClick)
            .padding(horizontal = size.padding),
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            leading != null -> leading()
            glyph != null -> GlyphIcon(glyph, fg, size = size.glyph)
        }
        RText(title, RType.sans(size.text, FontWeight.SemiBold), fg, maxLines = 1)
    }
}

/** A round 40 dp button: a raised plate with a hairline ring on paper, the wash on a field. */
@Composable
fun RoundIconButton(glyph: Glyph, label: String, modifier: Modifier = Modifier, onField: Boolean = false, size: Dp = 40.dp, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        modifier
            .size(size)
            .clip(CircleShape)
            .background(if (onField) c.onFieldWash else c.paperRaised)
            .then(if (onField) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        GlyphIcon(glyph, if (onField) c.onField else c.ink, size = if (onField) 18.dp else 20.dp)
    }
}

// ---- Allow for: the chooser ---------------------------------------------------------------------------------------

/** One choice of a [Chooser]: its value, its words, its test tag. */
@Immutable
data class Choice<T>(val value: T, val label: String, val tag: String = "")

/**
 * "Once · 1 hour · 1 day · Custom ▾": a sunken well with the chosen segment raised. The rest of the choices ([more])
 * live in a menu behind the last segment, which then shows the one picked. [locked] (irreversible kinds) shows the
 * well with a lock and [lockedText] instead: nothing can be remembered.
 */
@Composable
fun <T> Chooser(
    choices: List<Choice<T>>,
    selected: T?,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    more: List<Choice<T>> = emptyList(),
    moreLabel: String = "Custom",
    moreTag: String = "chooserMore",
    locked: Boolean = false,
    lockedText: String = "Asked every time",
) {
    val c = LocalColors.current
    if (locked) {
        Row(
            modifier.fillMaxWidth().height(46.dp).clip(CircleShape).background(c.paperSunken).semantics(mergeDescendants = true) {}.testTag("chooserLocked"),
            horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphIcon(Glyph.Lock, c.ink2, size = 16.dp)
            RText(lockedText, RType.sans(14f, FontWeight.Medium), c.ink2, maxLines = 1)
        }
        return
    }
    val feedback = LocalFeedback.current
    var menu by remember { mutableStateOf(false) }
    val picked = more.firstOrNull { it.value == selected }
    Row(
        modifier.fillMaxWidth().clip(CircleShape).background(c.paperSunken).padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        choices.forEach { choice ->
            ChooserSegment(choice.label, choice.value == selected, Modifier.weight(1f).then(if (choice.tag.isNotEmpty()) Modifier.testTag(choice.tag) else Modifier)) {
                if (choice.value != selected) {
                    onSelect(choice.value)
                    feedback.play(Event.Selection)
                }
            }
        }
        if (more.isNotEmpty()) {
            Box(Modifier.weight(1f)) {
                ChooserSegment(picked?.label ?: moreLabel, picked != null, Modifier.fillMaxWidth().testTag(moreTag), chevron = true) {
                    menu = true
                    feedback.play(Event.Open)
                }
                SignalMenu(
                    expanded = menu,
                    onDismiss = { menu = false },
                    items = more.map { choice ->
                        MenuItem(choice.label, selected = choice.value == selected, tag = choice.tag.ifEmpty { null }) {
                            menu = false
                            onSelect(choice.value)
                        }
                    },
                    width = 200.dp,
                )
            }
        }
    }
}

@Composable
private fun ChooserSegment(label: String, on: Boolean, modifier: Modifier, chevron: Boolean = false, onClick: () -> Unit) {
    val c = LocalColors.current
    val bg by animateColorAsState(if (on) c.paperRaised else c.paperSunken.copy(alpha = 0f), Motion.fade(), label = "segment")
    Row(
        modifier
            .height(38.dp)
            .then(if (on) Modifier.shadow(1.dp, CircleShape, ambientColor = Color.Black.copy(alpha = 0.08f), spotColor = Color.Black.copy(alpha = 0.08f)) else Modifier)
            .clip(CircleShape)
            .background(bg)
            .then(if (on) Modifier.border(1.dp, c.rule, CircleShape) else Modifier)
            .pressable(shape = CircleShape, onClick = onClick)
            .semantics {
                role = Role.Tab
                selected = on
            }
            .padding(horizontal = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterHorizontally),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(label, RType.sans(14f, FontWeight.Medium), if (on) c.ink else c.ink2, maxLines = 1)
        if (chevron) GlyphIcon(Glyph.ChevronDown, if (on) c.ink else c.ink2, size = 14.dp)
    }
}

/** "Allow for       1 hour · at most 5 uses ▾": what was chosen, in a sunken pill that opens the choice. */
@Composable
fun ChooserSummary(label: String, value: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        modifier.fillMaxWidth().height(46.dp).clip(CircleShape).background(c.paperSunken).pressable(shape = CircleShape, label = label, onClick = onClick).padding(horizontal = 18.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(label, RType.sans(14f, FontWeight.Medium), c.ink2, Modifier.weight(1f), maxLines = 1)
        RText(value, RType.sans(14f, FontWeight.Medium), c.ink, maxLines = 1)
        Spacer(Modifier.width(4.dp))
        GlyphIcon(Glyph.ChevronDown, c.ink, size = 14.dp)
    }
}

// ---- Menus that grow from their trigger --------------------------------------------------------------------------

/** The tile of a menu row: plain, violet (the current Auto), warn (Lockdown), danger (Bypass). */
enum class MenuTint { Plain, Violet, Warn, Danger }

/** One row of a [SignalMenu]. [divider] puts a hairline above it. */
@Immutable
data class MenuItem(
    val title: String,
    val subtitle: String? = null,
    val glyph: Glyph? = null,
    val tint: MenuTint = MenuTint.Plain,
    val selected: Boolean = false,
    val destructive: Boolean = false,
    val chevron: Boolean = false,
    val divider: Boolean = false,
    val tag: String? = null,
    val onClick: () -> Unit,
)

/**
 * A menu that grows from its trigger (scale 0.92 and fade over 180 ms; closes in 120 ms), anchored under it at its
 * end ([alignEnd]) or start. A finger dragged down the menu feels a select tick on each row it crosses and picks the
 * row it lets go on. Menus replace system pickers.
 */
@Composable
fun SignalMenu(
    expanded: Boolean,
    onDismiss: () -> Unit,
    items: List<MenuItem>,
    modifier: Modifier = Modifier,
    alignEnd: Boolean = true,
    width: Dp = 290.dp,
) {
    val reduce = LocalReduceMotion.current
    val state = remember { MutableTransitionState(false) }
    state.targetState = expanded
    if (!state.currentState && !state.targetState && state.isIdle) return
    val gap = with(LocalDensity.current) { 8.dp.roundToPx() }
    val provider = remember(alignEnd, gap) { MenuPosition(alignEnd, gap) }
    val origin = TransformOrigin(if (alignEnd) 1f else 0f, 0f)
    Popup(popupPositionProvider = provider, onDismissRequest = onDismiss, properties = PopupProperties(focusable = true)) {
        AnimatedVisibility(state, enter = Motion.menuEnter(origin, reduce), exit = Motion.menuExit(origin, reduce)) {
            SignalMenuPanel(items, modifier, width = width, onPicked = onDismiss)
        }
    }
}

/** Under the anchor (or above it when there is no room), aligned to its end or start, kept on screen. */
private class MenuPosition(private val alignEnd: Boolean, private val gap: Int) : PopupPositionProvider {
    override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset {
        val end = alignEnd == (layoutDirection == LayoutDirection.Ltr)
        val x = (if (end) anchorBounds.right - popupContentSize.width else anchorBounds.left).coerceIn(0, (windowSize.width - popupContentSize.width).coerceAtLeast(0))
        val below = anchorBounds.bottom + gap
        val y = if (below + popupContentSize.height <= windowSize.height) below else (anchorBounds.top - gap - popupContentSize.height).coerceAtLeast(0)
        return IntOffset(x, y)
    }
}

/** The menu itself, on its raised plate: what [SignalMenu] shows, and what the gallery draws in place. */
@Composable
fun SignalMenuPanel(items: List<MenuItem>, modifier: Modifier = Modifier, width: Dp = 290.dp, onPicked: () -> Unit = {}) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val shape = RoundedCornerShape(20.dp)
    val bounds = remember(items.size) { mutableStateListOf<Rect>().apply { repeat(items.size) { add(Rect.Zero) } } }
    var hovered by remember { mutableIntStateOf(-1) }
    val latest by rememberUpdatedState(items)
    val picked by rememberUpdatedState(onPicked)
    Column(
        modifier
            .width(width)
            .shadow(24.dp, shape, ambientColor = Color(0x590A0A14), spotColor = Color(0x590A0A14))
            .clip(shape)
            .background(c.paperRaised)
            .border(1.dp, c.rule, shape)
            .pointerInput(Unit) {
                // The whole menu follows one finger: a tick on each row crossed, the row let go on is picked.
                awaitEachGesture {
                    val down = awaitFirstDown()
                    fun hit(p: Offset) = bounds.indexOfFirst { it.contains(p) }
                    var at = hit(down.position)
                    hovered = at
                    while (true) {
                        val event = awaitPointerEvent()
                        val change = event.changes.firstOrNull { it.id == down.id } ?: break
                        if (!change.pressed) break
                        val now = hit(change.position)
                        if (now != at) {
                            at = now
                            hovered = now
                            if (now >= 0) feedback.play(Event.MenuRow)
                        }
                        change.consume()
                    }
                    hovered = -1
                    latest.getOrNull(at)?.let {
                        feedback.play(Event.Selection)
                        it.onClick()
                        picked()
                    }
                }
            }
            .padding(6.dp),
    ) {
        items.forEachIndexed { i, item ->
            if (item.divider) Box(Modifier.padding(horizontal = 10.dp, vertical = 4.dp).fillMaxWidth().height(1.dp).background(c.rule))
            MenuRow(
                item,
                highlighted = item.selected || hovered == i,
                modifier = Modifier.onGloballyPositioned { bounds[i] = it.boundsInParent() },
            )
        }
    }
}

@Composable
private fun MenuRow(item: MenuItem, highlighted: Boolean, modifier: Modifier) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 52.dp)
            .clip(RoundedCornerShape(14.dp))
            .background(if (highlighted) c.paperSunken else Color.Transparent)
            .semantics(mergeDescendants = true) {
                role = Role.Button
                selected = item.selected
                onClick(label = item.title) {
                    item.onClick()
                    true
                }
            }
            .then(if (item.tag != null) Modifier.testTag(item.tag) else Modifier)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (item.glyph != null) {
            val (bg, fg) = when (item.tint) {
                MenuTint.Plain -> c.paperSunken to c.ink
                MenuTint.Violet -> c.violetFill to Color.White
                MenuTint.Warn -> c.kindWarn.copy(alpha = 0.14f) to c.kindWarn
                MenuTint.Danger -> c.dangerWash to c.danger
            }
            Box(Modifier.size(32.dp).clip(RoundedCornerShape(10.dp)).background(bg), contentAlignment = Alignment.Center) {
                GlyphIcon(item.glyph, fg, size = 18.dp)
            }
        }
        Column(Modifier.weight(1f)) {
            RText(item.title, RType.sans(15f, FontWeight.Medium), if (item.destructive) c.danger else c.ink, maxLines = 1)
            if (item.subtitle != null) RText(item.subtitle, RType.sans(12.5f), c.inkMeta, maxLines = 2)
        }
        when {
            item.selected -> GlyphIcon(Glyph.Check, c.violet, size = 18.dp)
            item.chevron -> GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 16.dp)
        }
    }
}

// ---- The mode chip ------------------------------------------------------------------------------------------------

/** What the mode chip in every header says: the same colour, glyph and name in every header (System board). */
enum class ChipMode(val label: String, val glyph: Glyph?) {
    Manual("Manual", Glyph.Person),
    Auto("Auto", Glyph.AutopilotFilled),
    Lockdown("Lockdown", Glyph.Block),
    Bypass("Bypass", Glyph.Bypass),

    /** Overnight: night blue with the moon. */
    Overnight("Overnight", Glyph.Moon),
}

/** A running mode's field: Auto violet, Overnight night, Lockdown amber, Bypass red. Manual has none (paper). */
fun ChipMode.field(): Field? = when (this) {
    ChipMode.Manual -> null
    ChipMode.Auto -> Field.Grant
    ChipMode.Overnight -> Field.Night
    ChipMode.Lockdown -> Field.Warn
    ChipMode.Bypass -> Field.Delete
}

/**
 * The mode chip (36 dp, 14 / 600): Manual on a raised plate with a hairline, the others on their field colour with
 * white ink; a running clock follows the name in mono ("Bypass 47:12"). [chevron] when it opens the mode sheet.
 */
@Composable
fun ModeChip(mode: ChipMode, modifier: Modifier = Modifier, timer: String? = null, chevron: Boolean = false, onClick: (() -> Unit)? = null) {
    val c = LocalColors.current
    val field = mode.field()
    val bg = field?.fill() ?: c.paperRaised
    val fg = if (field != null) Color.White else c.ink
    Row(
        modifier
            .height(36.dp)
            .clip(CircleShape)
            .background(bg)
            .then(if (field == null) Modifier.border(1.dp, c.rule, CircleShape) else Modifier)
            .then(if (onClick != null) Modifier.pressable(shape = CircleShape, label = mode.label, onClick = onClick) else Modifier)
            .padding(horizontal = 13.dp)
            .testTag("modeChip:${mode.name}"),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        mode.glyph?.let { GlyphIcon(it, fg, size = 16.dp) }
        RText(mode.label, RType.sans(14f, FontWeight.SemiBold), fg, maxLines = 1)
        if (timer != null) RText(timer, RType.mono(13.5f, FontWeight.Medium), fg.copy(alpha = 0.85f), maxLines = 1, ltr = true)
        if (chevron) GlyphIcon(Glyph.ChevronDown, fg, size = 15.dp)
    }
}

/**
 * The bypass dot: breathes 35 % to 100 % and back over 1.8 s while any bypass runs. Still under Reduce Motion and in
 * screenshots. The alpha is read while drawing, so a breath redraws the dot and nothing else.
 */
@Composable
fun BreathingDot(color: Color, modifier: Modifier = Modifier, size: Dp = 8.dp) {
    val still = LocalReduceMotion.current || !LocalLiveTimers.current
    if (still) {
        Box(modifier.size(size).background(color, CircleShape))
        return
    }
    val breath = rememberInfiniteTransition(label = "breathe")
    val alpha = breath.animateFloat(
        0.35f, 1f,
        infiniteRepeatable(tween(Motion.BREATHE_MS / 2, easing = Motion.InOut), RepeatMode.Reverse),
        label = "breatheAlpha",
    )
    Box(modifier.size(size).graphicsLayer { this.alpha = alpha.value }.background(color, CircleShape))
}

// ---- Waiting cards ------------------------------------------------------------------------------------------------

/**
 * A waiting request as a card: its field (with the grain), the label line, the timer, the ask in the serif with the
 * target in italic, and its buttons, Autopilot's pick first ([AutopilotPick]).
 */
@Composable
fun WaitingCard(
    field: Field,
    label: String,
    ask: String,
    modifier: Modifier = Modifier,
    target: String? = null,
    avatar: (@Composable () -> Unit)? = null,
    timer: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit,
) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(26.dp)
    Column(
        modifier
            .fillMaxWidth()
            .fieldSurface(field, shape)
            .clip(shape)
            .then(if (onClick != null) Modifier.pressable(shape = shape, label = ask, onClick = onClick) else Modifier)
            .padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            avatar?.invoke()
            LabelText(label, c.onField2, Modifier.weight(1f))
            timer?.invoke()
        }
        androidx.compose.foundation.text.BasicText(
            askString(ask, target),
            style = RType.askSmall.copy(color = c.onField),
            maxLines = 3,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), content = actions)
    }
}

/** A brand mark on the white 24 dp disc a field shows it on (62 % of the disc). */
@Composable
fun FieldAvatar(provider: Provider, modifier: Modifier = Modifier, size: Dp = 24.dp) {
    val context = LocalContext.current
    val mark = size * 0.62f
    val px = with(LocalDensity.current) { mark.roundToPx() }
    val bitmap = remember(provider.key, px) { SvgAssets.bitmap(context, provider.asset, px)?.asImageBitmap() }
    Box(modifier.size(size).background(Color.White, CircleShape), contentAlignment = Alignment.Center) {
        if (bitmap != null) {
            Image(bitmap, provider.name, Modifier.size(mark), colorFilter = if (provider.mono) ColorFilter.tint(Color(0xFF16151B)) else null)
        }
    }
}

/** Autopilot's pick, the first button on a waiting card. */
sealed interface Pick {
    /** It would allow (or deny), this sure. */
    data class Sure(val allow: Boolean, val percent: Int) : Pick

    /** It is not sure: a half ring and "Unsure". */
    data object Unsure : Pick

    /** An always-ask kind: a lock and "Always asks". */
    data object AlwaysAsks : Pick
}

/** [pick] as a small button on a field: a sure Allow is white with the fingerprint; the rest are the wash. */
@Composable
fun AutopilotPick(pick: Pick, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    when (pick) {
        is Pick.Sure -> SignalButton(
            "${if (pick.allow) "Allow" else "Deny"} · ${pick.percent}%",
            modifier,
            style = if (pick.allow) ButtonStyle.OnFieldWhite else ButtonStyle.OnFieldWash,
            size = ButtonSize.Small,
            glyph = if (pick.allow) Glyph.Fingerprint else null,
            onClick = onClick,
        )
        Pick.Unsure -> SignalButton(
            "Unsure",
            modifier,
            style = ButtonStyle.OnFieldWash,
            size = ButtonSize.Small,
            leading = { Ring(0.5f, c.onField, c.onField.copy(alpha = 0.35f), 16.dp) },
            onClick = onClick,
        )
        Pick.AlwaysAsks -> SignalButton("Always asks", modifier, style = ButtonStyle.OnFieldWash, size = ButtonSize.Small, glyph = Glyph.Lock, onClick = onClick)
    }
}

/** A thin ring from twelve o'clock: [fraction] in [color] over [track] (the boards' 3/36 stroke). */
@Composable
fun Ring(fraction: Float, color: Color, track: Color, size: Dp, modifier: Modifier = Modifier) {
    Canvas(modifier.size(size)) {
        val stroke = this.size.width * 3f / 36f
        val inset = this.size.width * 4f / 36f
        val arc = Size(this.size.width - inset * 2, this.size.height - inset * 2)
        drawArc(track, 0f, 360f, false, Offset(inset, inset), arc, style = Stroke(stroke))
        val f = fraction.coerceIn(0f, 1f)
        if (f > 0f) drawArc(color, -90f, 360f * f, false, Offset(inset, inset), arc, style = Stroke(stroke, cap = StrokeCap.Round))
    }
}

/**
 * The timer on a field: a ring that drains each second beside the time left; under [URGENT_SECONDS] (15 s) a white
 * pill with red digits.
 */
@Composable
fun TimerPill(secondsLeft: Long, totalSeconds: Long, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val left = secondsLeft.coerceAtLeast(0)
    val text = "${left / 60}:${(left % 60).toString().padStart(2, '0')}"
    if (left < URGENT_SECONDS) {
        Box(
            modifier.height(26.dp).background(Color.White, CircleShape).padding(horizontal = 10.dp).semantics(mergeDescendants = true) {}.testTag("timer:urgent"),
            contentAlignment = Alignment.Center,
        ) { RText(text, RType.mono(13f, FontWeight.Medium), c.fieldDelete, maxLines = 1, ltr = true) }
    } else {
        Row(modifier.testTag("timer"), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
            Ring(if (totalSeconds > 0) left.toFloat() / totalSeconds else 0f, c.onField, c.onField.copy(alpha = 0.25f), 18.dp)
            RText(text, RType.mono(13f), c.onField, maxLines = 1, ltr = true)
        }
    }
}

// ---- Rows, tiles, pills -------------------------------------------------------------------------------------------

/** A kind tile: the kind's glyph in its ink on its 13 % tile (38 dp, radius 11; 46 dp, radius 14). */
@Composable
fun KindTile(field: Field, glyph: Glyph, modifier: Modifier = Modifier, size: Dp = 38.dp) {
    val c = LocalColors.current
    Box(
        modifier.size(size).clip(RoundedCornerShape(if (size >= 46.dp) 14.dp else 11.dp)).background(field.tile(c)),
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(glyph, field.kindInk(c), size = size * 0.53f) }
}

/** A hairline under a row: the rule, 1 dp, drawn at the bottom edge. */
fun Modifier.hairlineBelow(color: Color, inset: Dp = 0.dp): Modifier = drawBehind {
    val y = size.height - 0.5.dp.toPx()
    drawLine(color, Offset(inset.toPx(), y), Offset(size.width, y), strokeWidth = 1.dp.toPx())
}

/**
 * A row of a list: hairlines, never cards. Optional [leading] (a [KindTile]), the title (row 15.5 / 500) over its
 * meta, and [trailing] (a status word, a value, a switch). [onClick] makes it pressable.
 */
@Composable
fun HairlineRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    leading: (@Composable () -> Unit)? = null,
    trailing: (@Composable () -> Unit)? = null,
    divider: Boolean = true,
    onClick: (() -> Unit)? = null,
) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .then(if (divider) Modifier.hairlineBelow(c.rule) else Modifier)
            .then(if (onClick != null) Modifier.pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .semantics(mergeDescendants = true) {}
            .padding(vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        leading?.invoke()
        Column(Modifier.weight(1f)) {
            RText(title, RType.row, c.ink, maxLines = 2)
            if (!subtitle.isNullOrEmpty()) RText(subtitle, RType.meta, c.inkMeta, maxLines = 3)
        }
        trailing?.invoke()
    }
}

/** A fact under a field: the key in ink-2 on the left, [value] on the right ("Branch   feature/passkeys"). */
@Composable
fun KeyValueRow(
    key: String,
    modifier: Modifier = Modifier,
    divider: Boolean = true,
    chevron: Boolean = false,
    onClick: (() -> Unit)? = null,
    value: @Composable RowScope.() -> Unit,
) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .then(if (divider) Modifier.hairlineBelow(c.rule) else Modifier)
            .then(if (onClick != null) Modifier.pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .semantics(mergeDescendants = true) {}
            .padding(vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(key, RType.sans(15f), c.ink2, Modifier.weight(1f), maxLines = 1)
        value()
        if (chevron) GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 14.dp)
    }
}

/** A tag that always carries a word ("New", "Auto", "Ready"): [color] on its 13 % wash, 11.5 / 600. */
@Composable
fun Pill(text: String, color: Color, modifier: Modifier = Modifier, glyph: Glyph? = null) {
    Row(
        modifier.clip(CircleShape).background(color.copy(alpha = Field.TILE_ALPHA)).semantics(mergeDescendants = true) {}.padding(horizontal = 8.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        glyph?.let { GlyphIcon(it, color, size = 12.dp) }
        RText(text, RType.tag, color, maxLines = 1)
    }
}

/** An outcome at the end of a row: a word in its colour, 12 / 600 ("Auto", "Denied"). */
@Composable
fun StatusWord(text: String, color: Color, modifier: Modifier = Modifier) {
    RText(text, RType.sans(12f, FontWeight.SemiBold), color, modifier, maxLines = 1)
}

/** A filter chip: raised with a hairline; the chosen one is ink turned into a plate. */
@Composable
fun SignalChip(text: String, selected: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Box(
        modifier
            .height(34.dp)
            .clip(CircleShape)
            .background(if (selected) c.inverse else c.paperRaised)
            .then(if (selected) Modifier else Modifier.border(1.dp, c.rule, CircleShape))
            .pressable(shape = CircleShape) {
                onClick()
                feedback.play(Event.Selection)
            }
            .semantics { this.selected = selected }
            .padding(horizontal = 13.dp),
        contentAlignment = Alignment.Center,
    ) { RText(text, RType.sans(14f, FontWeight.Medium), if (selected) c.onInverse else c.ink, maxLines = 1) }
}

/** A square tick box: violet-fill with a white tick, or a faint ring. */
@Composable
fun SignalCheckbox(checked: Boolean, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(8.dp)
    Box(
        modifier.size(24.dp).clip(shape).background(if (checked) c.violetFill else Color.Transparent).then(if (checked) Modifier else Modifier.border(1.5.dp, c.inkFaint, shape)),
        contentAlignment = Alignment.Center,
    ) { if (checked) GlyphIcon(Glyph.Check, Color.White, size = 16.dp, weight = 2.2f) }
}

/** A radio: a thick violet-fill ring when chosen, a faint ring when not. */
@Composable
fun SignalRadio(selected: Boolean, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Box(modifier.size(24.dp).border(if (selected) 7.dp else 1.5.dp, if (selected) c.violetFill else c.inkFaint, CircleShape))
}

/** − n +: a sunken pill with two round raised buttons; each step is a detent. */
@Composable
fun Stepper(value: Int, onChange: (Int) -> Unit, modifier: Modifier = Modifier, range: IntRange = 0..99) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        modifier.height(40.dp).clip(CircleShape).background(c.paperSunken).padding(horizontal = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StepButton(Glyph.Minus, "Less", value > range.first) {
            onChange(value - 1)
            feedback.play(Event.Detent, value - 1)
        }
        RollingNumber(value.toString(), RType.mono(15f), c.ink)
        StepButton(Glyph.Plus, "More", value < range.last) {
            onChange(value + 1)
            feedback.play(Event.Detent, value + 1)
        }
    }
}

@Composable
private fun StepButton(glyph: Glyph, label: String, enabled: Boolean, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier
            .size(30.dp)
            .graphicsLayer { alpha = if (enabled) 1f else DISABLED_ALPHA }
            .clip(CircleShape)
            .background(c.paperRaised)
            .pressable(enabled = enabled, shape = CircleShape, label = label, onClick = onClick)
            .semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) { GlyphIcon(glyph, c.ink, size = 16.dp) }
}

/**
 * A number that rolls digit by digit when it changes (240 ms): each changed character slides up (or down, when the
 * number falls); Reduce Motion cross-fades.
 */
@Composable
fun RollingNumber(text: String, style: androidx.compose.ui.text.TextStyle, color: Color, modifier: Modifier = Modifier) {
    val reduce = LocalReduceMotion.current
    var previous by remember { mutableStateOf(text) }
    val rising = remember(text) { (text.toLongOrNull() ?: 0) >= (previous.toLongOrNull() ?: 0) }
    androidx.compose.runtime.SideEffect { previous = text }
    Row(modifier.semantics(mergeDescendants = true) { contentDescription = text }) {
        // Keyed from the right, so units stay units when the number gains a digit.
        val chars = text.reversed().toList()
        for (i in chars.indices.reversed()) {
            androidx.compose.runtime.key(i) {
                AnimatedContent(
                    targetState = chars[i],
                    transitionSpec = {
                        if (reduce) {
                            fadeIn(tween(Motion.REDUCED_MS)) togetherWith fadeOut(tween(Motion.REDUCED_MS))
                        } else {
                            val dir = if (rising) 1 else -1
                            (slideInVertically(tween(Motion.NUMBER_ROLL_MS, easing = Motion.EaseOut)) { it * dir } + fadeIn(tween(Motion.NUMBER_ROLL_MS))) togetherWith
                                (slideOutVertically(tween(Motion.NUMBER_ROLL_MS, easing = Motion.EaseOut)) { -it * dir } + fadeOut(tween(Motion.NUMBER_ROLL_MS)))
                        }
                    },
                    label = "digit",
                ) { ch -> RText(ch.toString(), style, color, maxLines = 1) }
            }
        }
    }
}

// ---- Field header ---------------------------------------------------------------------------------------------------

/**
 * A field at the top of a screen: edge to edge, behind the status bar (its contents start below the system's inset,
 * never a hard-coded height), only its bottom corners round (30). The grain comes with the field.
 */
@Composable
fun FieldHeader(field: Field, modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .fieldSurface(field, FieldHeaderShape)
            // Never shorter than the boards' 36 dp status bar (a renderer without system bars reports none).
            .windowInsetsPadding(WindowInsets.statusBars.union(WindowInsets(top = 36.dp)))
            .padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        content = content,
    )
}

/** The field's shape under the status bar: square on top, 30 dp at the bottom. */
val FieldHeaderShape = RoundedCornerShape(bottomStart = 30.dp, bottomEnd = 30.dp)

// ---- The docked tab bar (Android) ---------------------------------------------------------------------------------

/**
 * One tab: its word, its glyph, its test tag, a count on its glyph (tagged `<tag>:count`), and [dot]: the breathing
 * bypass dot while any bypass runs.
 */
@Immutable
data class TabItem(val label: String, val glyph: Glyph, val tag: String = "", val badge: Int = 0, val dot: Boolean = false)

/**
 * Android's tab bar, the Material 3 navigation bar in Next's colours: docked on paper-raised with a hairline on top,
 * labels always shown, a violet 18 % pill behind the chosen tab's glyph that slides there (240 ms, a slight overshoot; a cross-fade under Reduce Motion). Choosing a tab
 * is felt (select).
 */
@Composable
fun DockedTabBar(tabs: List<TabItem>, selected: Int, onSelect: (Int) -> Unit, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val reduce = LocalReduceMotion.current
    val pillAt by animateFloatAsState(selected.toFloat(), Motion.tabPill(reduce), label = "tabPill")
    Column(
        modifier
            .fillMaxWidth()
            .background(c.navBar)
            .drawBehind { drawLine(c.rule, Offset(0f, 0.5.dp.toPx()), Offset(size.width, 0.5.dp.toPx()), strokeWidth = 1.dp.toPx()) }
            .windowInsetsPadding(WindowInsets.navigationBars),
    ) {
        BoxWithConstraints(Modifier.fillMaxWidth().padding(vertical = 10.dp)) {
            val tabWidth = maxWidth / tabs.size.coerceAtLeast(1)
            if (!reduce) {
                // One pill, sliding.
                Box(
                    Modifier
                        .offset { IntOffset(((tabWidth * pillAt) + (tabWidth - PILL_W) / 2).roundToPx(), 0) }
                        .size(PILL_W, PILL_H)
                        .background(c.navIndicator, RoundedCornerShape(16.dp)),
                )
            }
            Row(Modifier.fillMaxWidth()) {
                tabs.forEachIndexed { i, tab ->
                    val on = i == selected
                    Column(
                        Modifier
                            .weight(1f)
                            .pressable(shape = RoundedCornerShape(16.dp), label = tab.label) {
                                if (i != selected) {
                                    onSelect(i)
                                    feedback.play(Event.TabSelect)
                                }
                            }
                            .semantics {
                                role = Role.Tab
                                this.selected = on
                            }
                            .then(if (tab.tag.isNotEmpty()) Modifier.testTag(tab.tag) else Modifier),
                        horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Box(Modifier.size(PILL_W, PILL_H), contentAlignment = Alignment.Center) {
                            if (reduce) {
                                val a by animateFloatAsState(if (on) 1f else 0f, Motion.reduced(), label = "tabPillFade")
                                Box(Modifier.size(PILL_W, PILL_H).graphicsLayer { alpha = a }.background(c.navIndicator, RoundedCornerShape(16.dp)))
                            }
                            GlyphIcon(tab.glyph, if (on) c.ink else c.ink2, size = 20.dp)
                            if (tab.badge > 0) {
                                TabBadge(tab.badge, Modifier.align(Alignment.TopStart).offset(x = 34.dp, y = (-2).dp).then(if (tab.tag.isNotEmpty()) Modifier.testTag("${tab.tag}:count") else Modifier))
                            } else if (tab.dot) {
                                BreathingDot(c.fieldDelete, Modifier.align(Alignment.TopEnd).offset(x = (-14).dp, y = 3.dp).testTag("${tab.tag}:dot"))
                            }
                        }
                        RText(
                            tab.label,
                            RType.sans(12f, FontWeight.SemiBold),
                            if (on) c.ink else c.ink2,
                            maxLines = 1,
                            align = TextAlign.Center,
                        )
                    }
                }
            }
        }
    }
}

private val PILL_W = 60.dp
private val PILL_H = 32.dp

/** The count on a tab: white on the delete red, 16 dp tall and at least as wide (10 / 600). */
@Composable
private fun TabBadge(count: Int, modifier: Modifier = Modifier) {
    Box(
        modifier.defaultMinSize(minWidth = 16.dp, minHeight = 16.dp).background(Field.Delete.fill(), CircleShape).padding(horizontal = 4.dp),
        contentAlignment = Alignment.Center,
    ) { RText(countText(count), RType.sans(10f, FontWeight.SemiBold), Color.White, maxLines = 1) }
}

// ---- Dialog -------------------------------------------------------------------------------------------------------

/**
 * A question in the app's own chrome: a sheet-title in the serif (with [italic] set in italic), one line of body,
 * Cancel (outline) and the action (inverse, or the danger wash when [destructive]).
 */
@Composable
fun SignalDialog(
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    destructive: Boolean = false,
    italic: String? = null,
    cancelLabel: String = "Cancel",
    /** The answer button's test tag. */
    confirmTag: String? = null,
    /** A glyph on a small tile above the title (And-Outside 05). */
    glyph: Glyph? = null,
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        SignalDialogPanel(title, text, confirmLabel, onConfirm, onDismiss, Modifier.padding(horizontal = 24.dp), destructive, italic, cancelLabel, confirmTag, glyph)
    }
}

/** What [SignalDialog] shows: also drawn in place by the gallery. */
@Composable
fun SignalDialogPanel(
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
    destructive: Boolean = false,
    italic: String? = null,
    cancelLabel: String = "Cancel",
    confirmTag: String? = null,
    glyph: Glyph? = null,
) {
    val c = LocalColors.current
    val shape = RoundedCornerShape(28.dp)
    Column(
        modifier.fillMaxWidth().shadow(24.dp, shape, ambientColor = Color(0x590A0A14), spotColor = Color(0x590A0A14)).clip(shape).background(c.paperRaised).border(1.dp, c.rule, shape).padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (glyph != null) {
            Box(Modifier.size(38.dp).clip(RoundedCornerShape(11.dp)).background(c.paperSunken), contentAlignment = Alignment.Center) {
                GlyphIcon(glyph, c.ink2, size = 18.dp)
            }
        }
        androidx.compose.foundation.text.BasicText(askString(title, italic), style = RType.serif(28f, 30f, -0.01f).copy(color = c.ink))
        RText(text, RType.body, c.ink2)
        Spacer(Modifier.height(8.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            SignalButton(cancelLabel, Modifier.weight(1f), style = ButtonStyle.Outline, onClick = onDismiss)
            SignalButton(
                confirmLabel,
                Modifier.weight(1f).then(if (confirmTag != null) Modifier.testTag(confirmTag) else Modifier),
                style = if (destructive) ButtonStyle.DangerWash else ButtonStyle.Inverse,
                onClick = onConfirm,
            )
        }
    }
}
