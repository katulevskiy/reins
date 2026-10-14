package dev.reins.android.ui.settings

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
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
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.HelpText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Spinner
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.SheetOpenFeedback
import dev.reins.android.feedback.play

// The pieces Settings and a connection's page are built from: a back arrow over a large title, cards of rows with a
// round tonal icon each (no dividers), accent section headers, status chips and pill buttons.

/** The card behind a group of rows. */
internal fun Modifier.settingsCard(c: RColors, radius: Dp = 24.dp): Modifier = this
    .fillMaxWidth()
    .clip(RoundedCornerShape(radius))
    .background(if (c.dark) Color(0xFF121215) else c.elevated)
    .border(1.dp, if (c.dark) Color(0xFF1B1B1F) else c.hairline, RoundedCornerShape(radius))

/** A page: the back arrow when pushed ([onBack]; none at the root of a tab), an optional large [title], and content that scrolls under them. */
@Composable
fun SettingsPage(
    title: String?,
    onBack: (() -> Unit)?,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    Column(modifier.fillMaxSize().background(c.background).imePadding()) {
        if (onBack != null) {
            Row(
                Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.statusBars).height(56.dp).padding(start = 6.dp, end = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(
                    Modifier.size(44.dp).pressable(shape = CircleShape, label = "Back", onClick = onBack).semantics { contentDescription = "Back" }.testTag("back"),
                    contentAlignment = Alignment.Center,
                ) { Icon(Icons.AutoMirrored.Rounded.ArrowBack, null, Modifier.size(24.dp), tint = c.text) }
            }
        } else {
            Spacer(Modifier.windowInsetsPadding(WindowInsets.statusBars))
        }
        Column(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .windowInsetsPadding(WindowInsets.navigationBars),
        ) {
            if (title != null) {
                RText(
                    title,
                    RType.sans(30f, FontWeight.SemiBold),
                    c.text,
                    Modifier.padding(start = 20.dp, end = 20.dp, top = if (onBack == null) 22.dp else 4.dp, bottom = 16.dp).semantics { heading() },
                    maxLines = 1,
                )
            }
            content()
            // At the root of a tab, the content clears the floating tab bar.
            if (onBack == null) Spacer(Modifier.height(96.dp).testTag("pageEnd"))
        }
    }
}

/**
 * A group of rows on a card. [header] is in the accent colour above it; without one the group sits a section gap
 * below the last. [help] puts a (?) at the end of the header that shows the longer text under it.
 */
@Composable
fun SettingsGroup(
    modifier: Modifier = Modifier,
    header: String? = null,
    help: String? = null,
    helpOpen: Boolean = false,
    onHelp: (Boolean) -> Unit = {},
    gap: Dp = 22.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = LocalColors.current
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp)) {
        if (header != null) {
            Row(
                Modifier.fillMaxWidth().padding(start = 6.dp, top = gap, bottom = if (help != null) 0.dp else 8.dp).defaultMinSize(minHeight = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                // The (?) right beside the words it explains.
                RText(header, RType.sans(14f, FontWeight.Medium), c.accent, Modifier.weight(1f, fill = false).semantics { heading() }, maxLines = 1)
                if (help != null) HelpIcon(helpOpen, header, onHelp)
                Spacer(Modifier.weight(1f))
            }
            if (help != null) HelpText(help, helpOpen, Modifier.padding(start = 6.dp, end = 6.dp, bottom = 8.dp))
        } else {
            Spacer(Modifier.height(gap))
        }
        Column(Modifier.settingsCard(c), content = content)
    }
}

/** The small (?) that shows or hides the longer explanation of [topic]. */
@Composable
fun HelpIcon(open: Boolean, topic: String, onToggle: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Box(
        Modifier
            .size(36.dp)
            .pressable(shape = CircleShape, dim = 0.6f, label = if (open) "Hide help" else "Help") {
                onToggle(!open)
                feedback.play(Event.expand(!open))
            }
            .semantics { contentDescription = "About $topic" }
            .testTag("help:$topic"),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.AutoMirrored.Rounded.HelpOutline, null, Modifier.size(19.dp), tint = if (open) c.accent else c.tertiary) }
}

/** The round tonal plate a row's icon sits on. */
@Composable
fun IconPlate(icon: ImageVector, tint: Color, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    Box(modifier.size(size).clip(CircleShape).background(tint.copy(alpha = 0.15f)), contentAlignment = Alignment.Center) {
        Icon(icon, null, Modifier.size(size * 0.55f), tint = tint)
    }
}

/**
 * A row of a settings card: the [icon] on its tonal plate (or any [leading]), the title (and an optional line under
 * it), the [value] on the right, and a chevron when it opens a page.
 */
@Composable
fun SettingsRow(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tint: Color = LocalColors.current.tertiary,
    subtitle: String? = null,
    subtitleTag: String? = null,
    value: String? = null,
    monoValue: Boolean = false,
    chevron: Boolean = true,
    destructive: Boolean = false,
    titleColor: Color? = null,
    busy: Boolean = false,
    leading: (@Composable () -> Unit)? = null,
    valueContent: (@Composable RowScope.() -> Unit)? = null,
    onClick: (() -> Unit)? = null,
) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 64.dp)
            .then(if (onClick != null) Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), onClick = onClick) else Modifier)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        when {
            leading != null -> leading()
            icon != null -> IconPlate(icon, if (destructive) c.danger else tint)
        }
        if (leading != null || icon != null) Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(16.5f, FontWeight.Medium), titleColor ?: if (destructive) c.danger else c.text, maxLines = 1)
            if (!subtitle.isNullOrEmpty()) {
                RText(subtitle, RType.sans(13f), c.tertiary, Modifier.padding(top = 2.dp).testTag(subtitleTag ?: "rowDetail"), maxLines = 1, ltr = true)
            }
        }
        if (valueContent != null) {
            Spacer(Modifier.width(10.dp))
            valueContent()
        } else if (value != null) {
            Spacer(Modifier.width(10.dp))
            RText(value, if (monoValue) RType.mono(14f) else RType.sans(15f), c.secondary, maxLines = 1, ltr = monoValue)
        }
        if (busy) {
            Spacer(Modifier.width(8.dp))
            Spinner(c.secondary, size = 18.dp)
        } else if (chevron && onClick != null) {
            Spacer(Modifier.width(6.dp))
            Icon(Icons.Rounded.ChevronRight, null, Modifier.size(20.dp), tint = c.tertiary)
        }
    }
}

enum class ChipTone { Ok, Warn, Plain }

/** A small status chip: an icon and a few words, green when all is well and yellow when something needs a look. */
@Composable
fun StatusChip(text: String, icon: ImageVector, tone: ChipTone, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val fg = when (tone) {
        ChipTone.Ok -> c.success
        ChipTone.Warn -> c.warning
        ChipTone.Plain -> c.secondary
    }
    Row(
        modifier
            .semantics(mergeDescendants = true) {}
            .clip(RoundedCornerShape(10.dp))
            .background(if (tone == ChipTone.Plain) c.controlFill else fg.copy(alpha = 0.12f))
            .padding(start = 9.dp, end = 12.dp, top = 7.dp, bottom = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(15.dp), tint = fg)
        Spacer(Modifier.width(6.dp))
        RText(text, RType.sans(13f, FontWeight.SemiBold), fg, maxLines = 1)
    }
}

/** A 40 dp pill: a light primary one, a tinted one, or a quiet one. */
@Composable
fun PillButton(
    title: String,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    background: Color,
    foreground: Color,
    height: Dp = 40.dp,
    enabled: Boolean = true,
    busy: Boolean = false,
    textSize: Float = 14.5f,
    textTag: String? = null,
    /** The icon's own colour, when it is not the text's. */
    iconTint: Color? = null,
    onClick: () -> Unit,
) {
    Row(
        modifier
            .height(height)
            .graphicsLayer { alpha = if (enabled) 1f else 0.45f }
            .clip(CircleShape)
            .background(background)
            .pressable(enabled = enabled && !busy, shape = CircleShape, onClick = onClick)
            .padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (busy) {
            Spinner(foreground, size = 16.dp)
            Spacer(Modifier.width(8.dp))
        } else if (icon != null) {
            Icon(icon, null, Modifier.size(18.dp), tint = iconTint ?: foreground)
            Spacer(Modifier.width(7.dp))
        }
        RText(title, RType.sans(textSize, FontWeight.SemiBold), foreground, if (textTag != null) Modifier.testTag(textTag) else Modifier, maxLines = 1)
    }
}

/** The light, high-contrast fill of a primary pill (and the dark text on it). */
internal fun primaryFill(c: RColors): Pair<Color, Color> =
    if (c.dark) Color(0xFFF2F2F4) to Color(0xFF0A0A0A) else c.text to Color.White

/**
 * A connected button group: one choice of a few, the outer ends fully round and the inner corners small, the chosen
 * one in a wash of the accent. [selected] may be true of none of them.
 */
@Composable
fun <T> ConnectedButtons(
    options: List<T>,
    selected: (T) -> Boolean,
    label: (T) -> String,
    tag: (T) -> String,
    modifier: Modifier = Modifier,
    icon: (T) -> Pair<ImageVector, Color>? = { null },
    onSelect: (T) -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(modifier.fillMaxWidth().height(40.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        options.forEachIndexed { i, option ->
            val on = selected(option)
            val first = i == 0
            val last = i == options.lastIndex
            val shape = RoundedCornerShape(
                topStart = if (first) 20.dp else 6.dp,
                bottomStart = if (first) 20.dp else 6.dp,
                topEnd = if (last) 20.dp else 6.dp,
                bottomEnd = if (last) 20.dp else 6.dp,
            )
            val bg by animateColorAsState(if (on) c.accent.copy(alpha = if (c.dark) 0.24f else 0.16f) else c.controlFill, label = "segBg")
            Row(
                Modifier
                    .weight(1f)
                    .height(40.dp)
                    .clip(shape)
                    .background(bg)
                    .pressable(shape = shape, dim = 0.7f) {
                        if (!on) {
                            onSelect(option)
                            feedback.play(Event.Selection)
                        }
                    }
                    .semantics {
                        this.selected = on
                        role = Role.Tab
                    }
                    .testTag(tag(option)),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                icon(option)?.let { (vector, tint) ->
                    Icon(vector, null, Modifier.size(16.dp), tint = tint)
                    Spacer(Modifier.width(4.dp))
                }
                RText(label(option), RType.sans(14f, FontWeight.SemiBold), if (on) c.text else c.secondary, maxLines = 1)
            }
        }
    }
}

/**
 * A sheet that slides up from the bottom of the page over a dim: a grab handle, a [title] with a close button (none
 * without a title), and [content]. Tapping outside, the close button or Back dismisses it.
 */
@Composable
fun BoxScope.BottomSheet(open: Boolean, title: String?, onClose: () -> Unit, tag: String, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    SheetOpenFeedback(open)
    val dismiss = {
        feedback.play(Event.Close)
        onClose()
    }
    BackHandler(enabled = open, onBack = dismiss)
    AnimatedVisibility(open, Modifier.matchParentSize(), enter = fadeIn(tween(180)), exit = fadeOut(tween(160))) {
        Box(
            Modifier
                .fillMaxSize()
                .background(c.scrim)
                .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = dismiss),
        )
    }
    AnimatedVisibility(
        open,
        Modifier.align(Alignment.BottomCenter),
        enter = slideInVertically(tween(260)) { it },
        exit = slideOutVertically(tween(200)) { it },
    ) {
        val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
        Column(
            Modifier
                .fillMaxWidth()
                .clip(shape)
                .background(if (c.dark) Color(0xFF19191C) else c.elevated)
                .border(0.75.dp, c.glassEdge, shape)
                // Taps on the sheet stay on it (a gesture, not a click: a click would merge the rows into one node
                // for screen readers).
                .pointerInput(Unit) { detectTapGestures { } }
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(bottom = 16.dp)
                .testTag(tag),
        ) {
            Box(Modifier.fillMaxWidth().padding(top = 10.dp), contentAlignment = Alignment.Center) {
                Box(Modifier.size(width = 32.dp, height = 4.dp).clip(CircleShape).background(c.tertiary.copy(alpha = 0.6f)))
            }
            if (title != null) {
                Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 10.dp, top = 12.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                    RText(title, RType.sans(18f, FontWeight.SemiBold), c.text, Modifier.weight(1f), maxLines = 1)
                    Box(
                        Modifier.size(44.dp).pressable(shape = CircleShape, label = "Close", onClick = dismiss).semantics { contentDescription = "Close" }.testTag("closeSheet"),
                        contentAlignment = Alignment.Center,
                    ) { Icon(Icons.Rounded.Close, null, Modifier.size(22.dp), tint = c.secondary) }
                }
            } else {
                Spacer(Modifier.height(14.dp))
            }
            Column(Modifier.verticalScroll(rememberScrollState()), content = content)
        }
    }
}
