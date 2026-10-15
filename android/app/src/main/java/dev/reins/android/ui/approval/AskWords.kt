package dev.reins.android.ui.approval

import dev.reins.android.design.Field
import dev.reins.android.design.serviceName
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView

// The ask on its field (Reins Signal), in words: which field, the label line, and the ask itself with its target. Built
// from the approval's own words (ApprovalWords), which stay as they are.

/** The field of an ask: what kind of thing it is. Paying is red like a delete: money can't come back. */
fun askField(view: ApprovalView): Field = if (view.purchase != null) Field.Delete else heroTone(view).field

/** Paying and what can't be undone are held, not tapped. */
fun askHeld(view: ApprovalView): Boolean = view.purchase != null || destructive(view)

/**
 * The ask as one imperative sentence and its target, set in italic: "Push 8 commits to *octo/app*", "Read 3 emails
 * from *Stripe*". A question from the computer stays the question.
 */
fun askWords(view: ApprovalView): Pair<String, String?> {
    val spans = heroSentence(view)
    val plain = spans.joinToString("") { it.text }.replace('\n', ' ').replace("  ", " ").trim()
    val text = when {
        view.ask != null -> plain
        plain.startsWith("Wants to ") -> plain.removePrefix("Wants to ")
        plain.startsWith("Wants ") -> "Hand over " + plain.removePrefix("Wants ")
        else -> plain
    }.replaceFirstChar { it.uppercase() }
    val bold = spans.filter { it.bold }
    val target = if (view.ask == null && bold.size >= 2) bold.last().text.trim().takeIf { it.isNotEmpty() && it in text } else null
    return text to target
}

/** The label line over the ask: what it touches, then what it does ("GitHub · Push", "Payments · Buy"). */
fun askLabel(view: ApprovalView): String {
    val mcp = view.mcp
    val where = when {
        mcp != null -> untrusted(mcp.serverName)
        view.purchase != null -> "Payments"
        view.ask != null -> "Computer"
        view.secrets != null || isSecretsMove(view) -> "Vault"
        view.ssh != null -> "SSH"
        else -> serviceName(view.service.ifBlank { "gmail" })
    }
    val what = when {
        mcp != null -> "MCP tool"
        view.purchase != null -> "Buy"
        view.ask != null -> "Asks"
        view.secrets != null || view.envRelease != null -> "Secrets"
        view.secretsStore != null || view.tokenStore != null -> "Save"
        view.ssh != null -> "Sign in"
        view.kind == ApprovalKind.GRANT -> "Permission"
        view.kind == ApprovalKind.ACCOUNTS -> "Accounts"
        deletes(view) -> "Delete"
        view.git != null -> "Push"
        view.kind == ApprovalKind.SEND -> "Send"
        view.kind == ApprovalKind.SEARCH -> "Search"
        view.kind == ApprovalKind.READ -> "Read"
        view.kind == ApprovalKind.FETCH -> "Fetch"
        else -> when (badge(view)) {
            Badge.Search -> "Search"
            Badge.Read -> "Read"
            Badge.List -> "List"
            Badge.Send -> "Send"
            Badge.Trash -> "Delete"
            Badge.Terminal -> "Run"
            else -> "Change"
        }
    }
    return "$where · $what"
}
