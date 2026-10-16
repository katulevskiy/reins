package dev.reins.android.ui.approval

import dev.reins.core.Attention
import dev.reins.core.AttentionLevel
import dev.reins.core.GitRefView
import dev.reins.core.MessageView
import dev.reins.core.ReasonCode

// What deserves a closer look inside a push or a list of emails, in words and in order (the iOS app's
// AttentionText.swift). The core marks each commit, file and message; this only words, counts and sorts. Pure, tested
// in AttentionTest.

/** One row's mark: the labels on its chip and how closely to look. */
data class AttentionMark(val level: AttentionLevel, val labels: List<String>) {
    /** "Changes CI · Deletes a file". */
    val text: String get() = labels.joinToString(" · ")

    /** What TalkBack reads: "Look closer: Changes CI, Deletes a file". */
    val spoken: String get() = "${if (level == AttentionLevel.CLOSE) "Look closer" else "Worth a look"}: ${labels.joinToString(", ")}"
}

/**
 * A commit's or a file's mark: the core's labels as they are, less any [shown] apart on the same screen (the commit
 * page's own "Hidden characters" tag). `null` when nothing is left.
 */
fun attentionMark(attention: Attention?, shown: Set<ReasonCode> = emptySet()): AttentionMark? {
    attention ?: return null
    val labels = attention.reasons.filter { it.code !in shown }.map { it.label }
    return if (labels.isEmpty()) null else AttentionMark(attention.level, labels)
}

/**
 * An email's (or an item's) mark. A sign-in code or a password keeps the row's one word ("Code", "Password") instead of
 * a second chip for the same fact; a message the core calls sensitive is marked even without reasons.
 */
fun mailMark(m: MessageView, service: String): AttentionMark? {
    val held = if (service == "vault") "Password" else "Code"
    val labels = mutableListOf<String>()
    m.attention?.reasons?.forEach { reason ->
        val label = when (reason.code) {
            ReasonCode.SIGN_IN_CODE, ReasonCode.PASSWORD -> held
            else -> reason.label
        }
        if (label !in labels) labels += label
    }
    if (m.sensitive && held !in labels) labels.add(0, held)
    if (labels.isEmpty()) return null
    return AttentionMark(if (m.sensitive) AttentionLevel.CLOSE else m.attention?.level ?: AttentionLevel.LOOK, labels)
}

/** Close first, then Look, then the rest. */
private fun band(level: AttentionLevel?) = when (level) {
    AttentionLevel.CLOSE -> 0
    AttentionLevel.LOOK -> 1
    null -> 2
}

/**
 * The rows to show, flagged first (Close, then Look, then the rest), each band in its original order. Each keeps its
 * index in [items], which is what a row opens: the list itself is never reordered.
 */
fun <T> attentionOrder(items: List<T>, level: (T) -> AttentionLevel?): List<IndexedValue<T>> =
    items.withIndex().sortedWith(compareBy<IndexedValue<T>> { band(level(it.value)) }.thenBy { it.index })

/**
 * "2 of 8 to look at"; "2 of 50 shown to look at" when only [listed] of [total] came with the request (those not listed
 * were not looked at). `null` when nothing is flagged.
 */
fun attentionSummary(flagged: Int, listed: Int, total: Int): String? = when {
    flagged == 0 -> null
    listed < total -> "$flagged of $listed shown to look at"
    else -> "$flagged of $total to look at"
}

/** The summary on a ref's folded row: its commits, or its files when the commits are not listed. */
fun attentionSummary(ref: GitRefView): String? =
    if (ref.commits.isNotEmpty() || ref.commitCount > 0u) {
        attentionSummary(ref.commits.count { it.attention != null }, ref.commits.size, ref.commitCount.toInt())
    } else {
        attentionSummary(ref.files.count { it.attention != null }, ref.files.size, ref.filesChanged.toInt())
    }

/**
 * Whether a row shows its own "Hidden" tag: whenever it has hidden characters, unless its chip already names them. The
 * core keeps at most three reasons, so heavier ones can leave hidden characters out of the chip.
 */
fun hiddenApart(hidden: Boolean, attention: Attention?): Boolean =
    hidden && attention?.reasons?.any { it.code == ReasonCode.HIDDEN_CHARS } != true

/** "Rewrites history" on a force push, or one whose history could not be checked, flagged or not. */
fun rewritesHistory(ref: GitRefView): Boolean = ref.force || ref.forceUnknown

/** The emails' header: "12 found", "12 found · 2 to look at"; an Activity entry also says how many were shared. */
fun mailHeader(messages: List<MessageView>, service: String, shared: Int? = null): String {
    val flagged = messages.count { mailMark(it, service) != null }
    return listOfNotNull("${messages.size} found", shared?.let { "$it shared" }, if (flagged > 0) "$flagged to look at" else null).joinToString(" · ")
}
