package dev.rewarden.android.ui.grants

import dev.rewarden.core.GrantScopeChoice
import dev.rewarden.core.GrantView
import dev.rewarden.core.StandingGrant

/** For how long an ended grant can be started again. */
enum class ResumePeriod(val label: String, val seconds: Long) {
    HOUR("1 hour", 3_600),
    DAY("24 hours", 86_400),
    WEEK("7 days", 604_800),
    MONTH("30 days", 2_592_000),
}

/** A grant for all mail can be resumed for a week at most, like it can be created. */
fun resumePeriods(allMail: Boolean): List<ResumePeriod> =
    if (allMail) listOf(ResumePeriod.HOUR, ResumePeriod.DAY, ResumePeriod.WEEK) else ResumePeriod.entries

/** The units a custom time can be given in. */
enum class CustomUnit(val label: String, val seconds: Long) {
    MINUTES("minutes", 60),
    HOURS("hours", 3_600),
    DAYS("days", 86_400),
}

private const val MIN_SECONDS = 60L
private const val MAX_SECONDS = 30 * 86_400L
private const val MAX_ALL_MAIL_SECONDS = 7 * 86_400L
const val MAX_RESUME_USES = 1000

/** Everything the resume dialog lets the user change. */
data class ResumeDraft(
    /** One of the quick choices; ignored when a custom time is typed. */
    val period: ResumePeriod = ResumePeriod.DAY,
    val customAmount: String = "",
    val customUnit: CustomUnit = CustomUnit.HOURS,
    val moreOpen: Boolean = false,
    val limitUses: Boolean = false,
    val uses: String = "5",
    val allMail: Boolean = false,
    /** Addresses (`a@b.com`) and domains (`@b.com`): senders for a read grant, recipients for a send grant. */
    val partiesText: String = "",
    val subject: String = "",
)

sealed interface ResumeResult {
    /** [standing] is set only when the user changed who or what it covers, or the number of uses. */
    data class Ok(val seconds: Long, val standing: StandingGrant?) : ResumeResult

    data class Invalid(val message: String) : ResumeResult
}

/** The dialog as it opens: the grant exactly as it was, for a day. */
fun initialResumeDraft(grant: GrantView): ResumeDraft {
    val scope = grant.editableScope
    return ResumeDraft(
        period = ResumePeriod.DAY,
        limitUses = grant.maxUses != null,
        uses = (grant.maxUses ?: 5u).toString(),
        allMail = scope?.allMail == true,
        partiesText = scope?.let { parties(grant, it) }.orEmpty(),
        subject = scope?.subjectPattern.orEmpty(),
    )
}

private fun parties(grant: GrantView, scope: GrantScopeChoice): String {
    val addresses = if (grant.action == "send") scope.recipientAddresses else scope.senderAddresses
    val domains = if (grant.action == "send") scope.recipientDomains else scope.senderDomains
    return (addresses + domains.map { "@$it" }).joinToString(", ")
}

/** How long the draft asks for: the typed time if there is one, else the quick choice. */
fun resumeSeconds(draft: ResumeDraft): Long? =
    if (draft.customAmount.isNotBlank()) {
        draft.customAmount.trim().toLongOrNull()?.let { it * draft.customUnit.seconds }
    } else {
        draft.period.seconds
    }

/** Turns the dialog into what the core takes, or says what is wrong. Never guesses. */
fun buildResume(grant: GrantView, draft: ResumeDraft): ResumeResult {
    val seconds = resumeSeconds(draft) ?: return ResumeResult.Invalid("Enter the time as a whole number.")
    val editable = grant.editableScope
    val allMail = editable != null && draft.allMail
    if (seconds < MIN_SECONDS) return ResumeResult.Invalid("Choose at least a minute.")
    if (seconds > MAX_SECONDS) return ResumeResult.Invalid("Choose 30 days at most.")
    if (allMail && seconds > MAX_ALL_MAIL_SECONDS) return ResumeResult.Invalid("Access to all mail can last 7 days at most.")
    val maxUses = if (draft.limitUses) {
        draft.uses.trim().toIntOrNull()?.takeIf { it in 1..MAX_RESUME_USES }
            ?: return ResumeResult.Invalid("Uses must be between 1 and $MAX_RESUME_USES.")
    } else {
        null
    }
    val usesChanged = maxUses?.toUInt() != grant.maxUses

    var scope: GrantScopeChoice? = null
    if (editable != null) {
        val send = grant.action == "send"
        val (addresses, domains) = parseParties(draft.partiesText)
        val subject = draft.subject.trim().ifEmpty { null }
        if (!allMail) {
            if (send && addresses.isEmpty() && domains.isEmpty()) {
                return ResumeResult.Invalid("Enter who it may send to: addresses like a@b.com or domains like @b.com.")
            }
            if (!send && addresses.isEmpty() && domains.isEmpty() && subject == null) {
                return ResumeResult.Invalid("Enter which senders or which subject text it covers, or choose all mail.")
            }
        }
        scope = GrantScopeChoice(
            allMail = allMail,
            selectedMessagesOnly = false,
            senderAddresses = if (send || allMail) emptyList() else addresses,
            senderDomains = if (send || allMail) emptyList() else domains,
            subjectPattern = if (allMail) null else subject,
            recipientAddresses = if (send) addresses else emptyList(),
            recipientDomains = if (send) domains else emptyList(),
            resources = emptyList(),
            classes = emptyList(),
        )
    }
    val scopeChanged = scope != null && !sameScope(scope, editable!!)
    val standing = if (scope != null && (scopeChanged || usesChanged)) {
        StandingGrant(durationSecs = seconds.toULong(), maxUses = maxUses?.toUInt(), scope = scope)
    } else {
        null
    }
    return ResumeResult.Ok(seconds, standing)
}

private fun sameScope(a: GrantScopeChoice, b: GrantScopeChoice): Boolean =
    a.allMail == b.allMail &&
        a.senderAddresses.sorted() == b.senderAddresses.sorted() &&
        a.senderDomains.sorted() == b.senderDomains.sorted() &&
        a.recipientAddresses.sorted() == b.recipientAddresses.sorted() &&
        a.recipientDomains.sorted() == b.recipientDomains.sorted() &&
        a.subjectPattern?.trim().orEmpty() == b.subjectPattern?.trim().orEmpty()
