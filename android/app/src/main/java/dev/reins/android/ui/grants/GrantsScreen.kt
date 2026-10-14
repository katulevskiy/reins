package dev.reins.android.ui.grants

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.History
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.LargeTitle
import dev.reins.android.design.LocalColors
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.state.AppState
import dev.reins.android.ui.activity.RoundPlus
import dev.reins.android.ui.activity.SectionHeader
import dev.reins.android.ui.common.EndedGrantRow
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.settings.BottomSheet
import dev.reins.android.ui.settings.SettingsGroup
import dev.reins.android.ui.settings.SettingsRow
import dev.reins.core.GrantView
import dev.reins.core.StandingGrant
import kotlinx.coroutines.launch

/**
 * The second tab: the permissions that are running, each with its time left; the ones that ended folded into one
 * "Expired" row that opens their list; and what a newly connected AI may do.
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
    /** The starting rule for new AIs, changed at the end of the list. */
    onStartingPolicy: (dev.reins.core.StartingPolicy) -> Unit = {},
) {
    val c = LocalColors.current
    val grants by state.grants.collectAsStateWithLifecycle()
    val startingPolicy by state.startingPolicy.collectAsStateWithLifecycle()
    val running = remember(grants) { grants.filter { it.active }.sortedBy { it.expiresAt ?: Long.MAX_VALUE } }
    val ended = remember(grants) { grants.filterNot { it.active }.sortedByDescending { it.createdAt } }
    var resuming by remember { mutableStateOf<GrantView?>(null) }
    var deleting by remember { mutableStateOf<GrantView?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    Box(Modifier.fillMaxSize().background(c.background)) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 130.dp)) {
            item(key = "title") {
                LargeTitle("Grants") { RoundPlus("newGrant", "New grant", onNew) }
            }
            error?.let {
                item(key = "error") { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 6.dp), BannerKind.Error, tag = "resumeError") }
            }
            if (running.isEmpty()) {
                item(key = "empty") {
                    EmptyState(Glyph.Key, if (grants.isEmpty()) "No grants" else "No active grants", null, tag = "noGrants")
                }
            } else {
                item(key = "active") { SectionHeader("Active · ${running.size}") }
            }
            items(running, key = { "g:${it.id}" }, contentType = { "grant" }) { grant ->
                GrantCard(grant, Modifier.padding(start = 12.dp, end = 12.dp, bottom = 10.dp).testTag("grant:${grant.id}")) { onOpen(grant.id) }
            }
            if (ended.isNotEmpty()) {
                item(key = "expired") {
                    SettingsGroup(gap = 4.dp) {
                        SettingsRow(
                            "Expired",
                            Modifier.testTag("expiredHeader"),
                            icon = Icons.Rounded.History,
                            tint = c.tertiary,
                            value = ended.size.toString(),
                            onClick = feedbackAction(Event.expand(!expiredOpen), onToggleExpired),
                        )
                    }
                }
            }
            item(key = "startingRule") {
                StartingRuleChooser(startingPolicy, Modifier.padding(top = 8.dp).testTag("startingRule"), onStartingPolicy)
            }
        }
        BottomSheet(expiredOpen, "Expired", onClose = onToggleExpired, tag = "expiredList") {
            Column(Modifier.padding(bottom = 8.dp)) {
                ended.forEach { grant ->
                    // The tag on the row itself: the sheet takes the taps around it, which would swallow a tag around it.
                    EndedGrantRow(grant, Modifier.testTag("ended:${grant.id}"), onOpen = { onOpen(grant.id) }, onResume = { resuming = grant }, onDelete = { deleting = grant })
                }
            }
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
