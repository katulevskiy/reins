package dev.reins.android.ui.activity

import dev.reins.android.design.ActionKind
import dev.reins.android.design.MCP_PREFIX
import dev.reins.android.design.serviceName
import dev.reins.android.ui.common.operationTitle
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ActivityEntry
import dev.reins.core.ApprovalView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict
import java.util.Calendar
import java.util.TimeZone

// What Activity says and offers, kept apart from the drawing so it can be tested.

/** One sentence with its key nouns in bold ("Wants to **push 8 commits** to **octo/app**"). */
data class Sentence(val parts: List<Part>) {
    data class Part(val text: String, val bold: Boolean)

    val plain: String get() = parts.joinToString("") { it.text }
}

private class SentenceBuilder {
    val parts = mutableListOf<Sentence.Part>()

    fun t(text: String) = apply { if (text.isNotEmpty()) parts += Sentence.Part(text, false) }

    fun b(text: String) = apply { if (text.isNotEmpty()) parts += Sentence.Part(text, true) }

    fun build() = Sentence(parts.toList())
}

private fun sentence(build: SentenceBuilder.() -> Unit) = SentenceBuilder().apply(build).build()

private fun mail(service: String, op: String) = op.isEmpty() && (service.isBlank() || service == "gmail")

private fun emails(n: Int) = if (n == 1) "1 email" else "$n emails"

/** "Push to a branch" → "push to a branch"; a name that starts the phrase ("GitHub …") keeps its capital. */
private fun lowerFirst(s: String): String =
    if (s.length > 1 && s[1].isUpperCase()) s else s.replaceFirstChar { it.lowercase() }

/** "Stripe <receipts@stripe.com>" → "Stripe"; a bare address stays as it is. */
fun senderName(from: String): String {
    val raw = from.trim()
    val name = raw.substringBefore('<').trim().trim('"')
    return name.ifEmpty { raw.removePrefix("<").removeSuffix(">") }
}

/** The one sender every email comes from, or null when they differ (or there are none). */
fun commonSender(froms: List<String>): String? =
    froms.map { senderName(it) }.filter { it.isNotBlank() }.distinct().singleOrNull()

/** What a waiting request asks, as one sentence. [view] (the request read ahead) adds who the emails are from. */
fun pendingSentence(item: PendingItem, view: ApprovalView? = null): Sentence {
    val label = untrusted(item.connectionLabel)
    return when (item.kind) {
        PendingKind.PAIRING -> sentence { b(label).t(" wants to ").b("connect") }
        PendingKind.JOIN -> sentence { b(label).t(" wants to ").b("join your account") }
        PendingKind.BLOB -> sentence {
            t("Wants to ").b("share a file")
            untrusted(item.subtitle).takeIf { it.isNotBlank() }?.let { t(": ").b(it) }
        }
        else -> requestSentence(item, view)
    }
}

private fun requestSentence(item: PendingItem, view: ApprovalView?): Sentence {
    val count = item.count.toInt()
    val subtitle = untrusted(item.subtitle)
    if (mail(item.service, item.op)) {
        when (ActionKind.of(item.action)) {
            ActionKind.Search -> return sentence {
                t("Wants to ").b("search Gmail")
                (view?.query?.let(::untrusted) ?: subtitle).takeIf { it.isNotBlank() }?.let { t(" for ").b(it) }
            }
            ActionKind.Read -> return sentence {
                t("Wants to ").b("read ${emails(count)}")
                view?.let { v -> commonSender(v.messages.map { untrusted(it.from) }) }?.let { t(" from ").b(it) }
            }
            ActionKind.Send -> return sentence {
                t("Wants to ").b("send an email")
                val to = view?.email?.to?.firstOrNull()?.let(::untrusted)
                    ?: untrusted(item.title).substringAfter(" send an email to ", "").trim()
                if (to.isNotBlank()) {
                    t(" to ").b(to)
                    if (count > 1) t(" and ${count - 1} more")
                }
            }
            ActionKind.Grant -> return sentence {
                t("Asks for ").b("access")
                untrusted(item.title).substringAfter("asks for permission: ", "").trim().takeIf { it.isNotBlank() }?.let { t(": ").b(it) }
            }
            ActionKind.Accounts -> return sentence { t("Wants to ").b("see your accounts") }
            ActionKind.Headlines -> return sentence { t("Wants to ").b("see headlines") }
            else -> Unit
        }
    }
    val verb = lowerFirst(operationTitle(item.action, count, item.service, untrusted(item.opTitle), item.op))
    return sentence {
        t("Wants to ").b(verb)
        // An MCP server's tool says which server; anything else what it acts on (a repository, a chat).
        val where = if (item.service.startsWith(MCP_PREFIX)) serviceName(item.service) else subtitle
        if (where.isNotBlank() && !where.equals(verb, ignoreCase = true)) t(" in ").b(where)
    }
}

/** A group of routine requests from one AI: "Wants to **read 5 emails**", "Wants to **search Gmail** and **read 2 emails**". */
fun groupSentence(items: List<PendingItem>, views: (String) -> ApprovalView?): Sentence {
    data class Phrase(val key: String, val text: (Int) -> String)
    val phrases = linkedMapOf<String, Pair<Phrase, Int>>()
    val froms = mutableListOf<String>()
    var mailReadsOnly = true
    for (item in items) {
        val count = item.count.toInt()
        val (key, phrase, add) = if (mail(item.service, item.op)) {
            when (ActionKind.of(item.action)) {
                ActionKind.Read -> Triple("read", Phrase("read") { "read ${emails(it)}" }, count)
                ActionKind.Search -> Triple("search", Phrase("search") { if (it == 1) "search Gmail" else "search Gmail $it times" }, 1)
                ActionKind.Send -> Triple("send", Phrase("send") { if (it == 1) "send an email" else "send $it emails" }, 1)
                else -> {
                    val title = lowerFirst(operationTitle(item.action, count, item.service, untrusted(item.opTitle), item.op))
                    Triple(title, Phrase(title) { n -> if (n == 1) title else "$title ($n)" }, 1)
                }
            }
        } else {
            val title = lowerFirst(operationTitle(item.action, count, item.service, untrusted(item.opTitle), item.op))
            Triple(title, Phrase(title) { n -> if (n == 1) title else "$title ($n)" }, 1)
        }
        if (key != "read") mailReadsOnly = false
        val prev = phrases[key]
        phrases[key] = phrase to ((prev?.second ?: 0) + add)
        views(item.id)?.messages?.forEach { froms += untrusted(it.from) }
    }
    val list = phrases.values.toList()
    return sentence {
        t("Wants to ")
        list.forEachIndexed { i, (phrase, n) ->
            if (i > 0) t(if (i == list.lastIndex) " and " else ", ")
            b(phrase.text(n))
        }
        if (mailReadsOnly) commonSender(froms)?.let { t(" from ").b(it) }
    }
}

/** The line a group card lists for one of its requests: an email's subject, the search, else the request itself. */
fun groupLine(item: PendingItem, view: ApprovalView?): String {
    view?.messages?.firstOrNull()?.subject?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { subject ->
        val more = (view.messages.size - 1).takeIf { it > 0 }?.let { " +$it" }.orEmpty()
        return subject + more
    }
    view?.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { return it }
    view?.email?.subject?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { return it }
    return pendingSentence(item, view).plain.removePrefix("Wants to ").replaceFirstChar { it.uppercase() }
}

// ---- what the card offers ------------------------------------------------------------------------------------------

/** The bottom row of a waiting card, following Autopilot's suggestion. */
enum class WaitAction {
    /** Autopilot would approve: one-tap Allow (owner check, approve once) and Review. */
    Allow,

    /** Autopilot would deny: one-tap Deny and Review. */
    Deny,

    /** Autopilot judged but is not sure: a plain "Unsure" and Review. */
    Unsure,

    /** Asked every time (passwords, deletions, new connections): a lock, "Always asks" and Review. */
    AlwaysAsks,

    /** Nothing to suggest (no model, a pairing): Review alone. */
    Review,
}

/** Asked every time: never a one-tap Allow, and the card is red. */
fun alwaysAsks(item: PendingItem, suggestion: SuggestionView?): Boolean =
    item.kind == PendingKind.REQUEST && (suggestion?.floor == true || !item.quick)

fun waitAction(item: PendingItem, suggestion: SuggestionView?): WaitAction = when {
    item.kind != PendingKind.REQUEST -> WaitAction.Review
    alwaysAsks(item, suggestion) -> WaitAction.AlwaysAsks
    suggestion == null || !suggestion.judged -> WaitAction.Review
    suggestion.verdict == Verdict.APPROVE -> WaitAction.Allow
    suggestion.verdict == Verdict.DENY -> WaitAction.Deny
    else -> WaitAction.Unsure
}

/** A group's action: Allow (or Deny) only when Autopilot says so for every request in it. */
fun groupAction(items: List<PendingItem>, suggestions: (String) -> SuggestionView?): WaitAction {
    val each = items.map { waitAction(it, suggestions(it.id)) }
    return when {
        each.isEmpty() -> WaitAction.Review
        each.all { it == WaitAction.Allow } -> WaitAction.Allow
        each.all { it == WaitAction.Deny } -> WaitAction.Deny
        each.any { it == WaitAction.AlwaysAsks } -> WaitAction.AlwaysAsks
        each.all { it == WaitAction.Review } -> WaitAction.Review
        else -> WaitAction.Unsure
    }
}

/** The colour family of a card or a history tile: what kind of thing it does. */
enum class Tone { Search, Read, Write, Danger, Grant, Pair, Other }

fun toneOf(action: String, danger: Boolean = false): Tone = when {
    danger -> Tone.Danger
    else -> when (ActionKind.of(action)) {
        ActionKind.Search, ActionKind.List -> Tone.Search
        ActionKind.Read, ActionKind.Headlines, ActionKind.Upload -> Tone.Read
        ActionKind.Write, ActionKind.Send -> Tone.Write
        ActionKind.Grant, ActionKind.Accounts -> Tone.Grant
        ActionKind.Pair, ActionKind.Join -> Tone.Pair
        ActionKind.Other -> Tone.Other
    }
}

fun pendingTone(item: PendingItem, suggestion: SuggestionView?): Tone = when (item.kind) {
    PendingKind.PAIRING, PendingKind.JOIN -> Tone.Pair
    PendingKind.BLOB -> Tone.Read
    else -> toneOf(item.action, alwaysAsks(item, suggestion))
}

/**
 * The cards of the waiting section, in order: each AI's routine requests (a burst of at least two) fold into one
 * group card where the first of them was; everything else is a card of its own.
 */
sealed interface WaitCard {
    data class One(val item: PendingItem) : WaitCard

    data class Group(val burst: Burst, val items: List<PendingItem>) : WaitCard
}

fun waitCards(pending: List<PendingItem>): List<WaitCard> {
    val groups = bursts(pending).associateBy { it.connectionId }
    val grouped = groups.values.flatMap { it.quick }.toSet()
    val placed = mutableSetOf<String>()
    return buildList {
        for (item in pending) {
            if (item.id !in grouped) {
                add(WaitCard.One(item))
                continue
            }
            val burst = groups[item.connectionId] ?: continue
            if (placed.add(burst.connectionId)) {
                val items = pending.filter { it.id in burst.quick }
                add(WaitCard.Group(burst.copy(all = burst.quick), items))
            }
        }
    }
}

// ---- history -------------------------------------------------------------------------------------------------------

private val PAST = mapOf(
    "search" to "Searched", "read" to "Read", "send" to "Sent", "push" to "Pushed", "create" to "Created",
    "comment" to "Commented", "list" to "Listed", "add" to "Added", "delete" to "Deleted", "change" to "Changed",
    "see" to "Saw", "get" to "Got", "update" to "Updated", "use" to "Used", "share" to "Shared", "open" to "Opened",
    "edit" to "Edited", "close" to "Closed", "merge" to "Merged", "move" to "Moved", "archive" to "Archived",
    "download" to "Downloaded", "upload" to "Uploaded", "fetch" to "Fetched", "buy" to "Bought", "connect" to "Connected",
    "reply" to "Replied", "label" to "Labelled", "write" to "Wrote", "post" to "Posted", "sign" to "Signed",
)

/** "Push to a branch" → "Pushed to a branch" (a verb it does not know stays as it is). */
fun pastTense(phrase: String): String {
    val first = phrase.substringBefore(' ')
    val past = PAST[first.lowercase()] ?: return phrase
    return past + phrase.removePrefix(first)
}

private fun done(outcome: String) = outcome == "released" || outcome == "sent" || outcome == "granted"

/** What an entry did, as one line: "Read 2 emails from Stripe", "Pushed to a branch"; what was refused stays as asked. */
fun historySentence(entry: ActivityEntry): String = historyParts(entry).plain

/** [historySentence] with its key nouns in bold, for the entry's page. */
fun historyParts(entry: ActivityEntry): Sentence {
    val count = entry.count.toInt()
    val past = done(entry.outcome)
    val info = entry.info
    if (ActionKind.of(entry.action) == ActionKind.Grant) {
        return sentence {
            t(if (entry.outcome == "granted") "Got " else "Asked for ").b("access")
            info.grantSummary?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { t(": ").b(it) }
        }
    }
    if (mail(entry.service, entry.op)) {
        when (ActionKind.of(entry.action)) {
            ActionKind.Search -> return sentence {
                t(if (past) "Searched " else "Search ").b("Gmail")
                info.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { t(" for ").b("“$it”") }
            }
            ActionKind.Read -> return sentence {
                t("Read ").b(emails(count))
                commonSender(info.messages.map { untrusted(it.from) })?.let { t(" from ").b(it) }
            }
            ActionKind.Send -> return sentence {
                t(if (past) "Sent " else "Send ").b("an email")
                info.email?.to?.firstOrNull()?.let(::untrusted)?.let { t(" to ").b(it) }
            }
            else -> Unit
        }
    }
    val title = operationTitle(entry.action, count, entry.service, untrusted(entry.opTitle), entry.op)
    return sentence { b(if (past) pastTense(title) else title) }
}

/** How an entry ended, as the marker at the end of its row. */
enum class Outcome { Approved, Denied, Auto, AutoDenied, Grant, Bypass, Lockdown, Failed }

fun outcomeOf(entry: ActivityEntry): Outcome = when {
    entry.decidedBy == "bypass" -> Outcome.Bypass
    entry.decidedBy == "lockdown" -> Outcome.Lockdown
    entry.decidedBy.isNotEmpty() -> if (entry.outcome == "denied") Outcome.AutoDenied else Outcome.Auto
    entry.outcome == "error" -> Outcome.Failed
    entry.outcome == "denied" -> Outcome.Denied
    entry.grantId != null -> Outcome.Grant
    else -> Outcome.Approved
}

/** The history's filters, shown as chips once the filter button is tapped. */
enum class HistoryFilter(val label: String) {
    All("All"),
    Autopilot("Autopilot"),
    Denied("Denied"),
    Grants("Grants");

    fun matches(e: ActivityEntry): Boolean = when (this) {
        All -> true
        Autopilot -> e.decidedBy.isNotEmpty()
        Denied -> e.outcome == "denied"
        Grants -> e.grantId != null || ActionKind.of(e.action) == ActionKind.Grant
    }
}

/** Days since the epoch in the phone's time zone, to group the history by day. */
fun dayOf(epochSeconds: Long, zone: TimeZone = TimeZone.getDefault()): Long {
    val ms = epochSeconds * 1000
    return Math.floorDiv(ms + zone.getOffset(ms), 86_400_000L)
}

/** "Today", "Yesterday", else the date ("Mon, 3 Mar"). */
fun dayLabel(epochSeconds: Long, nowSeconds: Long, zone: TimeZone = TimeZone.getDefault()): String =
    when (dayOf(nowSeconds, zone) - dayOf(epochSeconds, zone)) {
        0L -> "Today"
        1L -> "Yesterday"
        else -> {
            val cal = Calendar.getInstance(zone).apply { timeInMillis = epochSeconds * 1000 }
            val sameYear = cal.get(Calendar.YEAR) == Calendar.getInstance(zone).apply { timeInMillis = nowSeconds * 1000 }.get(Calendar.YEAR)
            val pattern = if (sameYear) "EEE, d MMM" else "d MMM yyyy"
            java.text.SimpleDateFormat(pattern, java.util.Locale.getDefault()).apply { timeZone = zone }.format(java.util.Date(epochSeconds * 1000))
        }
    }

/** A row's time: "just now", "5 min" within the hour, else the clock time. */
fun rowTime(epochSeconds: Long, nowSeconds: Long, zone: TimeZone = TimeZone.getDefault()): String {
    val d = nowSeconds - epochSeconds
    return when {
        d in 0 until 45 -> "just now"
        d in 45 until 3_600 -> "${maxOf(1, d / 60)} min"
        else -> java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).apply { timeZone = zone }.format(java.util.Date(epochSeconds * 1000))
    }
}

/** The AI's pick in words, for the entry's outcome pill: "Allowed by Autopilot · 96%". */
fun outcomeLine(entry: ActivityEntry): String {
    val note = entry.autopilot
    val denied = entry.outcome == "denied"
    return when (entry.decidedBy) {
        "autopilot" -> {
            val p = note?.let { if (denied) it.pDeny else it.pApprove }?.takeIf { it > 0f }
            (if (denied) "Denied by Autopilot" else "Allowed by Autopilot") + (p?.let { " · ${Math.round(it * 100)}%" } ?: "")
        }
        "bypass" -> "Allowed during a bypass"
        "lockdown" -> "Denied by Lockdown"
        else -> when {
            entry.outcome == "error" -> "Failed"
            denied -> "Denied by you"
            entry.grantId != null -> "Covered by a grant"
            entry.outcome == "granted" -> "Access given by you"
            else -> "Allowed by you"
        }
    }
}

/** "Gmail" for a service id, for the entry detail's value rows. */
fun serviceLabel(service: String): String = serviceName(service.ifBlank { "gmail" })

/** "Today, 14:14", "Yesterday, 09:02", "Tomorrow, 19:40", else the date and time. */
fun dayTime(epochSeconds: Long, nowSeconds: Long, zone: TimeZone = TimeZone.getDefault()): String {
    val time = java.text.DateFormat.getTimeInstance(java.text.DateFormat.SHORT).apply { timeZone = zone }.format(java.util.Date(epochSeconds * 1000))
    return when (dayOf(epochSeconds, zone) - dayOf(nowSeconds, zone)) {
        0L -> "Today, $time"
        -1L -> "Yesterday, $time"
        1L -> "Tomorrow, $time"
        else -> "${dayLabel(epochSeconds, nowSeconds, zone)}, $time"
    }
}
