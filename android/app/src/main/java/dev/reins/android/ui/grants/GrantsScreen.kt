package dev.reins.android.ui.grants

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Hairline
import dev.reins.android.design.LargeTitle
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.glass
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.state.AppState
import dev.reins.android.ui.common.EndedGrantRow
import dev.reins.android.ui.common.GrantTile
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.main.SettingsButton
import dev.reins.core.GrantView
import dev.reins.core.StandingGrant
import kotlinx.coroutines.launch

/** Space the floating navigation bar takes at the bottom of the screen. */
private val NavBarSpace = 92.dp
private val PillHeight = 52.dp

/**
 * The second tab: the permissions that are running, each with its time left. The ones that ended (expired, used up or
 * deleted) wait in an island that sticks above the navigation bar and slides up over the list when opened.
 */
@Composable
fun GrantsScreen(
    state: AppState,
    expiredOpen: Boolean,
    onToggleExpired: () -> Unit,
    onOpen: (String) -> Unit,
    onNew: () -> Unit,
    onResume: suspend (grantId: String, seconds: Long, standing: StandingGrant?) -> String?,
    onDelete: suspend (grantId: String) -> String? = { null },
    onSettings: () -> Unit = {},
    /** The starting rule for new AIs, changed at the end of the list. */
    onStartingPolicy: (dev.reins.core.StartingPolicy) -> Unit = {},
) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val startingPolicy by state.startingPolicy.collectAsStateWithLifecycle()
    val running = remember(grants) { grants.filter { it.active }.sortedByDescending { it.createdAt } }
    val ended = remember(grants) { grants.filterNot { it.active }.sortedByDescending { it.createdAt } }
    var resuming by remember { mutableStateOf<GrantView?>(null) }
    var deleting by remember { mutableStateOf<GrantView?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Box(Modifier.fillMaxSize().background(c.background)) {
        LazyColumn(
            Modifier.fillMaxSize(),
            contentPadding = PaddingValues(bottom = if (ended.isEmpty()) 130.dp else NavBarSpace + PillHeight + 56.dp),
        ) {
            item(key = "title") {
                LargeTitle("Grants") {
                    dev.reins.android.ui.main.HeaderPill("New grant", Glyph.Plus, "newGrant", onNew)
                    SettingsButton(onSettings)
                }
            }
            error?.let {
                item(key = "error") { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp), BannerKind.Error, tag = "resumeError") }
            }
            if (running.isEmpty()) {
                item(key = "empty") {
                    EmptyState(
                        Glyph.Key,
                        if (grants.isEmpty()) "No grants" else "No active grants",
                        null,
                        tag = "noGrants",
                    )
                }
            }
            items(running, key = { "g:${it.id}" }, contentType = { "grant" }) { grant ->
                Box(Modifier.testTag("grant:${grant.id}")) { GrantTile(grant) { onOpen(grant.id) } }
            }
            item(key = "startingRule") {
                StartingRuleChooser(startingPolicy, Modifier.padding(top = 18.dp).testTag("startingRule"), onStartingPolicy)
            }
        }
        if (ended.isNotEmpty()) {
            ExpiredIsland(
                ended = ended,
                open = expiredOpen,
                onToggle = feedbackAction(Event.expand(!expiredOpen), onToggleExpired),
                onOpen = onOpen,
                onResume = { resuming = it },
                onDelete = { deleting = it },
            )
        }
    }

    resuming?.let { grant ->
        ResumeDialog(
            grant = grant,
            onDismiss = { resuming = null },
            onResume = { seconds, standing ->
                resuming = null
                error = null
                scope.launch { error = onResume(grant.id, seconds, standing) }
            },
        )
    }
    deleting?.let { grant ->
        ConfirmDialog(
            title = "Delete this grant for good?",
            text = untrusted(grant.summary) + "\n\nIt cannot be resumed afterwards.",
            confirmLabel = "Delete",
            onConfirm = {
                deleting = null
                error = null
                scope.launch { error = onDelete(grant.id) }
            },
            onDismiss = { deleting = null },
        )
    }
}

/**
 * The header pill that always sits just above the navigation bar, and the panel that rises out from behind it.
 *
 * The motion is a single progress value read only where things are drawn (a graphics layer for the panel, the scrim
 * and the chevron), so a frame of the animation re-composes and re-measures nothing: the list keeps its layout and the
 * GPU thread just moves the panel's layer.
 */
@Composable
private fun ExpiredIsland(
    ended: List<GrantView>,
    open: Boolean,
    onToggle: () -> Unit,
    onOpen: (String) -> Unit,
    onResume: (GrantView) -> Unit,
    onDelete: (GrantView) -> Unit,
) {
    val c = LocalColors.current
    val progress by animateFloatAsState(if (open) 1f else 0f, tween(durationMillis = 340, easing = FastOutSlowInEasing), label = "expired")
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val pillBottom = NavBarSpace
        val panelBottom = NavBarSpace + PillHeight + 8.dp
        val visible = open || progress > 0f

        if (visible) {
            Box(
                Modifier
                    .fillMaxSize()
                    .graphicsLayer { alpha = progress }
                    .background(c.scrim)
                    .then(
                        if (open) {
                            Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onToggle)
                        } else {
                            Modifier
                        },
                    ),
            )
            Column(
                Modifier
                    .align(Alignment.BottomCenter)
                    .windowInsetsPadding(WindowInsets.navigationBars)
                    .padding(start = 16.dp, end = 16.dp, bottom = panelBottom)
                    .heightIn(max = maxHeight * 0.58f)
                    .graphicsLayer {
                        // Starts hidden below the screen edge, behind the pill and the navigation bar.
                        translationY = (1f - progress) * (size.height + (panelBottom + 60.dp).toPx())
                    }
                    .testTag("expiredList")
                    .glass(c, RoundedCornerShape(26.dp), 14.dp)
                    .verticalScroll(rememberScrollState()),
            ) {
                ended.forEachIndexed { index, grant ->
                    if (index > 0) Hairline(inset = 74.dp)
                    Box(Modifier.testTag("ended:${grant.id}")) {
                        EndedGrantRow(grant, onOpen = { onOpen(grant.id) }, onResume = { onResume(grant) }, onDelete = { onDelete(grant) })
                    }
                }
            }
        }

        Row(
            Modifier
                .align(Alignment.BottomCenter)
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(start = 16.dp, end = 16.dp, bottom = pillBottom)
                .fillMaxWidth()
                .heightIn(min = PillHeight)
                .testTag("expiredHeader")
                .glass(c, RoundedCornerShape(26.dp), 8.dp)
                .pressable(shape = RoundedCornerShape(26.dp), onClick = onToggle)
                .padding(horizontal = 20.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText("Expired · ${ended.size}", RType.sans(15f, FontWeight.SemiBold), c.text, Modifier.weight(1f))
            Spacer(Modifier.width(8.dp))
            GlyphIcon(Glyph.ChevronRight, c.secondary, size = 15.dp, modifier = Modifier.graphicsLayer { rotationZ = -90f + 180f * progress })
        }
    }
}
