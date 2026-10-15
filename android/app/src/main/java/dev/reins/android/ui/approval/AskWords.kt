package dev.reins.android.ui.approval

import dev.reins.android.design.Field
import dev.reins.android.design.serviceName
import dev.reins.android.ui.common.askSentence
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView

// The ask on its field (Reins Signal), in words: which field, the label line, and the ask itself with its target. Built
// by the one formatter ([askSentence], ASK-TEXT.md).

/** The field of an ask: what kind of thing it is. Paying is red like a delete: money can't come back. */
fun askField(view: ApprovalView): Field = if (view.purchase != null) Field.Delete else heroTone(view).field

/** Paying and what can't be undone are held, not tapped. */
fun askHeld(view: ApprovalView): Boolean = view.purchase != null || destructive(view)

/**
 * The ask as one imperative sentence and its target, set in italic: "Push 8 commits to *octo/app*", "Read 3 emails
 * from *Stripe*". A question from the computer stays the question.
 */
fun askWords(view: ApprovalView): Pair<String, String?> = askSentence(view).let { it.text to it.target }

/** The label line over the ask: what it touches, then what it does ("GitHub · Push", "Terminal · Run"). */
fun askLabel(view: ApprovalView): String = askSentence(view).kind
