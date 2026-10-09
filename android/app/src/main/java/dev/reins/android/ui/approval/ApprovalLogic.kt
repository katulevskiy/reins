package dev.reins.android.ui.approval

import dev.reins.core.ApprovalChoice
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.GrantScopeChoice
import dev.reins.core.StandingGrant

/** Lifetime choices of the approval screen: Once · 1 h · 24 h · 7 days · until revoked · N uses. */
enum class LifetimeKind(val label: String, val seconds: Long?) {
    ONCE("Once", null),
    HOUR("1 hour", 3_600),
    DAY("24 hours", 86_400),
    WEEK("7 days", 604_800),
    MONTH("30 days", 2_592_000),
    UNTIL_REVOKED("Until revoked", null),
    USES("N uses", null),
}

const val MAX_USES = 1000

/** Time boxes offered for "allow all mail": the policy never allows an open-ended grant for everything. */
val ALL_MAIL_LIFETIMES = listOf(LifetimeKind.HOUR, LifetimeKind.DAY, LifetimeKind.WEEK)

/** Everything the user can change on the approval screen. */
data class ApprovalDraft(
    val selected: Set<String> = emptySet(),
    val lifetime: LifetimeKind = LifetimeKind.ONCE,
    val uses: Int = 5,
    /** Read: false = "only these messages", true = "also allow similar". */
    val similar: Boolean = false,
    val senderAddresses: Set<String> = emptySet(),
    val senderDomains: Set<String> = emptySet(),
    val subject: String = "",
    /** Read: allow every message (any search or read) for this long; overrides lifetime and scope when set. */
    val allMail: LifetimeKind? = null,
    /** Send: recipient addresses that are allowed by whole domain instead of exactly. */
    val domainRecipients: Set<String> = emptySet(),
    /** Permission requests: allow it for less time than asked (seconds); null = as asked. */
    val grantSeconds: Long? = null,
    /** Another integration: the chats, calendars or repositories a standing permission covers. */
    val resources: Set<String> = emptySet(),
    /** Another integration, a permission to change things: the kinds of change it allows (ids from `ApprovalView.classes`). */
    val classes: Set<String> = emptySet(),
)

sealed interface BuildResult {
    data class Ok(val choice: ApprovalChoice) : BuildResult

    data class Invalid(val message: String) : BuildResult
}

/** `Name <a@b.com>` or `a@b.com` → `a@b.com` (lower-cased); null when there is no usable address. */
fun addressOf(line: String): String? {
    val inBrackets = Regex("<([^<>\\s]+)>\\s*$").find(line)?.groupValues?.get(1)
    val candidate = (inBrackets ?: line.trim()).lowercase()
    val at = candidate.indexOf('@')
    return if (at > 0 && at == candidate.lastIndexOf('@') && at < candidate.length - 1 && candidate.none { it.isWhitespace() }) {
        candidate
    } else {
        null
    }
}

fun domainOf(address: String): String = address.substringAfterLast('@').lowercase()

/** Distinct sender addresses of the messages in [view] that the user ticked, in order of appearance. */
fun senderAddresses(view: ApprovalView, selected: Set<String>): List<String> =
    view.messages.filter { it.id in selected }.mapNotNull { addressOf(it.from) }.distinct()

fun recipientAddresses(view: ApprovalView): List<String> =
    view.email?.let { (it.to + it.cc).mapNotNull(::addressOf).distinct() } ?: emptyList()

/** Turns the screen state into what the core expects, or says what is missing. Never guesses. */
fun buildChoice(view: ApprovalView, draft: ApprovalDraft): BuildResult {
    if (view.kind == ApprovalKind.GRANT) return buildPermissionAnswer(view, draft)
    if (view.kind == ApprovalKind.ACCOUNTS) return buildAccountsAnswer(view, draft)
    if (view.kind == ApprovalKind.FETCH || view.kind == ApprovalKind.WRITE) return buildConnectorAnswer(view, draft)
    val send = view.kind == ApprovalKind.SEND
    val covered = view.messages.filter { it.coveredByGrant }.map { it.id }
    // "All mail" releases everything shown but what looks like a code, so nothing has to be ticked.
    val ids = when {
        send -> emptyList()
        draft.allMail != null -> view.messages.filter { !it.sensitive || it.id in draft.selected }.map { it.id }
        else -> view.messages.map { it.id }.filter { it in draft.selected || it in covered }
    }
    if (draft.allMail != null) return buildAllMail(view, draft, ids)
    if (!send && ids.isEmpty()) return BuildResult.Invalid("Select at least one message.")
    val standing = if (draft.lifetime == LifetimeKind.ONCE) {
        null
    } else {
        if (draft.lifetime == LifetimeKind.USES && draft.uses !in 1..MAX_USES) {
            return BuildResult.Invalid("Uses must be between 1 and $MAX_USES.")
        }
        val scope = when (val built = if (send) sendScope(view, draft) else readScope(draft)) {
            is ScopeResult.Ok -> built.scope
            is ScopeResult.Missing -> return BuildResult.Invalid(built.message)
        }
        StandingGrant(
            durationSecs = draft.lifetime.seconds?.toULong(),
            maxUses = if (draft.lifetime == LifetimeKind.USES) draft.uses.toUInt() else null,
            scope = scope,
        )
    }
    return BuildResult.Ok(ApprovalChoice(selectedMessageIds = ids, standing = standing))
}

/** Time boxes offered for "everything in this integration": never open-ended, and never for changing anything. */
val EVERYTHING_LIFETIMES = ALL_MAIL_LIFETIMES

/**
 * Lists, reads and searches in another integration (the ticked items are released) and changes to it (the preview is
 * what will be done). A permission made alongside covers the things ticked under "What should it cover?".
 */
private fun buildConnectorAnswer(view: ApprovalView, draft: ApprovalDraft): BuildResult {
    val write = view.kind == ApprovalKind.WRITE
    val covered = view.messages.filter { it.coveredByGrant }.map { it.id }
    val ids = if (write) emptyList() else view.messages.map { it.id }.filter { it in draft.selected || it in covered }
    if (!write && ids.isEmpty() && view.messages.isNotEmpty()) return BuildResult.Invalid("Tick at least one item.")
    if (view.noStanding || (draft.lifetime == LifetimeKind.ONCE && draft.allMail == null)) {
        return BuildResult.Ok(ApprovalChoice(ids, null))
    }
    if (draft.lifetime == LifetimeKind.USES && draft.uses !in 1..MAX_USES) {
        return BuildResult.Invalid("Uses must be between 1 and $MAX_USES.")
    }
    val everything = draft.allMail
    if (everything != null) {
        if (write) return BuildResult.Invalid("Changing things cannot be allowed everywhere.")
        if (everything !in EVERYTHING_LIFETIMES) return BuildResult.Invalid("Allowing everything needs a time limit: 1 hour, 24 hours or 7 days.")
        val scope = GrantScopeChoice(true, false, emptyList(), emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList())
        return BuildResult.Ok(ApprovalChoice(ids, StandingGrant(everything.seconds?.toULong(), null, scope)))
    }
    val known = view.resources.map { it.id }.toSet()
    val picked = draft.resources.filter { it in known }
    // A wider permission already covers what is inside it: `owner/repo` and `owner/repo@main` is just `owner/repo`.
    val chosen = picked.filter { id -> picked.none { other -> other != id && resourceCovers(other, id) } }.sorted()
    if (chosen.isEmpty()) return BuildResult.Invalid("Choose what the permission should cover.")
    // Nothing ticked (or nothing offered) = the kind of this request, which is what the core assumes.
    val classes = if (write) view.classes.map { it.id }.filter { it in draft.classes } else emptyList()
    val scope = GrantScopeChoice(false, false, emptyList(), emptyList(), null, emptyList(), emptyList(), chosen, classes)
    return BuildResult.Ok(
        ApprovalChoice(
            ids,
            StandingGrant(
                durationSecs = draft.lifetime.seconds?.toULong(),
                maxUses = if (draft.lifetime == LifetimeKind.USES) draft.uses.toUInt() else null,
                scope = scope,
            ),
        ),
    )
}

/** The permission for [granted] also covers [thing]: the same thing, or something inside it (`A` covers `A`, `A@x`, `A/x`). */
fun resourceCovers(granted: String, thing: String): Boolean =
    thing == granted || (thing.startsWith(granted) && thing[granted.length].let { it == '@' || it == '/' })

/** What starts ticked under "What should it cover?": the things the request touches, not the wider ones they belong to. */
fun defaultResources(view: ApprovalView): Set<String> = view.resources.filter { !it.wider }.map { it.id }.toSet()

/** What starts ticked under "Allow these kinds of change": only the kind of this request. */
fun defaultClasses(view: ApprovalView): Set<String> = view.classes.map { it.id }.filter { it == view.`class` }.toSet()

/**
 * Ticks or unticks [id]. Ticking a wider thing unticks the ones inside it (its permission covers them anyway), and
 * ticking a narrower one unticks the wider ones around it, so the ticks never say the same thing twice.
 */
fun toggleResource(selected: Set<String>, id: String, on: Boolean): Set<String> =
    if (!on) {
        selected - id
    } else {
        selected.filter { other -> !resourceCovers(id, other) && !resourceCovers(other, id) }.toSet() + id
    }

/** Ticks or unticks a kind of change; the last one stays ticked. */
fun toggleClass(selected: Set<String>, id: String, on: Boolean): Set<String> =
    if (on) selected + id else if (selected == setOf(id)) selected else selected - id

/** The periods offered for showing an integration's accounts to an AI; a month is what the user gets by default. */
val ACCOUNTS_LIFETIMES = listOf(LifetimeKind.ONCE, LifetimeKind.HOUR, LifetimeKind.DAY, LifetimeKind.WEEK, LifetimeKind.MONTH)

/** The accounts that can still be ticked: those the AI was not already allowed to see. */
fun shareableAccounts(view: ApprovalView): List<String> = view.accounts.filter { it !in view.sharedAccounts }

/**
 * Showing the ticked accounts once, or for a while (which leaves a grant naming exactly those accounts for this AI).
 * The addresses travel in `selectedMessageIds`.
 */
private fun buildAccountsAnswer(view: ApprovalView, draft: ApprovalDraft): BuildResult {
    val picked = shareableAccounts(view).filter { it in draft.selected }
    if (picked.isEmpty()) return BuildResult.Invalid("Tick at least one account, or deny the request.")
    val lifetime = draft.lifetime
    if (lifetime !in ACCOUNTS_LIFETIMES) return BuildResult.Invalid("Choose once, or one of the periods.")
    val standing = lifetime.seconds?.let {
        StandingGrant(
            durationSecs = it.toULong(),
            maxUses = null,
            scope = GrantScopeChoice(false, false, emptyList(), emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList()),
        )
    }
    return BuildResult.Ok(ApprovalChoice(picked, standing))
}

/** Allowing a permission an AI asked for: as asked, or (never longer) for less time. */
private fun buildPermissionAnswer(view: ApprovalView, draft: ApprovalDraft): BuildResult {
    val asked = view.grant?.durationSecs ?: return BuildResult.Invalid("This request has no permission attached.")
    val shorter = draft.grantSeconds?.toULong()?.takeIf { it < asked }
    val standing = shorter?.let {
        StandingGrant(
            durationSecs = it,
            maxUses = null,
            scope = GrantScopeChoice(false, false, emptyList(), emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList()),
        )
    }
    return BuildResult.Ok(ApprovalChoice(emptyList(), standing))
}

private fun buildAllMail(view: ApprovalView, draft: ApprovalDraft, ids: List<String>): BuildResult {
    val lifetime = draft.allMail
    if (view.kind == ApprovalKind.SEND) return BuildResult.Invalid("Sending cannot be allowed for everyone.")
    if (lifetime == null || lifetime !in ALL_MAIL_LIFETIMES) {
        return BuildResult.Invalid("Allowing all mail needs a time limit: 1 hour, 24 hours or 7 days.")
    }
    val scope = GrantScopeChoice(
        allMail = true,
        selectedMessagesOnly = false,
        senderAddresses = emptyList(),
        senderDomains = emptyList(),
        subjectPattern = null,
        recipientAddresses = emptyList(),
        recipientDomains = emptyList(),
        resources = emptyList(),
        classes = emptyList(),
    )
    return BuildResult.Ok(ApprovalChoice(ids, StandingGrant(lifetime.seconds?.toULong(), null, scope)))
}

private sealed interface ScopeResult {
    data class Ok(val scope: GrantScopeChoice) : ScopeResult

    data class Missing(val message: String) : ScopeResult
}

private fun readScope(draft: ApprovalDraft): ScopeResult {
    if (!draft.similar) {
        return ScopeResult.Ok(GrantScopeChoice(false, true, emptyList(), emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList()))
    }
    val subject = draft.subject.trim().ifEmpty { null }
    if (draft.senderAddresses.isEmpty() && draft.senderDomains.isEmpty() && subject == null) {
        return ScopeResult.Missing("Choose at least one sender, domain or subject text for the similar mail.")
    }
    return ScopeResult.Ok(
        GrantScopeChoice(
            allMail = false,
            selectedMessagesOnly = false,
            senderAddresses = draft.senderAddresses.sorted(),
            senderDomains = draft.senderDomains.sorted(),
            subjectPattern = subject,
            recipientAddresses = emptyList(),
            recipientDomains = emptyList(),
            resources = emptyList(),
            classes = emptyList(),
        ),
    )
}

private fun sendScope(view: ApprovalView, draft: ApprovalDraft): ScopeResult {
    val recipients = recipientAddresses(view)
    val domains = recipients.filter { it in draft.domainRecipients }.map(::domainOf).distinct().sorted()
    val exact = recipients.filter { it !in draft.domainRecipients }
    if (exact.isEmpty() && domains.isEmpty()) return ScopeResult.Missing("This email has no recipients to allow.")
    return ScopeResult.Ok(
        GrantScopeChoice(
            allMail = false,
            selectedMessagesOnly = false,
            senderAddresses = emptyList(),
            senderDomains = emptyList(),
            subjectPattern = draft.subject.trim().ifEmpty { null },
            recipientAddresses = exact,
            recipientDomains = domains,
            resources = emptyList(),
            classes = emptyList(),
        ),
    )
}

/** Free mail providers: allowing one of these domains allows millions of unrelated people. */
object PublicMailDomains {
    private val domains = setOf(
        "gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "live.com", "msn.com", "yahoo.com", "ymail.com",
        "rocketmail.com", "icloud.com", "me.com", "mac.com", "aol.com", "proton.me", "protonmail.com", "pm.me",
        "gmx.com", "gmx.de", "gmx.net", "web.de", "mail.com", "mail.ru", "yandex.com", "yandex.ru", "zoho.com",
        "fastmail.com", "hey.com", "tutanota.com", "tuta.io", "qq.com", "163.com", "126.com", "comcast.net",
        "att.net", "verizon.net", "sbcglobal.net", "orange.fr", "free.fr", "t-online.de", "libero.it",
    )

    fun isPublic(domain: String): Boolean = domain.lowercase().trimStart('@') in domains
}

/** Public mail domains a standing grant built from [choice] would allow as a whole. */
fun publicDomainWarnings(choice: ApprovalChoice): List<String> {
    val scope = choice.standing?.scope ?: return emptyList()
    return (scope.senderDomains + scope.recipientDomains).filter(PublicMailDomains::isPublic).distinct()
}
