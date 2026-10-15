package dev.reins.android.ui.common

import dev.reins.core.GitPushView

/**
 * A push as an Activity entry kept it in words, for an entry whose approval is not on this phone (decided on another
 * phone and synced, or too large to keep): the branches and the commits listed, nothing more. Lines added and removed,
 * files and diffs are not in it, so they are not shown (never made up).
 *
 * The core writes it at the top of the entry's full request (`titles::push_text`), then a blank line:
 *
 *     git push katulevskiy/reins
 *     create branch signal/desktop-release: 3 commits
 *       1a2b3c4 Subject of the newest commit
 *     delete tag v0.1
 *
 * Anything else, or a block the core cut short (no blank line after it), reads as no push at all.
 */
data class KeptPush(val repo: String, val refs: List<KeptRef>)

/** A branch or tag the push changed: [change] is "create", "update" or "delete"; [kind] "branch", "tag" or "other". */
data class KeptRef(
    val change: String,
    val kind: String,
    val name: String,
    /** Commits the ref gained; may be more than [commits] lists. */
    val commitCount: Int,
    val force: Boolean,
    val commits: List<KeptCommit>,
)

/** [hidden]: the subject held characters that change how text reads (the core shows each as U+FFFD). */
data class KeptCommit(val sha: String, val subject: String, val hidden: Boolean)

private val PUSH_HEAD = Regex("^git push (\\S+)$")
private val REF_LINE = Regex("^(create|update|delete) (branch|tag|other) (\\S+?)(?:: (\\d+) commits?)?(, force)?$")
private val COMMIT_LINE = Regex("^ {2}([0-9a-f]{7,40}) (.*)$")

/** The push in [full] (an entry's full request), or null when it holds none. */
fun keptPush(full: String?): KeptPush? {
    val lines = untrustedLine(full ?: return null).lines()
    val repo = PUSH_HEAD.find(lines.firstOrNull()?.trimEnd() ?: return null)?.groupValues?.get(1) ?: return null
    val end = lines.indexOfFirst { it.isBlank() }
    if (end < 0) return null
    val refs = mutableListOf<KeptRef>()
    for (line in lines.subList(1, end)) {
        val ref = REF_LINE.find(line.trimEnd())
        val commit = COMMIT_LINE.find(line.trimEnd())
        when {
            ref != null -> {
                val (change, kind, name, count, force) = ref.destructured
                if (change != "delete" && count.isEmpty()) return null
                refs += KeptRef(change, kind, name, count.toIntOrNull() ?: 0, force.isNotEmpty(), emptyList())
            }
            commit != null && refs.isNotEmpty() && refs.last().change != "delete" -> {
                val subject = untrusted(commit.groupValues[2])
                val last = refs.removeAt(refs.lastIndex)
                refs += last.copy(commits = last.commits + KeptCommit(commit.groupValues[1].take(7), subject, '�' in subject))
            }
            else -> return null
        }
    }
    return if (refs.isEmpty()) null else KeptPush(repo, refs)
}

/** One ref for the mono line under a push's sentence. */
internal data class RefLine(val name: String, val change: String, val kind: String)

internal fun refLines(git: GitPushView?): List<RefLine> = git?.refs.orEmpty().map { RefLine(it.shortName, it.change, it.kind) }

internal fun refLines(kept: KeptPush?): List<RefLine> = kept?.refs.orEmpty().map { RefLine(it.name, it.change, it.kind) }
