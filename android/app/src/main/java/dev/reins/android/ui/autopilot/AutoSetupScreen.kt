package dev.reins.android.ui.autopilot

import androidx.activity.compose.BackHandler
import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.BackArrow
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Hairline
import dev.reins.android.design.HelpText
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.NextPill
import dev.reins.android.design.PaneGroup
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SectionHeader
import dev.reins.android.design.SignalButton
import dev.reins.android.design.pressable
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.activity.Zones
import dev.reins.android.ui.activity.zoneGlyph
import dev.reins.android.ui.activity.zoneName
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.main.HeaderInsets
import dev.reins.android.ui.settings.HelpIcon
import dev.reins.core.AutoSetup
import dev.reins.core.CellChoice
import dev.reins.core.MatrixRowView
import dev.reins.core.WorldItem

// Turning on Auto (Decisions v2, 01 §9.1): whose things are (Your world), what Auto may do in each zone at each level
// (the matrix, with real names), then Turn on, held and checked. The safety nets step comes with the nets (P3).

/** The setup's steps, in order. */
private enum class SetupStep(val title: String, val help: String?) {
    World("Your world", "Auto does more for what's yours. Unsure? Leave it Outside."),
    Matrix("What Auto does", "Rows: whose it is. Columns: how hard it is to undo. Tap a cell to change it."),
    TurnOn("Turn on Auto", null),
}

/** The sections of Your world, by the item's `service`, in this order; an empty one is hidden. */
private val Sections = listOf(
    Triple("github", "GitHub", Glyph.GitBranch),
    Triple("email", "Email", Glyph.Mail),
    Triple("messages", "Messages", Glyph.Send),
    Triple("computers", "Computers", Glyph.Laptop),
)

/** The matrix's columns: L0 to L3. */
private val Levels = listOf("Undoable", "Recoverable", "Reaches people", "Can't be undone")

/** A tap moves a cell along: Auto, Ask first, Ask, Auto again. */
internal fun nextCell(cell: CellChoice): CellChoice = when (cell) {
    CellChoice.AUTO -> CellChoice.ASK_FIRST
    CellChoice.ASK_FIRST -> CellChoice.ASK
    CellChoice.ASK -> CellChoice.AUTO
}

internal fun cellName(cell: CellChoice): String = when (cell) {
    CellChoice.AUTO -> "Auto"
    CellChoice.ASK_FIRST -> "Ask first"
    CellChoice.ASK -> "Ask"
}

@Composable
fun AutoSetupScreen(viewModel: AutopilotViewModel, authenticator: Authenticator, onBack: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.showingSetup() }
    var at by rememberSaveable { mutableIntStateOf(0) }
    val step = SetupStep.entries[at]
    var help by rememberSaveable(at) { mutableStateOf(false) }
    BackHandler(enabled = at > 0) { at-- }
    val setup = ui.setup

    Column(Modifier.fillMaxSize().background(c.paper).testTag("autoSetup")) {
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()).windowInsetsPadding(HeaderInsets)) {
            Row(Modifier.fillMaxWidth().padding(start = 12.dp, end = 16.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                BackArrow({ if (at > 0) at-- else onBack() })
                Spacer(Modifier.weight(1f))
                StepDots(at, SetupStep.entries.size)
            }
            Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 12.dp, top = 14.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(step.title, RType.pageTitle, c.ink, Modifier.weight(1f).semantics { heading() }.testTag("autoSetupTitle"), maxLines = 2)
                if (step.help != null) HelpIcon(help, step.title) { help = it }
            }
            step.help?.let { HelpText(it, help, Modifier.padding(start = 20.dp, end = 20.dp, top = 4.dp)) }
            ui.error?.let { Banner(it, Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp), BannerKind.Error, tag = "autoSetupError") }
            when {
                setup == null -> Unit
                step == SetupStep.World -> WorldStep(setup) { item, zone -> viewModel.setWorldZone(item, zone, authenticator) }
                step == SetupStep.Matrix -> MatrixStep(setup) { row, level -> viewModel.setMatrixCell(row.zone, level, nextCell(row.cells[level]), authenticator) }
                else -> TurnOnStep()
            }
            Spacer(Modifier.height(24.dp))
        }
        Column(Modifier.fillMaxWidth().background(c.paper).windowInsetsPadding(WindowInsets.navigationBars).padding(16.dp)) {
            if (step == SetupStep.TurnOn) {
                HoldButton(
                    "Hold to turn on",
                    c.violet,
                    Modifier.fillMaxWidth().testTag("turnOnAuto"),
                    busy = ui.busy,
                    enabled = setup != null,
                ) { viewModel.finishSetup(authenticator, onDone) }
            } else {
                SignalButton("Next", Modifier.fillMaxWidth().testTag("autoSetupNext"), enabled = setup != null) { at++ }
            }
        }
    }
}

/** Where the setup is: a dot per step, the current one violet and wider. */
@Composable
private fun StepDots(at: Int, count: Int) {
    val c = LocalColors.current
    Row(
        Modifier.semantics(mergeDescendants = true) { contentDescription = "Step ${at + 1} of $count" }.testTag("autoSetupStep:${at + 1}"),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        repeat(count) { i ->
            Box(Modifier.height(6.dp).width(if (i == at) 18.dp else 6.dp).clip(CircleShape).background(if (i <= at) c.violet else c.rule2))
        }
    }
}

/** Step 1: each thing Auto knows of, by service, with how often it came up and whose it is. */
@Composable
private fun WorldStep(setup: AutoSetup, onZone: (WorldItem, dev.reins.core.ZoneChoice) -> Unit) {
    if (setup.world.isEmpty()) {
        EmptyState(Glyph.Globe, "Nothing to sort yet", "Connect GitHub, email or a computer first.", tag = "worldEmpty")
        return
    }
    Sections.forEach { (service, name, glyph) ->
        val items = setup.world.filter { it.service == service }
        if (items.isEmpty()) return@forEach
        SectionHeader(name, Modifier.testTag("worldSection:$service"), glyph = glyph, count = "${items.size}")
        PaneGroup {
            items.forEachIndexed { i, item ->
                if (i > 0) Hairline(inset = 14.dp)
                WorldRow(item) { onZone(item, it) }
            }
        }
    }
    // Anything the core lists under a section this screen does not know still shows, last.
    val other = setup.world.filter { item -> Sections.none { it.first == item.service } }
    if (other.isNotEmpty()) {
        SectionHeader("Other", glyph = Glyph.More, count = "${other.size}")
        PaneGroup {
            other.forEachIndexed { i, item ->
                if (i > 0) Hairline(inset = 14.dp)
                WorldRow(item) { onZone(item, it) }
            }
        }
    }
}

/** One thing: its name, how often requests touched it, "Suggested" until confirmed, and the four zones. */
@Composable
private fun WorldRow(item: WorldItem, onZone: (dev.reins.core.ZoneChoice) -> Unit) {
    val c = LocalColors.current
    val id = "${item.kind.name}:${item.key}"
    Column(Modifier.fillMaxWidth().padding(14.dp).testTag("world:$id"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            RText(untrusted(item.label), RType.row, c.ink, Modifier.weight(1f), maxLines = 1)
            if (!item.confirmed) NextPill("Suggested", c.violet, Modifier.testTag("proposed:$id"))
            if (item.count > 0u) {
                Row(verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(3.dp)) {
                    RText("${item.count}", RType.sans(15f, FontWeight.SemiBold), c.ink, Modifier.testTag("count:$id"), maxLines = 1)
                    RText("requests", RType.meta, c.inkMeta, maxLines = 1)
                }
            }
        }
        Chooser(
            Zones.map { Choice(it, zoneName(it), "zone:$id:${it.name}") },
            item.zone,
            onZone,
        )
    }
}

/** Step 2: the matrix, a row per zone with real names under it, a column per level; L3 always asks. */
@Composable
private fun MatrixStep(setup: AutoSetup, onCell: (MatrixRowView, Int) -> Unit) {
    val c = LocalColors.current
    Column(Modifier.padding(start = 16.dp, end = 16.dp, top = 18.dp).testTag("matrix"), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.Bottom) {
            Spacer(Modifier.width(ZONE_COLUMN))
            Levels.forEach { level ->
                RText(level, RType.sans(10.5f, FontWeight.Medium, 13f), c.inkMeta, Modifier.weight(1f), maxLines = 2, align = androidx.compose.ui.text.style.TextAlign.Center)
            }
        }
        setup.matrix.forEach { row ->
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.width(ZONE_COLUMN).testTag("matrixZone:${row.zone.name}")) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        GlyphIcon(zoneGlyph(row.zone), c.ink2, size = 14.dp)
                        RText(zoneName(row.zone), RType.sans(14f, FontWeight.SemiBold), c.ink, maxLines = 1)
                    }
                    row.names.take(3).forEach { name ->
                        RText(untrusted(name), RType.sans(11.5f, lineHeight = 14f), c.inkMeta, maxLines = 1)
                    }
                }
                row.cells.take(Levels.size).forEachIndexed { level, cell ->
                    MatrixCell(cell, locked = level == Levels.lastIndex, Modifier.weight(1f).testTag("cell:${row.zone.name}:$level")) { onCell(row, level) }
                }
            }
        }
    }
}

private val ZONE_COLUMN = 78.dp

/** One cell: Auto violet, Ask first amber, Ask plain; a locked one (can't be undone) shows the lock and stays Ask. */
@Composable
private fun MatrixCell(cell: CellChoice, locked: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalColors.current
    val shown = if (locked) CellChoice.ASK else cell
    val (fill, ink) = cellColors(shown, c)
    val bg by animateColorAsState(fill, label = "cell")
    val shape = RoundedCornerShape(12.dp)
    Box(
        modifier
            .heightIn(min = 52.dp)
            .clip(shape)
            .background(bg)
            .then(if (locked) Modifier else Modifier.pressable(shape = shape, label = "Change", onClick = onClick))
            .semantics(mergeDescendants = true) {
                role = Role.Button
                stateDescription = if (locked) "Ask, always" else cellName(shown)
            },
        contentAlignment = Alignment.Center,
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(3.dp), modifier = Modifier.padding(horizontal = 4.dp)) {
            if (locked) GlyphIcon(Glyph.Lock, ink, size = 11.dp)
            RText(cellName(shown), RType.sans(12.5f, FontWeight.SemiBold), ink, maxLines = 2, align = androidx.compose.ui.text.style.TextAlign.Center)
        }
    }
}

private fun cellColors(cell: CellChoice, c: RColors): Pair<Color, Color> = when (cell) {
    CellChoice.AUTO -> c.violet.copy(alpha = 0.16f) to c.violet
    CellChoice.ASK_FIRST -> c.warning.copy(alpha = 0.16f) to c.warning
    CellChoice.ASK -> c.paperSunken to c.ink2
}

/** Step 3: what turning on means, in a line; the hold is in the dock below. */
@Composable
private fun TurnOnStep() {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 28.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Box(Modifier.size(72.dp).clip(RoundedCornerShape(22.dp)).background(c.violet.copy(alpha = 0.14f)), contentAlignment = Alignment.Center) {
            GlyphIcon(Glyph.Autopilot, c.violet, size = 36.dp)
        }
        RText("Where it says Ask, it asks you.", RType.sans(16f), c.ink2, Modifier.testTag("turnOnLine"), align = androidx.compose.ui.text.style.TextAlign.Center)
    }
}
