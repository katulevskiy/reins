package dev.reins.android.ui.grants

import dev.reins.core.ApprovalKind
import dev.reins.core.GrantScopeChoice
import dev.reins.core.StandingGrant

enum class NewGrantLifetime(val label: String, val seconds: Long?) {
    ONE_TIME("One time", null),
    HOUR("1 hour", 3_600),
    DAY("24 hours", 86_400),
    WEEK("7 days", 604_800),
    MONTH("30 days", 2_592_000),
}

/** What the "New grant" form holds. */
data class NewGrantDraft(
    val connectionId: String? = null,
    /** Which connected Gmail account the permission is for. */
    val account: String? = null,
    val send: Boolean = false,
    /** Read only: every email (needs a time limit of at most 7 days). */
    val anyMail: Boolean = false,
    /** Addresses (`a@b.com`) and domains (`@b.com` or `b.com`), separated by commas, spaces or lines. */
    val partiesText: String = "",
    val subject: String = "",
    val lifetime: NewGrantLifetime = NewGrantLifetime.DAY,
)

sealed interface NewGrantResult {
    data class Ok(val connectionId: String, val account: String, val kind: ApprovalKind, val standing: StandingGrant) : NewGrantResult

    data class Invalid(val message: String) : NewGrantResult
}

/** Splits the text into addresses and domains, dropping blanks and duplicates. */
fun parseParties(text: String): Pair<List<String>, List<String>> {
    val tokens = text.split(',', ';', ' ', '\n', '\t').map { it.trim().lowercase() }.filter { it.isNotEmpty() }.distinct()
    val domains = tokens.filter { it.startsWith("@") || !it.contains('@') }.map { it.removePrefix("@") }
    val addresses = tokens.filter { !it.startsWith("@") && it.contains('@') }
    return addresses to domains
}

fun buildNewGrant(draft: NewGrantDraft): NewGrantResult {
    val connection = draft.connectionId ?: return NewGrantResult.Invalid("Choose which AI this is for.")
    val account = draft.account ?: return NewGrantResult.Invalid("Choose which Gmail account this is for.")
    val (addresses, domains) = parseParties(draft.partiesText)
    val subject = draft.subject.trim().ifEmpty { null }
    if (draft.anyMail && draft.send) return NewGrantResult.Invalid("Sending cannot be allowed for everyone.")
    if (draft.anyMail) {
        val ok = draft.lifetime.seconds != null && draft.lifetime.seconds <= NewGrantLifetime.WEEK.seconds!!
        if (!ok) return NewGrantResult.Invalid("Allowing all mail needs a time limit of 1 hour, 24 hours or 7 days.")
    } else if (draft.send && addresses.isEmpty() && domains.isEmpty()) {
        return NewGrantResult.Invalid("Enter who it may send to: addresses like a@b.com or domains like @b.com.")
    } else if (!draft.send && addresses.isEmpty() && domains.isEmpty() && subject == null) {
        return NewGrantResult.Invalid("Enter which senders or which subject text it covers, or choose all mail.")
    }
    val scope = GrantScopeChoice(
        allMail = draft.anyMail,
        selectedMessagesOnly = false,
        senderAddresses = if (draft.send || draft.anyMail) emptyList() else addresses,
        senderDomains = if (draft.send || draft.anyMail) emptyList() else domains,
        subjectPattern = if (draft.anyMail) null else subject,
        recipientAddresses = if (draft.send) addresses else emptyList(),
        recipientDomains = if (draft.send) domains else emptyList(),
        resources = emptyList(),
        classes = emptyList(),
    )
    val standing = StandingGrant(
        durationSecs = draft.lifetime.seconds?.toULong(),
        maxUses = if (draft.lifetime == NewGrantLifetime.ONE_TIME) 1u else null,
        scope = scope,
    )
    return NewGrantResult.Ok(connection, account, if (draft.send) ApprovalKind.SEND else ApprovalKind.READ, standing)
}
