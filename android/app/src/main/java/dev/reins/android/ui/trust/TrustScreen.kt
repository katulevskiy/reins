package dev.reins.android.ui.trust

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.DontMemoize
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.Timers
import dev.reins.android.platform.Authenticator
import dev.reins.android.state.AppState
import dev.reins.android.ui.activity.dayOf
import dev.reins.android.ui.autopilot.AutopilotModes
import dev.reins.android.ui.autopilot.AutopilotSheets
import dev.reins.android.ui.autopilot.AutopilotSheetsState
import dev.reins.android.ui.autopilot.AutopilotViewModel
import dev.reins.android.ui.autopilot.AutopilotWeek
import dev.reins.android.ui.autopilot.ModelRows
import dev.reins.android.ui.autopilot.ProfilesSection
import dev.reins.android.ui.autopilot.RunningFold
import dev.reins.android.ui.autopilot.TimedKind
import dev.reins.android.ui.autopilot.TrustRunningField
import dev.reins.android.ui.autopilot.trustField
import dev.reins.android.ui.grants.GrantsSection
import dev.reins.android.ui.grants.StartingRuleSection
import dev.reins.android.ui.main.RootHeader
import dev.reins.android.ui.main.mood
import dev.reins.core.ActivityEntry
import dev.reins.core.AutopilotMode
import dev.reins.core.StartingPolicy

/**
 * The Trust tab (Reins Signal, iOS-Trust 01 / And-Core 03): everything that runs without asking, in one place. A
 * running bypass, Overnight or lockdown as a field on top; the mode tiles; this week's numbers; the grants that run;
 * the profiles; the model and Try it; the starting rule. Autopilot's and the grants' views, moved here as sections.
 */
@Composable
fun TrustScreen(
    autopilot: AutopilotViewModel,
    state: AppState,
    authenticator: Authenticator,
    onProfile: (String) -> Unit,
    onTryIt: () -> Unit,
    onOpenGrant: (String) -> Unit,
    onNewGrant: () -> Unit,
    onEnded: () -> Unit,
    onPermissions: () -> Unit,
    onStartingPolicy: (StartingPolicy) -> Unit,
    /** An entry of the running mode's list: its page. */
    onOpenEntry: (Long) -> Unit = {},
    /** A sheet is up: the tab bar steps aside so the sheet's buttons are not under it. */
    onCovered: (Boolean) -> Unit = {},
) {
    val c = LocalColors.current
    LaunchedEffect(autopilot) { autopilot.showingAutopilot() }
    val settings by autopilot.settings.collectAsStateWithLifecycle()
    val ui by autopilot.ui.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val entries by state.activity.collectAsStateWithLifecycle()
    val startingPolicy by state.startingPolicy.collectAsStateWithLifecycle()
    val sheets = remember { AutopilotSheetsState() }
    LaunchedEffect(sheets.covered) { onCovered(sheets.covered) }
    DisposableEffect(Unit) { onDispose { onCovered(false) } }
    val decided = remember(entries) { decidedToday(entries, (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000) }

    val scroll = rememberScrollState()
    // A running mode's field stays pinned on top and folds as the page scrolls under it.
    val fold = remember { RunningFold() }
    val density = LocalDensity.current
    // The running mode's list opens in place; folded each time Trust opens (leaving the tab disposes this page).
    var listOpen by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxSize().background(c.paper).testTag("trust")) {
        val s = settings
        val field = trustField(s)
        Column(Modifier.fillMaxSize().verticalScroll(scroll).testTag("trustScroll")) {
            // The room the open field takes; the page scrolls on under it as it folds.
            if (s != null && field != null) Spacer(Modifier.height(with(density) { fold.open.toDp() }))
            val decidedWhileRunning = remember(entries, s) { runningEntries(entries, s) }
            // While a bypass or an Overnight runs, what it did today (from the same entries as its list); else Autopilot's.
            val moodLine = s?.let { runningMood(it, decidedWhileRunning, (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000) }
                ?: decidedLine(decided)
            RootHeader(
                "Trust",
                mood = mood(null, moodLine),
                underField = field != null,
            ) { RoundIconButton(Glyph.Plus, "New grant", Modifier.testTag("newGrant"), onClick = onNewGrant) }
            if (s != null && decidedWhileRunning != null) {
                RunningList(
                    s,
                    decidedWhileRunning,
                    nowSeconds = (Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000,
                    expanded = listOpen,
                    onExpanded = { listOpen = it },
                    onOpen = onOpenEntry,
                    modifier = Modifier.padding(horizontal = 24.dp),
                )
            }
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(top = 16.dp).testTag("modeHero")) {
                if (s == null) {
                    Spacer(Modifier.height(40.dp))
                } else {
                    val model = ui.model ?: s.model
                    AutopilotModes(
                        s,
                        AutopilotText.modelUsable(model),
                        onPick = @DontMemoize { mode -> if (mode != s.mode) autopilot.setMode(mode) },
                        onBypass = { sheets.bypass = true },
                        onOvernight = { sheets.overnight = true },
                        onLockdown = @DontMemoize { autopilot.setMode(AutopilotMode.LOCKDOWN) },
                    )
                    AutopilotWeek(autopilot, s)
                }
            }
            Column(Modifier.fillMaxWidth().padding(horizontal = 24.dp)) {
                CapsuleButton("Account and temporary permissions", Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("trustPermissions"), style = ButtonStyle.Soft, onClick = onPermissions, maxLines = Int.MAX_VALUE)
                GrantsSection(state, onOpen = onOpenGrant, onEnded = onEnded)
                val s2 = settings
                if (s2 != null) {
                    ProfilesSection(autopilot, s2, connections, sheets, onProfile)
                    ModelRows(autopilot, s2, sheets, onTryIt)
                }
                StartingRuleSection(startingPolicy, Modifier.padding(top = 26.dp).testTag("startingRule"), onStartingPolicy)
            }
            Spacer(Modifier.height(32.dp).testTag("pageEnd"))
        }
        if (s != null && field != null) {
            TrustRunningField(
                s,
                fold,
                scrolled = { scroll.value },
                onStop = @DontMemoize { kind -> if (kind == TimedKind.Bypass) autopilot.stopBypass() else autopilot.stopOvernight() },
                onEndLockdown = @DontMemoize { autopilot.endLockdown(s.baseMode.takeIf { it != AutopilotMode.LOCKDOWN } ?: AutopilotMode.MANUAL, authenticator) },
            )
        }
        AutopilotSheets(autopilot, sheets, authenticator, onProfile)
    }
}

/** How many requests Autopilot (or Overnight) decided today. */
internal fun decidedToday(entries: List<ActivityEntry>, nowSeconds: Long): Int {
    val today = dayOf(nowSeconds)
    return entries.count { (it.decidedBy == "autopilot" || it.decidedBy == "overnight") && dayOf(it.at) == today }
}

/** "Autopilot decided 9 things today." */
internal fun decidedLine(n: Int): String = when (n) {
    0 -> "Autopilot decided nothing yet today."
    1 -> "Autopilot decided 1 thing today."
    else -> "Autopilot decided $n things today."
}
