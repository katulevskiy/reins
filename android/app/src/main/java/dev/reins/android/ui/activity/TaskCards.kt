package dev.reins.android.ui.activity

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Stop
import androidx.compose.material.icons.rounded.TaskAlt
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.reins.android.AppContainer
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Field
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.pane
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AuthResult
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.OwnerPrompt
import dev.reins.android.ui.autopilot.Segmented
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedName
import dev.reins.android.ui.common.userMessage
import dev.reins.core.PendingItem
import dev.reins.core.TaskView
import kotlin.coroutines.cancellation.CancellationException

// Tasks (Decisions v2, 01 §12): an envelope inside which Auto may do more for one agent, up to a ceiling, for a while.
// An agent proposes one; the owner sees the envelope as chips and may only narrow it before approving (owner check).
// What the agent says is shown as the agent's words and never changes anything. A live task shows as a chip with Stop.

/** The lists of an envelope the owner can narrow, in the card's order. */
enum class TaskField(val label: String) {
    Repos("Repos"),
    Branches("Branches"),
    Folders("Folders"),
    People("People"),
    Kinds("Kinds"),
    ;

    fun of(t: TaskView): List<String> = when (this) {
        Repos -> t.repos
        Branches -> t.branches
        Folders -> t.paths
        People -> t.people
        Kinds -> t.operations
    }

    /**
     * Whether removing [value] narrows. An empty list bounds nothing (any repo, any kind), so the last entry of every
     * list but People stays: removing it would widen the task. People empty means nobody.
     */
    fun removable(t: TaskView): Boolean = this == People || of(t).size > 1
}

/** [t] without [value] in [field], if that narrows it (see [TaskField.removable]); else [t] as it was. */
fun TaskView.without(field: TaskField, value: String): TaskView {
    if (!field.removable(this)) return this
    val rest = field.of(this).filter { it != value }
    return when (field) {
        TaskField.Repos -> copy(repos = rest)
        TaskField.Branches -> copy(branches = rest)
        TaskField.Folders -> copy(paths = rest)
        TaskField.People -> copy(people = rest)
        TaskField.Kinds -> copy(operations = rest)
    }
}

/** "Up to: Recoverable": the highest level a task makes Auto. */
fun ceilingLabel(level: UByte): String = when (level.toInt()) {
    0 -> "Undoable"
    1 -> "Recoverable"
    else -> "Reaches people"
}

/** The ceilings the owner may pick: the proposed one and lower (never higher, never L3). */
fun ceilingChoices(proposed: UByte): List<UByte> = (0..minOf(proposed.toInt(), 2)).map { it.toUByte() }

private val LENGTHS = listOf(15L * 60, 3_600L, 4 * 3_600L, 8 * 3_600L, 24 * 3_600L)

/** The lengths the owner may pick (in seconds): shorter ones, and the proposed one. */
fun lengthChoices(proposed: Long): List<Long> = (LENGTHS.filter { it < proposed } + proposed.coerceAtLeast(60)).distinct().sorted()

fun lengthLabel(seconds: Long): String = when {
    seconds < 3_600 -> "${(seconds + 59) / 60} min"
    seconds % 3_600 == 0L -> "${seconds / 3_600} h"
    else -> "${seconds / 3_600} h ${(seconds % 3_600) / 60} min"
}

/** The budgets the owner may pick: the proposed one and smaller; none when the proposal has no limit. */
fun budgetChoices(proposed: UInt?): List<UInt> {
    proposed ?: return emptyList()
    return listOf(proposed, proposed / 2u, proposed / 4u).filter { it >= 1u }.distinct().sorted()
}

/** The proposals waiting for the owner, newest last as the core lists them. */
fun taskProposals(pending: List<PendingItem>): List<PendingItem> = pending.filter { it.taskProposal != null }

/** Live tasks: not ended, and not past their end ([nowSeconds]). */
fun liveTasks(tasks: List<TaskView>, nowSeconds: Long): List<TaskView> = tasks.filter { it.endedAt == null && it.endsAt > nowSeconds }

/** Tasks that ended in the last day, newest first. */
fun endedTasks(tasks: List<TaskView>, nowSeconds: Long): List<TaskView> =
    tasks.filter { t -> (t.endedAt ?: t.endsAt.takeIf { it <= nowSeconds })?.let { nowSeconds - it < 86_400 } == true }
        .sortedByDescending { it.endedAt ?: it.endsAt }

/** "Fix CI ended · 12 auto · 2 asked · 1 parked". */
fun endedLine(t: TaskView): String = "${untrusted(t.title)} ended · ${t.auto} auto · ${t.asked} asked · ${t.parked} parked"

// ---- cards -----------------------------------------------------------------------------------------------------

/** Each proposal as a card the owner narrows, then approves or denies. */
@Composable
fun TaskProposalCards(
    proposals: List<PendingItem>,
    onApprove: (PendingItem, TaskView) -> Unit,
    onDeny: (PendingItem) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (proposals.isEmpty()) return
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("taskProposals"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        proposals.forEach { item -> TaskProposalCard(item, { onApprove(item, it) }, { onDeny(item) }) }
    }
}

/**
 * One proposal in the waiting cards' shell: "Task" and the agent, its title, what the agent says (as the agent's), the
 * envelope as removable chips, "Up to", how long and how many uses (each only lower), then Deny and Approve.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun TaskProposalCard(item: PendingItem, onApprove: (TaskView) -> Unit, onDeny: () -> Unit) {
    val proposal = item.taskProposal ?: return
    val c = LocalColors.current
    val shape = RoundedCornerShape(26.dp)
    var draft by remember(item.id, proposal) { mutableStateOf(proposal) }
    Column(
        Modifier
            .fillMaxWidth()
            .fieldSurface(Field.Grant, shape)
            .clip(shape)
            .testTag("taskProposal:${item.id}")
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(Icons.Rounded.TaskAlt, null, Modifier.size(18.dp), tint = c.onField)
            RText("Task", RType.sans(14f, FontWeight.SemiBold), c.onField, maxLines = 1)
            RText(untrustedName(item.connectionLabel), RType.sans(13.5f), c.onField2, Modifier.weight(1f), maxLines = 1)
        }
        RText(untrusted(proposal.title), RType.askSmall, c.onField, Modifier.semantics { heading() }.testTag("taskTitle:${item.id}"), maxLines = 3)
        proposal.reason?.takeIf { it.isNotBlank() }?.let { AgentSays(it, Modifier.testTag("taskReason:${item.id}")) }
        TaskField.entries.forEach { field ->
            val values = field.of(draft)
            if (values.isEmpty() && field != TaskField.People) return@forEach
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                RText(field.label, RType.sans(12f, FontWeight.SemiBold, tracking = 0.02f), c.onField2, maxLines = 1)
                if (values.isEmpty()) {
                    RText("Nobody", RType.sans(13.5f), c.onField2, Modifier.testTag("taskNobody:${item.id}"), maxLines = 1)
                } else {
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        values.forEach { value ->
                            EnvelopeChip(value, removable = field.removable(draft), tag = "${field.name}:$value") { draft = draft.without(field, value) }
                        }
                    }
                }
            }
        }
        Labelled("Up to") {
            Segmented(
                ceilingChoices(proposal.ceiling),
                draft.ceiling,
                ::ceilingLabel,
                Modifier.testTag("taskCeiling:${item.id}"),
                tag = { "taskCeiling:${item.id}:$it" },
            ) { draft = draft.copy(ceiling = it) }
        }
        val proposedLength = proposal.endsAt - proposal.startedAt
        val lengths = lengthChoices(proposedLength)
        if (lengths.size > 1) {
            Labelled("For") {
                Segmented(
                    lengths,
                    draft.endsAt - draft.startedAt,
                    ::lengthLabel,
                    Modifier.testTag("taskLength:${item.id}"),
                    tag = { "taskLength:${item.id}:$it" },
                ) { draft = draft.copy(endsAt = draft.startedAt + it) }
            }
        }
        val budgets = budgetChoices(proposal.budget)
        if (budgets.size > 1) {
            Labelled("Uses") {
                Segmented(
                    budgets,
                    draft.budget ?: budgets.last(),
                    { "$it" },
                    Modifier.testTag("taskBudget:${item.id}"),
                    tag = { "taskBudget:${item.id}:$it" },
                ) { draft = draft.copy(budget = it, usesLeft = it) }
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            SignalButton("Deny", Modifier.weight(1f).testTag("taskDeny:${item.id}"), style = ButtonStyle.OnFieldWash, size = ButtonSize.Small) { onDeny() }
            SignalButton("Approve", Modifier.weight(1f).testTag("taskApprove:${item.id}"), style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small) { onApprove(draft) }
        }
    }
}

@Composable
private fun Labelled(label: String, content: @Composable () -> Unit) {
    val c = LocalColors.current
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        RText(label, RType.sans(12f, FontWeight.SemiBold, tracking = 0.02f), c.onField2, maxLines = 1)
        content()
    }
}

/** One entry of the envelope, in mono (it comes from the agent), with a remove button when removing narrows. */
@Composable
private fun EnvelopeChip(value: String, removable: Boolean, tag: String, onRemove: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier.clip(CircleShape).background(c.onField.copy(alpha = 0.14f)).padding(start = 10.dp, end = if (removable) 4.dp else 10.dp, top = 4.dp, bottom = 4.dp)
            .testTag("taskChip:$tag"),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(untrusted(value), RType.mono(12.5f), c.onField, maxLines = 1)
        if (removable) {
            Icon(
                Icons.Rounded.Close,
                null,
                Modifier
                    .size(22.dp)
                    .clip(CircleShape)
                    .pressable(shape = CircleShape, label = "Remove", onClick = onRemove)
                    .semantics { contentDescription = "Remove ${untrusted(value)}" }
                    .testTag("taskChipRemove:$tag")
                    .padding(3.dp),
                tint = c.onField2,
            )
        }
    }
}

/** The live tasks as chips with Stop, then the ones that ended today as quiet rows. */
@Composable
fun TaskChips(tasks: List<TaskView>, nowSeconds: Long, onStop: (TaskView) -> Unit, modifier: Modifier = Modifier) {
    val live = liveTasks(tasks, nowSeconds)
    val ended = endedTasks(tasks, nowSeconds)
    if (live.isEmpty() && ended.isEmpty()) return
    val c = LocalColors.current
    Column(modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("taskChips"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        live.forEach { t ->
            Row(
                Modifier.fillMaxWidth().pane().padding(start = 14.dp, end = 8.dp, top = 6.dp, bottom = 6.dp).testTag("task:${t.id}"),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.TaskAlt, null, Modifier.size(18.dp), tint = c.violet)
                Column(Modifier.weight(1f)) {
                    RText("Task · ${untrusted(t.title)}", RType.sans(14f, FontWeight.SemiBold), c.ink, maxLines = 1)
                    RText(untrustedName(t.connectionLabel), RType.sans(12.5f), c.inkMeta, maxLines = 1)
                }
                SignalButton(
                    "Stop",
                    Modifier.testTag("taskStop:${t.id}"),
                    style = ButtonStyle.Soft,
                    size = ButtonSize.Compact,
                    leading = { Icon(Icons.Rounded.Stop, null, Modifier.size(16.dp), tint = c.ink) },
                ) { onStop(t) }
            }
        }
        ended.forEach { t ->
            RText(endedLine(t), RType.sans(13f), c.inkMeta, Modifier.padding(horizontal = 14.dp).testTag("taskEnded:${t.id}"), maxLines = 1)
        }
    }
}

// ---- actions ---------------------------------------------------------------------------------------------------

/**
 * Approves a proposal as the owner narrowed it ([draft]). A task loosens, so the core asks the owner first
 * ([OwnerPrompt]). The card leaves at once and comes back when it did not land. Returns what to say, or null.
 */
suspend fun approveTaskProposal(container: AppContainer, authenticator: Authenticator, item: PendingItem, draft: TaskView): String? {
    val owner = OwnerPrompt(authenticator, "Approve task")
    container.state.removePending(item.id)
    return try {
        container.core.approveTask(item.id, draft, owner)
        container.feedback.play(Event.Approved)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        container.state.restorePending(item.id)
        when (owner.last) {
            AuthResult.Cancelled -> null
            AuthResult.Unavailable -> "Set a screen lock or fingerprint on this phone first."
            else -> {
                container.feedback.play(Event.Error)
                e.userMessage()
            }
        }
    } finally {
        container.refreshPending()
        container.refreshTasks()
    }
}

/** Denies a proposal like any request. */
fun denyTaskProposal(container: AppContainer, item: PendingItem) {
    container.feedback.play(Event.Denied)
    container.answerInBackground(item.id, "Not denied") { container.core.deny(item.id) }
}

/** Stop: ends the task now. It only takes away, so nothing is asked. Returns what to say, or null. */
suspend fun stopTask(container: AppContainer, task: TaskView): String? {
    val now = System.currentTimeMillis() / 1000
    container.state.setTasks(container.state.tasks.value.map { if (it.id == task.id) it.copy(endedAt = now) else it })
    return try {
        container.core.endTask(task.id)
        container.feedback.play(Event.Selection)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        container.feedback.play(Event.Error)
        "Not stopped: ${e.userMessage()}"
    } finally {
        container.refreshTasks()
    }
}

/**
 * "Approve all" for one agent's task: after one owner's check, the core approves what is batchable (soft, below L3);
 * the rest stays to be answered one by one. Returns what to say, or null.
 */
suspend fun approveAllForTask(container: AppContainer, authenticator: Authenticator, group: AgentTickets): String? {
    val ids = group.batchable.map { it.id }
    if (ids.isEmpty()) return null
    when (authenticator.authenticate(if (ids.size == 1) "Approve request" else "Approve ${ids.size} requests", group.taskTitle ?: group.label)) {
        AuthResult.Success -> Unit
        AuthResult.Cancelled -> return null
        AuthResult.Unavailable -> {
            container.feedback.play(Event.Error)
            return "Set a screen lock or fingerprint on this phone to approve."
        }
    }
    container.feedback.play(Event.Approved)
    ids.forEach(container.state::removePending)
    return try {
        container.core.approveAll(group.connectionId, group.taskId)
        null
    } catch (e: CancellationException) {
        throw e
    } catch (e: Exception) {
        ids.forEach(container.state::restorePending)
        container.feedback.play(Event.Error)
        "Not approved: ${e.userMessage()}"
    } finally {
        container.refreshPending()
    }
}
