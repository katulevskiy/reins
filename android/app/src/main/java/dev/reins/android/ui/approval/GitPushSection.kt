package dev.reins.android.ui.approval

import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Card
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.android.ui.common.untrusted
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import dev.reins.core.GitPushView
import dev.reins.core.GitRefView

/**
 * A push from git on the user's computer, ref by ref: what each branch or tag gains or loses, which commits and files,
 * and a warning when history is rewritten (or could not be checked). Everything shown came from the desktop app.
 */
@Composable
internal fun GitPushSection(git: GitPushView, host: String = "GitHub") {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("gitPush"), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            RText(
                untrusted(git.repo),
                RType.mono(15f, FontWeight.SemiBold),
                c.text,
                Modifier.weight(1f).testTag("gitRepo"),
                maxLines = 1,
                ltr = true,
                overflow = TextOverflow.MiddleEllipsis,
            )
            Spacer(Modifier.width(12.dp))
            RText("${packSizeLabel(git.packBytes)} sent", RType.sans(12.5f), c.tertiary, Modifier.testTag("gitPack"), maxLines = 1)
        }
        git.refs.forEachIndexed { i, ref -> GitRefCard(i, ref, host) }
        git.notes.forEachIndexed { k, note ->
            RText(untrusted(note), RType.sans(13f, lineHeight = 18f), c.tertiary, Modifier.padding(horizontal = 4.dp).testTag("gitNote:$k"))
        }
    }
}

@Composable
private fun GitRefCard(i: Int, ref: GitRefView, host: String) {
    val c = LocalColors.current
    Card(Modifier.animateContentSize()) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                RText(
                    untrusted(ref.shortName),
                    RType.mono(17f, FontWeight.SemiBold),
                    c.text,
                    Modifier.weight(1f, fill = false).testTag("gitRef:$i"),
                    maxLines = 1,
                    ltr = true,
                    overflow = TextOverflow.MiddleEllipsis,
                )
                Spacer(Modifier.width(10.dp))
                Tag(gitChipLabel(ref), Modifier.testTag("gitChip:$i").semantics(mergeDescendants = true) {}, tint = chipTint(ref, c))
            }
            when (gitHistory(ref)) {
                GitHistory.Rewrites -> Warning("Rewrites history (force push)", BannerKind.Error, "forceWarning:$i")
                GitHistory.Unknown -> Warning("Could not check history — treat as a force push", BannerKind.Warning, "forceUnknown:$i")
                GitHistory.Adds -> Unit
            }
            if (ref.change == "delete") {
                RText(
                    if (ref.kind == "tag") "The tag is removed from $host." else "The branch is removed from $host.",
                    RType.sans(14f),
                    c.secondary,
                    Modifier.padding(top = 8.dp),
                )
            }
            if (ref.commits.isNotEmpty() || ref.commitCount > 0u) Commits(i, ref)
            gitTotalsLabel(ref)?.let { totals ->
                Hairline(Modifier.padding(top = 14.dp, bottom = 12.dp), inset = 0.dp)
                Files(i, ref, totals)
            }
        }
    }
}

@Composable
private fun Warning(text: String, kind: BannerKind, tag: String) {
    Box(Modifier.padding(top = 12.dp).testTag(tag).semantics(mergeDescendants = true) {}) { Banner(text, kind = kind) }
}

private fun chipTint(ref: GitRefView, c: RColors): Color? = when {
    ref.change == "delete" -> c.danger
    ref.change == "create" -> c.success
    ref.kind == "tag" -> c.accent
    else -> null
}

@Composable
private fun Commits(i: Int, ref: GitRefView) {
    val c = LocalColors.current
    var expanded by rememberSaveable(i) { mutableStateOf(false) }
    val count = ref.commitCount.toInt()
    RText(
        if (count == 1) "1 commit" else "$count commits",
        RType.sans(12.5f, FontWeight.Medium),
        c.tertiary,
        Modifier.padding(top = 14.dp, bottom = 4.dp),
    )
    val shown = if (expanded) ref.commits else ref.commits.take(GIT_COMMITS_SHOWN)
    shown.forEachIndexed { j, commit -> CommitRow(commit, Modifier.testTag("gitCommit:$i:$j")) }
    val hidden = ref.commits.size - shown.size
    if (hidden > 0) MoreLink("and $hidden more", "gitMoreCommits:$i") { expanded = true }
    val unlisted = count - ref.commits.size
    if (unlisted > 0) NotListed(unlisted, "gitCommitsNotListed:$i")
}

@Composable
private fun CommitRow(commit: GitCommitView, modifier: Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().semantics(mergeDescendants = true) {}.padding(vertical = 6.dp)) {
        RText(commit.shortSha, RType.mono(13f), c.secondary, Modifier.width(66.dp).padding(top = 1.dp), maxLines = 1, ltr = true)
        Column(Modifier.weight(1f)) {
            RText(untrusted(commit.subject).ifEmpty { "(no message)" }, RType.sans(14.5f, lineHeight = 19f), c.text, maxLines = 2)
            RText(untrusted(commit.author), RType.sans(12.5f), c.tertiary, Modifier.padding(top = 1.dp), maxLines = 1)
        }
    }
}

@Composable
private fun Files(i: Int, ref: GitRefView, totals: String) {
    val c = LocalColors.current
    var expanded by rememberSaveable(i) { mutableStateOf(false) }
    Counts(totals, RType.sans(14f, FontWeight.Medium), c.text, Modifier.testTag("gitTotals:$i").padding(bottom = 4.dp))
    val shown = if (expanded) ref.files else ref.files.take(GIT_FILES_SHOWN)
    shown.forEachIndexed { j, file -> FileRow(file, Modifier.testTag("gitFile:$i:$j")) }
    val hidden = ref.files.size - shown.size
    if (hidden > 0) MoreLink("and $hidden more", "gitMoreFiles:$i") { expanded = true }
    val unlisted = ref.filesChanged.toInt() - ref.files.size
    if (unlisted > 0) NotListed(unlisted, "gitFilesNotListed:$i")
}

@Composable
private fun FileRow(file: GitFileView, modifier: Modifier) {
    val c = LocalColors.current
    val letter = gitFileLetter(file.status)
    val tone = when (letter) {
        "A" -> c.success
        "D" -> c.danger
        "M" -> c.search
        "T" -> c.accent
        else -> c.secondary
    }
    Row(modifier.fillMaxWidth().semantics(mergeDescendants = true) {}.padding(vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
        RText(letter, RType.mono(13f, FontWeight.Bold), tone, Modifier.width(22.dp), maxLines = 1)
        RText(
            untrusted(file.path),
            RType.mono(13f),
            c.text,
            Modifier.weight(1f),
            maxLines = 1,
            ltr = true,
            overflow = TextOverflow.MiddleEllipsis,
        )
        val counts = gitFileCounts(file)
        if (counts.isNotEmpty()) {
            Spacer(Modifier.width(10.dp))
            if (file.binary) {
                RText(counts, RType.sans(12.5f), c.tertiary, maxLines = 1)
            } else {
                Counts(counts, RType.mono(12.5f), c.secondary)
            }
        }
    }
}

/** A line such as "+120 −14 in 12 files" with the additions in green and the deletions in red. */
@Composable
private fun Counts(text: String, style: TextStyle, color: Color, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val annotated = buildAnnotatedString {
        text.split(' ').forEachIndexed { k, word ->
            if (k > 0) append(' ')
            val tone = when {
                word.startsWith("+") -> c.success
                word.startsWith("−") -> c.danger
                else -> null
            }
            if (tone != null) withStyle(SpanStyle(color = tone)) { append(word) } else append(word)
        }
    }
    BasicText(annotated, modifier, style.copy(color = color), maxLines = 1, softWrap = false, overflow = TextOverflow.Ellipsis)
}

@Composable
private fun MoreLink(text: String, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(Modifier.testTag(tag).pressable(onClick = onClick).padding(vertical = 8.dp)) {
        RText(text, RType.sans(14f, FontWeight.Medium), c.accent)
    }
}

@Composable
private fun NotListed(n: Int, tag: String) {
    RText("$n more not listed", RType.sans(12.5f), LocalColors.current.tertiary, Modifier.padding(top = 4.dp).testTag(tag))
}
