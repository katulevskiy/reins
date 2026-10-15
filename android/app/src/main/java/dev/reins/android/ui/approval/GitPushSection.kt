package dev.reins.android.ui.approval

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Description
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.common.untrustedLine
import dev.reins.core.ApprovalView
import dev.reins.core.DiffLineKind
import dev.reins.core.DiffLineView
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import dev.reins.core.GitPushView
import dev.reins.core.GitRefView

private const val HIDDEN_HELP = "Invisible characters can make code read unlike what it does. They show as � here."

private fun GitRefView.hiddenChars() = commits.any { it.hiddenChars } || files.any { it.hiddenChars }

/**
 * A push from git on the user's computer, as facts: each branch or tag, its commits folded into a row that opens them
 * (with their lines added and removed), and the account. Everything shown came from the desktop app.
 */
@Composable
internal fun GitFacts(view: ApprovalView, git: GitPushView) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    Column(Modifier.testTag("gitPush"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        FactGroup(
            rows = buildList<@Composable () -> Unit> {
                git.refs.forEachIndexed { i, ref ->
                    add {
                        FactRow(
                            if (ref.kind == "tag") "Tag" else "Branch",
                            untrusted(ref.shortName),
                            mono = true,
                            valueTag = "gitRef:$i",
                            trailing = if (ref.change == "update" && ref.kind != "tag") null else {
                                { RefChip(gitChipLabel(ref), ref.change, ref.kind, Modifier.testTag("gitChip:$i")) }
                            },
                        )
                    }
                }
                view.account?.takeIf { it.isNotBlank() }?.let { add { FactRow("Account", untrusted(it), mono = true) } }
                git.refs.forEachIndexed { i, ref ->
                    when {
                        ref.commits.isNotEmpty() || ref.commitCount > 0u -> add {
                            DrillRow(commitsWord(ref.commitCount.toInt()), "gitCommits:$i", { nav.push(ApprovalPage.Commits(i)) }) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    if (ref.hiddenChars()) {
                                        WarnTag("Hidden", c.danger, small = true)
                                        Spacer(Modifier.width(8.dp))
                                    }
                                    Counts(ref.additions, ref.deletions, modifier = Modifier.testTag("gitTotals:$i"))
                                }
                            }
                        }
                        ref.filesChanged > 0u -> add {
                            DrillRow(filesWord(ref.filesChanged.toInt()), "gitFiles:$i", { nav.push(ApprovalPage.Files(i)) }) {
                                Counts(ref.additions, ref.deletions, modifier = Modifier.testTag("gitTotals:$i"))
                            }
                        }
                    }
                }
            },
        )
        git.notes.forEachIndexed { k, note ->
            RText(untrusted(note), RType.sans(12.5f, lineHeight = 17f), c.tertiary, Modifier.padding(horizontal = 24.dp).testTag("gitNote:$k"))
        }
    }
}

@Composable
internal fun RefChip(text: String, change: String, kind: String, modifier: Modifier) {
    val c = LocalColors.current
    val tint = when {
        change == "delete" -> c.danger
        change == "create" -> c.success
        kind == "tag" -> c.accent
        else -> c.secondary
    }
    RText(
        text,
        RType.sans(12f, FontWeight.SemiBold),
        tint,
        modifier.clip(CircleShape).background(tint.copy(alpha = 0.12f)).padding(horizontal = 9.dp, vertical = 4.dp).semantics(mergeDescendants = true) {},
        maxLines = 1,
    )
}

/** The commits a ref gains, newest first, each with its own lines added and removed. */
@Composable
internal fun CommitsPage(view: ApprovalView, refIndex: Int) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val ref = view.git?.refs?.getOrNull(refIndex) ?: return
    Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 22.dp, top = 2.dp, bottom = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        RText(untrusted(ref.shortName), RType.mono(13f), c.secondary, Modifier.weight(1f, fill = false), maxLines = 1, ltr = true)
        if (ref.filesChanged > 0u) {
            RText(
                " · ${filesWord(ref.filesChanged.toInt())}",
                RType.sans(13f),
                c.tertiary,
                Modifier.pressable(shape = RoundedCornerShape(6.dp)) { nav.push(ApprovalPage.Files(refIndex)) }.testTag("gitFiles:$refIndex"),
                maxLines = 1,
            )
        }
        Spacer(Modifier.weight(1f))
        Counts(ref.additions, ref.deletions, 13f)
    }
    FactGroup(
        rows = buildList<@Composable () -> Unit> {
            ref.commits.forEachIndexed { j, commit ->
                add { CommitRow(commit, Modifier.testTag("gitCommit:$refIndex:$j")) { nav.push(ApprovalPage.Commit(refIndex, j)) } }
            }
            val unlisted = ref.commitCount.toInt() - ref.commits.size
            if (unlisted > 0) add { NotListed("$unlisted more not listed", "gitCommitsNotListed:$refIndex") }
        },
    )
}

@Composable
private fun CommitRow(commit: GitCommitView, modifier: Modifier, onOpen: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val opens = commit.filesChanged != null
    Row(
        modifier
            .fillMaxWidth()
            .then(
                if (opens) {
                    Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) {
                        onOpen()
                        feedback.play(Event.Open)
                    }
                } else {
                    Modifier
                },
            )
            .padding(top = 12.dp, bottom = 12.dp)
            .semantics(mergeDescendants = true) {},
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText(commit.shortSha, RType.mono(12.5f), c.tertiary, Modifier.width(66.dp).align(Alignment.Top).padding(top = 2.dp), maxLines = 1, ltr = true)
        Column(Modifier.weight(1f)) {
            RText(untrusted(commit.subject).ifEmpty { "(no message)" }, RType.sans(15f, lineHeight = 19.5f), c.text, maxLines = 2)
            Row(Modifier.padding(top = 3.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                RText(untrusted(authorName(commit.author)), RType.sans(12.5f), c.tertiary, Modifier.weight(1f, fill = false), maxLines = 1)
                Counts(commit.additions, commit.deletions, 12f)
                if (commit.hiddenChars) WarnTag("Hidden", c.danger, Modifier.testTag("commitHidden"), small = true)
            }
        }
        if (opens) {
            Spacer(Modifier.width(6.dp))
            Icon(Icons.Rounded.ChevronRight, null, Modifier.size(20.dp), tint = c.tertiary)
        }
    }
}

/** One commit: its message, who and when, and each file it changes, which opens to its lines. */
@Composable
internal fun CommitPage(view: ApprovalView, refIndex: Int, index: Int) {
    val c = LocalColors.current
    val commit = view.git?.refs?.getOrNull(refIndex)?.commits?.getOrNull(index) ?: return
    Column(Modifier.fillMaxWidth().padding(start = 22.dp, end = 22.dp, top = 6.dp, bottom = 14.dp)) {
        RText(untrusted(commit.subject).ifEmpty { "(no message)" }, RType.sans(21f, FontWeight.SemiBold, lineHeight = 26f), c.text, Modifier.testTag("commitSubject"))
        Row(Modifier.padding(top = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            val meta = listOfNotNull(untrusted(authorName(commit.author)), if (commit.date != 0L) relativeTime(commit.date) else null).joinToString(" · ")
            RText(meta, RType.sans(13f), c.tertiary, Modifier.weight(1f, fill = false), maxLines = 1)
            if (commit.additions != null || commit.deletions != null) {
                RText("·", RType.sans(13f), c.tertiary)
                Counts(commit.additions, commit.deletions, 12.5f)
            }
        }
        if (commit.hiddenChars) {
            Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                WarnTag("Hidden characters", c.danger, Modifier.testTag("hiddenChars"))
                Spacer(Modifier.width(8.dp))
                HelpDot("hiddenCommit", HIDDEN_HELP)
            }
        }
    }
    val files = commit.files
    if (commit.filesChanged == null) {
        FactGroup(rows = listOf { NotListed("Files not compared", "commitFilesUnknown") })
        return
    }
    FactGroup(
        rows = buildList<@Composable () -> Unit> {
            files.forEachIndexed { j, file -> add { FileRow(file, "commitFile:$j", expandable = true) } }
            val unlisted = commit.filesChanged!!.toInt() - files.size
            if (unlisted > 0) add { NotListed("$unlisted more not listed", "commitFilesNotListed") }
        },
    )
}

/** Every file a ref changes, with its lines added and removed. */
@Composable
internal fun FilesPage(view: ApprovalView, refIndex: Int) {
    val ref = view.git?.refs?.getOrNull(refIndex) ?: return
    FactGroup(
        modifier = Modifier.padding(top = 6.dp),
        rows = buildList<@Composable () -> Unit> {
            ref.files.forEachIndexed { j, file -> add { FileRow(file, "gitFile:$refIndex:$j", expandable = false) } }
            val unlisted = ref.filesChanged.toInt() - ref.files.size
            if (unlisted > 0) add { NotListed("$unlisted more not listed", "gitFilesNotListed:$refIndex") }
        },
    )
}

@Composable
private fun NotListed(text: String, tag: String) {
    RText(text, RType.sans(13.5f), LocalColors.current.tertiary, Modifier.fillMaxWidth().padding(vertical = 13.dp).testTag(tag))
}

/** A file: its path (the folder dimmed), lines added and removed; a commit's file opens to its diff. */
@Composable
private fun FileRow(file: GitFileView, tag: String, expandable: Boolean) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by rememberSaveable(tag) { mutableStateOf(false) }
    val opens = expandable && file.diff.isNotEmpty()
    val (dir, name) = splitPath(untrusted(file.path))
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .then(
                    if (opens) {
                        Modifier.pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp)) {
                            open = !open
                            feedback.play(Event.expand(open))
                        }
                    } else {
                        Modifier
                    },
                )
                .padding(top = 12.dp, bottom = 12.dp)
                .testTag(tag)
                .semantics(mergeDescendants = true) {},
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Rounded.Description, null, Modifier.size(16.dp), tint = c.secondary)
            Spacer(Modifier.width(10.dp))
            BasicText(
                buildAnnotatedString {
                    withStyle(SpanStyle(color = c.tertiary)) { append(dir) }
                    append(name)
                },
                Modifier.weight(1f),
                style = RType.mono(13f).copy(color = c.text),
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Spacer(Modifier.width(8.dp))
            if (file.hiddenChars) {
                WarnTag("Hidden", c.danger, Modifier.testTag("$tag:hidden"), small = true)
                Spacer(Modifier.width(6.dp))
            }
            if (file.binary) {
                RText("binary", RType.sans(12.5f), c.tertiary, maxLines = 1)
            } else {
                Counts(file.additions?.toULong(), file.deletions?.toULong(), 12.5f)
            }
            if (opens) {
                Spacer(Modifier.width(6.dp))
                Icon(if (open) Icons.Rounded.KeyboardArrowDown else Icons.Rounded.ChevronRight, null, Modifier.size(18.dp), tint = c.tertiary)
            }
        }
        if (open && opens) DiffBlock(file.diff, file.diffCut, Modifier.testTag("$tag:diff"))
    }
}

/** A file's change line by line: hunk headers, numbers, additions in green and deletions in red. */
@Composable
private fun DiffBlock(lines: List<DiffLineView>, cut: Boolean, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val dark = c.dark
    Column(
        modifier
            .fillMaxWidth()
            .background(if (dark) Color(0xFF0B0B0D) else Color(0xFFF6F6F8))
            .horizontalScroll(rememberScrollState())
            .width(IntrinsicSize.Max)
            .padding(vertical = 2.dp),
    ) {
        val style = RType.mono(11.5f).copy(lineHeight = androidx.compose.ui.unit.TextUnit(20f, androidx.compose.ui.unit.TextUnitType.Sp))
        lines.forEach { line ->
            when (line.kind) {
                DiffLineKind.HUNK -> RText(
                    untrustedLine(line.text),
                    style,
                    if (dark) Color(0xFF7C88B8) else Color(0xFF4A5A99),
                    Modifier.fillMaxWidth().background(c.search.copy(alpha = 0.06f)).padding(start = 12.dp, end = 12.dp),
                    maxLines = 1,
                    ltr = true,
                )
                DiffLineKind.NOTE -> RText(untrustedLine(line.text), style, c.tertiary, Modifier.padding(start = 46.dp, end = 12.dp), maxLines = 1, ltr = true)
                else -> {
                    val (bg, fg, num) = when (line.kind) {
                        DiffLineKind.ADD -> Triple(c.success.copy(alpha = 0.10f), if (dark) Color(0xFF9FF0CF) else Color(0xFF0F6A3F), if (dark) Color(0xFF2F7F5F) else c.success)
                        DiffLineKind.DELETE -> Triple(c.danger.copy(alpha = 0.10f), if (dark) Color(0xFFFDB4B4) else Color(0xFF9B1C1C), if (dark) Color(0xFF8A3A3A) else c.danger)
                        else -> Triple(Color.Transparent, c.secondary, if (dark) Color(0xFF4A4A52) else c.tertiary)
                    }
                    val sign = when (line.kind) {
                        DiffLineKind.ADD -> "+"
                        DiffLineKind.DELETE -> "−"
                        else -> " "
                    }
                    Row(Modifier.fillMaxWidth().background(bg)) {
                        RText(
                            (line.newLine ?: line.oldLine)?.toString().orEmpty(),
                            style,
                            num,
                            Modifier.widthIn(min = 34.dp).padding(end = 8.dp),
                            maxLines = 1,
                            align = TextAlign.End,
                        )
                        RText("$sign ${untrustedLine(line.text)}", style, fg, Modifier.padding(end = 14.dp), maxLines = 1, ltr = true)
                    }
                }
            }
        }
        if (cut) {
            RText("More changes not shown", RType.sans(12.5f), c.tertiary, Modifier.padding(horizontal = 14.dp, vertical = 8.dp).testTag("diffCut"))
        }
    }
}
