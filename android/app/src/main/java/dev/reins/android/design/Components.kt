package dev.reins.android.design

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.IndicationNodeFactory
import androidx.compose.foundation.clickable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.InteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.getValue
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.geometry.toRect
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawOutline
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.node.CompositionLocalConsumerModifierNode
import androidx.compose.ui.node.DelegatableNode
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.node.currentValueOf
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import dev.reins.android.feedback.DialogFeedback
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.Feedback
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.defaultTap
import dev.reins.android.feedback.play
import kotlinx.coroutines.launch

/**
 * Press feedback without Material ripples: the element dims (or a fill appears behind it) while the finger is down, and
 * the click answers with the default tap unless [onClick] asked for feedback of its own.
 *
 * Built from modifier nodes, not a composition of its own: a list has one per row, and a row scrolling into view should
 * cost no more than its layout.
 */
fun Modifier.pressable(
    enabled: Boolean = true,
    highlight: Color? = null,
    shape: Shape = RoundedCornerShape(16.dp),
    dim: Float = 0.55f,
    label: String? = null,
    onClick: () -> Unit,
): Modifier {
    val click = PressClick(onClick)
    return this
        .then(PressClickElement(click))
        .clickable(
            interactionSource = null,
            indication = PressIndication(highlight, shape, dim),
            enabled = enabled,
            onClickLabel = label,
            role = Role.Button,
            onClick = click,
        )
}

/**
 * A [pressable]'s click: the caller's action, then the default tap from the [LocalFeedback] of the node it is wired to
 * ([PressClickElement]). Equal when the action is, so an unchanged row updates nothing.
 */
private class PressClick(val onClick: () -> Unit) : () -> Unit {
    var node: PressClickNode? = null

    override fun invoke() {
        onClick()
        node?.feedback()?.defaultTap()
    }

    override fun equals(other: Any?) = other is PressClick && other.onClick == onClick

    override fun hashCode() = onClick.hashCode()
}

private data class PressClickElement(val click: PressClick) : ModifierNodeElement<PressClickNode>() {
    override fun create() = PressClickNode(click)

    override fun update(node: PressClickNode) {
        node.click = click
        click.node = node
    }
}

private class PressClickNode(var click: PressClick) : Modifier.Node(), CompositionLocalConsumerModifierNode {
    fun feedback(): Feedback? = if (isAttached) currentValueOf(LocalFeedback) else null

    override fun onAttach() {
        click.node = this
    }
}

/** The look of a press: the content dims, or [highlight] fills [shape] behind it. */
private data class PressIndication(val highlight: Color?, val shape: Shape, val dim: Float) : IndicationNodeFactory {
    override fun create(interactionSource: InteractionSource): DelegatableNode = PressIndicationNode(interactionSource, highlight, shape, dim)
}

private class PressIndicationNode(
    private val source: InteractionSource,
    private val highlight: Color?,
    private val shape: Shape,
    private val dim: Float,
) : Modifier.Node(), DrawModifierNode {
    /** 0 at rest, 1 while pressed; read only while drawing, so a press redraws and recomposes nothing. */
    private val pressed = Animatable(0f)
    private val layer = Paint()

    override fun onAttach() {
        coroutineScope.launch {
            val presses = mutableListOf<PressInteraction.Press>()
            source.interactions.collect { interaction ->
                when (interaction) {
                    is PressInteraction.Press -> presses += interaction
                    is PressInteraction.Release -> presses -= interaction.press
                    is PressInteraction.Cancel -> presses -= interaction.press
                }
                val target = if (presses.isEmpty()) 0f else 1f
                if (pressed.targetValue != target) launch { pressed.animateTo(target, spring()) }
            }
        }
    }

    override fun ContentDrawScope.draw() {
        val p = pressed.value
        when {
            p == 0f -> drawContent()
            highlight != null -> {
                drawOutline(shape.createOutline(size, layoutDirection, this), lerp(Color.Transparent, highlight, p))
                drawContent()
            }
            else -> {
                layer.alpha = 1f - (1f - dim) * p
                drawContext.canvas.saveLayer(size.toRect(), layer)
                drawContent()
                drawContext.canvas.restore()
            }
        }
    }
}

/** Floating chrome surface (dialogs, toasts). */
fun Modifier.glass(colors: RColors, shape: Shape, elevation: Dp = 10.dp): Modifier = this
    .shadow(
        elevation,
        shape,
        clip = false,
        ambientColor = Color.Black.copy(alpha = 0.25f),
        spotColor = Color.Black.copy(alpha = if (colors.dark) 0.5f else 0.18f),
    )
    .clip(shape)
    .background(colors.glass, shape)
    .border(0.75.dp, colors.glassEdge, shape)

enum class ButtonStyle { Primary, Accent, Secondary, Ghost, Destructive }

@Composable
fun CapsuleButton(
    title: String,
    modifier: Modifier = Modifier,
    style: ButtonStyle = ButtonStyle.Primary,
    enabled: Boolean = true,
    busy: Boolean = false,
    glyph: Glyph? = null,
    compact: Boolean = false,
    /** A long label wraps onto this many lines instead of being cut off. */
    maxLines: Int = 1,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    val (bg, fg) = when (style) {
        ButtonStyle.Primary -> c.text to c.background
        ButtonStyle.Accent -> c.accent to Color.White
        ButtonStyle.Secondary -> c.controlFill to c.text
        ButtonStyle.Ghost -> Color.Transparent to c.secondary
        ButtonStyle.Destructive -> c.danger.copy(alpha = 0.12f) to c.danger
    }
    Row(
        modifier
            .defaultMinSize(minHeight = if (compact) 34.dp else 50.dp)
            .graphicsLayer { alpha = if (enabled) 1f else 0.4f }
            .clip(CircleShape)
            .background(bg)
            .pressable(enabled = enabled && !busy, shape = CircleShape, onClick = onClick)
            .padding(horizontal = if (compact) 14.dp else 20.dp, vertical = if (compact) 7.dp else 14.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) {
            Spinner(fg, size = 15.dp)
            Spacer(Modifier.width(8.dp))
        } else if (glyph != null) {
            GlyphIcon(glyph, fg, size = if (compact) 15.dp else 18.dp)
            Spacer(Modifier.width(7.dp))
        }
        RText(title, RType.sans(if (compact) 14f else 16.5f, FontWeight.SemiBold), fg, Modifier.weight(1f, fill = false), maxLines = maxLines, align = if (maxLines > 1) TextAlign.Center else null)
    }
}

/** Round icon button on a glass plate (back, toolbar actions). */
@Composable
fun CircleIconButton(
    glyph: Glyph,
    modifier: Modifier = Modifier,
    size: Dp = 40.dp,
    iconSize: Dp = 18.dp,
    tint: Color? = null,
    label: String? = null,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    Box(
        modifier
            .size(size)
            .glass(c, CircleShape, 6.dp)
            .pressable(shape = CircleShape, label = label, onClick = onClick)
            .semantics { if (label != null) contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        GlyphIcon(glyph, tint ?: c.text, size = iconSize)
    }
}

/** A capsule choice (lifetime, filters): the selected one is lifted out with a hairline and a brighter fill. */
@Composable
fun SelectChip(title: String, selected: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        modifier
            .height(38.dp)
            .clip(CircleShape)
            .background(if (selected) c.accentSoft else c.controlFill)
            .border(0.75.dp, if (selected) c.accent.copy(alpha = 0.6f) else Color.Transparent, CircleShape)
            .pressable(shape = CircleShape) {
                onClick()
                feedback.play(Event.Selection)
            }
            .padding(horizontal = 15.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sans(14.5f, FontWeight.Medium), if (selected) c.accent else c.text, maxLines = 1)
    }
}

/** Small tinted label. */
@Composable
fun Tag(title: String, modifier: Modifier = Modifier, tint: Color? = null, mono: Boolean = false) {
    val c = LocalColors.current
    Row(
        modifier
            .height(26.dp)
            .clip(CircleShape)
            .background(tint?.copy(alpha = 0.1f) ?: c.controlFill)
            .padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(
            title,
            if (mono) RType.mono(12f, FontWeight.Medium) else RType.sans(12.5f, FontWeight.Medium),
            tint?.copy(alpha = 0.9f) ?: c.secondary,
            maxLines = 1,
        )
    }
}

/** Screen header: back button, centred two-line title, trailing actions. */
@Composable
fun TopBar(
    title: String,
    subtitle: String? = null,
    onBack: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val c = LocalColors.current
    Box(
        modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .height(56.dp)
            .padding(horizontal = 10.dp),
    ) {
        if (onBack != null) {
            CircleIconButton(Glyph.ChevronLeft, Modifier.align(Alignment.CenterStart), label = "Back", onClick = onBack)
        }
        Column(Modifier.align(Alignment.Center).padding(horizontal = 64.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            RText(title, RType.sans(16f, FontWeight.SemiBold), c.text, maxLines = 1)
            if (!subtitle.isNullOrEmpty()) RText(subtitle, RType.sans(12f), c.secondary, maxLines = 1)
        }
        Row(Modifier.align(Alignment.CenterEnd), horizontalArrangement = Arrangement.spacedBy(8.dp), content = actions)
    }
}

/** Root-screen header with a large title. */
@Composable
fun LargeTitle(title: String, modifier: Modifier = Modifier, actions: @Composable RowScope.() -> Unit = {}) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.statusBars)
            .padding(start = 20.dp, end = 12.dp, top = 22.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(title, RType.sans(30f, FontWeight.SemiBold), c.text, Modifier.weight(1f), maxLines = 1)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), content = actions)
    }
}

/** The standard screen: background, header, scrolling content that clears the system bars and the keyboard. */
@Composable
fun Screen(
    title: String?,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    onBack: (() -> Unit)? = null,
    actions: @Composable RowScope.() -> Unit = {},
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    Column(modifier.fillMaxSize().background(c.background).imePadding()) {
        if (title != null) {
            if (onBack == null) LargeTitle(title, actions = actions) else TopBar(title, subtitle, onBack, actions = actions)
        } else {
            Spacer(Modifier.windowInsetsPadding(WindowInsets.statusBars))
        }
        Column(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .windowInsetsPadding(WindowInsets.navigationBars),
            content = content,
        )
    }
}

/** A switch in the accent colour. It sounds after the change lands, so a switch that governs sound is heard in its new state. */
@Composable
fun Toggle(on: Boolean, modifier: Modifier = Modifier, enabled: Boolean = true, onChange: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val x by animateFloatAsState(if (on) 1f else 0f, spring(dampingRatio = 0.8f, stiffness = 700f), label = "toggle")
    val track by animateColorAsState(if (on) c.accent else c.controlFill.copy(alpha = 0.16f), label = "track")
    Box(
        modifier
            .size(width = 50.dp, height = 30.dp)
            .graphicsLayer { alpha = if (enabled) 1f else 0.4f }
            .clip(CircleShape)
            .background(track)
            .pressable(enabled = enabled, shape = CircleShape) {
                onChange(!on)
                feedback.play(Event.toggle(!on))
            },
    ) {
        Box(Modifier.padding(2.dp).offset(x = (20 * x).dp).size(26.dp).shadow(2.dp, CircleShape).background(Color.White, CircleShape))
    }
}

/** A settings row with a switch: the whole row toggles, and answers like the switch does. [tag] goes on the row. */
@Composable
fun SwitchRow(
    title: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    glyph: Glyph? = null,
    enabled: Boolean = true,
    tag: String? = null,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        modifier
            .fillMaxWidth()
            .then(if (tag != null) Modifier.testTag(tag) else Modifier)
            .toggleable(value = checked, enabled = enabled, role = Role.Switch) {
                onChange(it)
                feedback.play(Event.toggle(it))
            }
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // Greyed out like the switch when a switch above turns this one off.
        val dim = Modifier.graphicsLayer { alpha = if (enabled) 1f else 0.45f }
        if (glyph != null) {
            GlyphIcon(glyph, c.secondary, size = 21.dp, modifier = dim)
            Spacer(Modifier.width(14.dp))
        }
        Column(dim.weight(1f)) {
            RText(title, RType.sans(16f, FontWeight.Medium), c.text, maxLines = 2)
            if (!subtitle.isNullOrEmpty()) RText(subtitle, RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 4)
        }
        Spacer(Modifier.width(10.dp))
        Toggle(checked, enabled = enabled, onChange = onChange)
    }
}

/** A round tick box, used to pick messages. The whole row around it is the touch target (see [CheckRow]). */
@Composable
fun CheckMark(checked: Boolean, enabled: Boolean = true, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val fill by animateColorAsState(if (checked) c.accent else Color.Transparent, label = "checkFill")
    Box(
        modifier
            .size(24.dp)
            .graphicsLayer { alpha = if (enabled) 1f else 0.45f }
            .clip(CircleShape)
            .background(fill)
            .border(1.5.dp, if (checked) c.accent else c.tertiary, CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        if (checked) GlyphIcon(Glyph.Check, Color.White, size = 14.dp, weight = 2.4f)
    }
}

/** A whole-row tick target with a label. [tag] is the test tag of the row. */
@Composable
fun CheckRow(
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    tag: String? = null,
    content: @Composable RowScope.() -> Unit,
) {
    val feedback = LocalFeedback.current
    Row(
        modifier
            .then(if (tag != null) Modifier.testTag(tag) else Modifier)
            .toggleable(value = checked, enabled = enabled, role = Role.Checkbox) {
                onChange(it)
                feedback.play(Event.toggle(it))
            },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CheckMark(checked, enabled)
        Spacer(Modifier.width(12.dp))
        content()
    }
}

/** A settings-style row inside a grouped card. */
@Composable
fun ListRow(
    title: String,
    modifier: Modifier = Modifier,
    subtitle: String? = null,
    glyph: Glyph? = null,
    tint: Color? = null,
    destructive: Boolean = false,
    chevron: Boolean = false,
    ltrSubtitle: Boolean = false,
    trailing: (@Composable () -> Unit)? = null,
    onClick: (() -> Unit)? = null,
) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 52.dp)
            .then(if (onClick != null) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (glyph != null) {
            GlyphIcon(glyph, if (destructive) c.danger else tint ?: c.secondary, size = 21.dp)
            Spacer(Modifier.width(14.dp))
        }
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(16f, FontWeight.Medium), if (destructive) c.danger else c.text, maxLines = 2)
            if (!subtitle.isNullOrEmpty()) {
                RText(subtitle, RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 4, ltr = ltrSubtitle)
            }
        }
        if (trailing != null) {
            Spacer(Modifier.width(10.dp))
            trailing()
        }
        if (chevron) {
            Spacer(Modifier.width(8.dp))
            GlyphIcon(Glyph.ChevronRight, c.tertiary, size = 14.dp)
        }
    }
}

/**
 * Grouped card with an optional small-caps header. [footer] is the longer explanation: it stays behind a (?) beside the
 * header until asked for.
 */
@Composable
fun Group(
    modifier: Modifier = Modifier,
    header: String? = null,
    footer: String? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    var help by rememberSaveable { mutableStateOf(false) }
    Column(modifier.padding(horizontal = 16.dp)) {
        if (header != null || footer != null) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 22.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(
                    header?.uppercase().orEmpty(),
                    RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
                    c.secondary,
                    Modifier.weight(1f),
                )
                if (footer != null) HelpButton(help, header ?: "this") { help = it }
            }
        }
        Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(c.elevated), content = content)
        if (footer != null) HelpText(footer, help, Modifier.padding(start = 16.dp, end = 16.dp, top = 8.dp))
    }
}

/** The small (?) that shows or hides the longer explanation of [topic]. */
@Composable
fun HelpButton(open: Boolean, topic: String, modifier: Modifier = Modifier, onToggle: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val bg by animateColorAsState(if (open) c.accentSoft else c.controlFill, label = "helpBg")
    Box(
        modifier
            .size(36.dp)
            .pressable(shape = CircleShape, dim = 0.6f, label = if (open) "Hide help" else "Help") {
                onToggle(!open)
                feedback.play(Event.expand(!open))
            }
            .semantics { contentDescription = "About $topic" }
            .testTag("help:$topic"),
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(22.dp).clip(CircleShape).background(bg), contentAlignment = Alignment.Center) {
            RText("?", RType.sans(12.5f, FontWeight.SemiBold), if (open) c.accent else c.secondary, maxLines = 1)
        }
    }
}

/** A (?) on its own line, at the end, for an explanation that belongs to no section header. */
@Composable
fun InlineHelp(topic: String, text: String, modifier: Modifier = Modifier, label: String? = null) {
    val c = LocalColors.current
    var open by rememberSaveable { mutableStateOf(false) }
    Column(modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            RText(label.orEmpty(), RType.sans(13f, FontWeight.Medium), c.tertiary, Modifier.weight(1f).padding(start = 4.dp), maxLines = 1)
            HelpButton(open, topic) { open = it }
        }
        HelpText(text, open, Modifier.padding(horizontal = 4.dp))
    }
}

/** The explanation a [HelpButton] opens. */
@Composable
fun HelpText(text: String, open: Boolean, modifier: Modifier = Modifier) {
    androidx.compose.animation.AnimatedVisibility(
        open,
        enter = androidx.compose.animation.expandVertically(spring(dampingRatio = 0.9f, stiffness = 600f)) + androidx.compose.animation.fadeIn(),
        exit = androidx.compose.animation.shrinkVertically(spring(dampingRatio = 0.9f, stiffness = 600f)) + androidx.compose.animation.fadeOut(),
    ) {
        RText(text, RType.sans(13f, lineHeight = 18f), LocalColors.current.secondary, modifier)
    }
}

@Composable
fun Hairline(modifier: Modifier = Modifier, inset: Dp = 16.dp) {
    Box(modifier.fillMaxWidth().padding(start = inset).height(0.75.dp).background(LocalColors.current.hairline))
}

/** Section title above a list. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
    RText(
        text.uppercase(),
        RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
        LocalColors.current.secondary,
        modifier.padding(start = 32.dp, top = 22.dp, bottom = 8.dp),
    )
}

/** Empty-state block: glyph, title, detail. */
@Composable
fun EmptyState(
    glyph: Glyph,
    title: String,
    detail: String?,
    modifier: Modifier = Modifier,
    tag: String? = null,
    /** What to do about it: buttons under the text. */
    actions: (@Composable ColumnScope.() -> Unit)? = null,
) {
    val c = LocalColors.current
    Column(
        modifier.fillMaxWidth().padding(32.dp).then(if (tag != null) Modifier.testTag(tag) else Modifier),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        GlyphIcon(glyph, c.tertiary, size = 34.dp)
        Spacer(Modifier.height(14.dp))
        RText(title, RType.sans(17f, FontWeight.SemiBold), c.text, align = TextAlign.Center)
        if (detail != null) {
            Spacer(Modifier.height(6.dp))
            RText(detail, RType.sans(14f), c.secondary, align = TextAlign.Center)
        }
        if (actions != null) {
            Spacer(Modifier.height(18.dp))
            Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(10.dp), content = actions)
        }
    }
}

enum class BannerKind { Error, Notice, Warning }

/** An inline message: tinted wash, glyph, text. */
@Composable
fun Banner(text: String, modifier: Modifier = Modifier, kind: BannerKind = BannerKind.Notice, tag: String? = null) {
    val c = LocalColors.current
    val tone = when (kind) {
        BannerKind.Error -> c.danger
        BannerKind.Warning -> c.warning
        BannerKind.Notice -> c.accent
    }
    Row(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .background(tone.copy(alpha = 0.1f))
            .padding(14.dp)
            .then(if (tag != null) Modifier.testTag(tag) else Modifier),
        verticalAlignment = Alignment.Top,
    ) {
        GlyphIcon(if (kind == BannerKind.Notice) Glyph.Info else Glyph.Warning, tone, size = 19.dp)
        Spacer(Modifier.width(10.dp))
        RText(text, RType.sans(14.5f, lineHeight = 20f), c.text)
    }
}

/** A filled text field (no Material). [tag] goes on the editable node so tests can type into it. */
@Composable
fun RTextField(
    value: String,
    onChange: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    tag: String? = null,
    enabled: Boolean = true,
    mono: Boolean = false,
    singleLine: Boolean = true,
    password: Boolean = false,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default,
) {
    val c = LocalColors.current
    val style = if (mono) RType.mono(14.5f) else RType.sans(16f)
    Box(
        modifier
            .fillMaxWidth()
            .background(c.controlFill, RoundedCornerShape(14.dp))
            .padding(horizontal = 14.dp, vertical = 13.dp),
    ) {
        if (value.isEmpty()) RText(placeholder, style, c.tertiary, maxLines = 1)
        BasicTextField(
            value,
            onChange,
            Modifier.fillMaxWidth().then(if (tag != null) Modifier.testTag(tag) else Modifier),
            enabled = enabled,
            textStyle = style.copy(color = c.text),
            cursorBrush = SolidColor(c.accent),
            singleLine = singleLine,
            visualTransformation = if (password) androidx.compose.ui.text.input.PasswordVisualTransformation() else VisualTransformation.None,
            keyboardOptions = keyboardOptions,
        )
    }
}

/** A modal question in the app's own chrome. */
@Composable
fun ConfirmDialog(
    title: String,
    text: String,
    confirmLabel: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    destructive: Boolean = true,
) {
    val c = LocalColors.current
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        DialogFeedback()
        Column(
            Modifier.padding(horizontal = 28.dp).fillMaxWidth().glass(c, RoundedCornerShape(26.dp), 16.dp).padding(22.dp),
        ) {
            RText(title, RType.sans(19f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(8.dp))
            RText(text, RType.sans(15f, lineHeight = 21f), c.secondary)
            Spacer(Modifier.height(22.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                CapsuleButton("Cancel", Modifier.weight(1f), style = ButtonStyle.Secondary, onClick = onDismiss)
                CapsuleButton(
                    confirmLabel,
                    Modifier.weight(1f),
                    style = if (destructive) ButtonStyle.Destructive else ButtonStyle.Primary,
                    onClick = onConfirm,
                )
            }
        }
    }
}

/** The expressive Material loader, tinted for the surface it sits on. */
@Composable
fun Spinner(tint: Color, modifier: Modifier = Modifier, size: Dp = 20.dp) {
    androidx.compose.material3.LoadingIndicator(modifier.size(size), color = tint)
}

/** A card-shaped surface for list items. */
@Composable
fun Card(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(modifier.fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(LocalColors.current.elevated), content = content)
}

/**
 * Text that gets crossed out when [struck]: a line is drawn across it from the left while the text fades back, so
 * dropping something from a list feels like striking it off.
 */
@Composable
fun StrikeText(text: String, style: androidx.compose.ui.text.TextStyle, color: Color, struck: Boolean, modifier: Modifier = Modifier) {
    val progress by androidx.compose.animation.core.animateFloatAsState(
        if (struck) 1f else 0f,
        androidx.compose.animation.core.tween(320),
        label = "strike",
    )
    Box(
        modifier.drawWithContent {
            drawContent()
            if (progress > 0f) {
                val y = size.height * 0.55f
                drawLine(color, androidx.compose.ui.geometry.Offset(0f, y), androidx.compose.ui.geometry.Offset(size.width * progress, y), strokeWidth = 2.dp.toPx(), cap = androidx.compose.ui.graphics.StrokeCap.Round)
            }
        },
    ) { RText(text, style, color, maxLines = 1, ltr = true) }
}
