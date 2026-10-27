package dev.reins.android.ui.circles

import dev.reins.android.ui.activity.dueIn
import dev.reins.core.CircleInviteProgress
import dev.reins.core.CircleLinkState
import dev.reins.core.CirclePerson
import dev.reins.core.CircleProgress
import dev.reins.core.CircleReviewState
import dev.reins.core.CircleReviewView
import dev.reins.core.CircleRole
import dev.reins.core.CircleScopePreset

// Circles (Decisions v2, 04): other Reins users who share control of an account. What the screens say about them,
// kept free of Compose so it is tested on its own.

/** The role as the screens name it. */
fun roleLabel(role: CircleRole): String = when (role) {
    CircleRole.DELEGATE -> "Delegate"
    CircleRole.COSIGN -> "Co-sign"
    CircleRole.GUARDIAN -> "Guardian"
    CircleRole.WATCHER -> "Watcher"
}

/** What the role means, in a few words (behind a (?) on the Add screen). */
fun roleHelp(role: CircleRole): String = when (role) {
    CircleRole.DELEGATE -> "Decides for you. Either of you is enough."
    CircleRole.COSIGN -> "Must approve too. Both of you."
    CircleRole.GUARDIAN -> "Must approve what you agreed to."
    CircleRole.WATCHER -> "Is told. No buttons."
}

/** The scope presets the Add screen offers, in order (One AI needs a connection, picked after). */
val PRESETS: List<String> = listOf("everything", "money", "messages", "risky", "routine", "agent")

/** A preset's key ([PRESETS]) as the core's preset; One AI with its connection. */
fun preset(key: String, connectionId: String? = null, label: String = ""): CircleScopePreset? = when (key) {
    "everything" -> CircleScopePreset.Everything
    "money" -> CircleScopePreset.Money
    "messages" -> CircleScopePreset.MessagesOutside
    "risky" -> CircleScopePreset.Risky
    "routine" -> CircleScopePreset.Routine
    "agent" -> connectionId?.let { CircleScopePreset.Agent(it, label) }
    else -> null
}

/** A preset's key as the Add screen names it. */
fun presetLabel(key: String): String = when (key) {
    "everything" -> "Everything"
    "money" -> "Money"
    "messages" -> "Messages to Work or Outside"
    "risky" -> "What can't be undone"
    "routine" -> "Routine"
    "agent" -> "One AI"
    else -> key
}

/** A watcher only hears about things: no window, no class, never the full card. */
fun decides(role: CircleRole): Boolean = role != CircleRole.WATCHER

/** "2 min", "4 h", "1 d": a window as the screens say it. */
fun windowLabel(secs: Long): String = when {
    secs <= 0 -> "None"
    secs < 3_600 -> "${(secs + 59) / 60} min"
    secs < 86_400 -> if (secs % 3_600 == 0L) "${secs / 3_600} h" else "${secs / 3_600} h ${(secs % 3_600) / 60} min"
    else -> "${secs / 86_400} d"
}

/** "Deferred · 4 h", "Instant · 2 min". */
fun classLine(deferred: Boolean, windowSecs: Long): String =
    (if (deferred) "Deferred" else "Instant") + if (windowSecs > 0) " · ${windowLabel(windowSecs)}" else ""

/** The arrangements, split: who helps this account, and whom this account helps. Ended ones are gone. */
data class PeopleGroups(val helpingYou: List<CirclePerson>, val youHelp: List<CirclePerson>) {
    val empty: Boolean get() = helpingYou.isEmpty() && youHelp.isEmpty()
}

/** [people] grouped for Settings › Circles: active first, then paused and suspended, each by name. */
fun groupPeople(people: List<CirclePerson>): PeopleGroups {
    val shown = people.filter { it.state != CircleLinkState.ENDED }
        .sortedWith(compareBy<CirclePerson> { stateOrder(it.state) }.thenBy { it.peer.lowercase() })
    return PeopleGroups(shown.filter { it.mine }, shown.filterNot { it.mine })
}

private fun stateOrder(state: CircleLinkState): Int = when (state) {
    CircleLinkState.ACTIVE -> 0
    CircleLinkState.PAUSED -> 1
    CircleLinkState.SUSPENDED -> 2
    CircleLinkState.ENDED -> 3
}

/** The badge a row carries for its state; none while active. */
fun stateBadge(state: CircleLinkState): String? = when (state) {
    CircleLinkState.PAUSED -> "Paused"
    CircleLinkState.SUSPENDED -> "Suspended"
    CircleLinkState.ENDED -> "Ended"
    CircleLinkState.ACTIVE -> null
}

/** A person's row under their name: "Co-sign · Money · Deferred". A watcher has no class. */
fun personLine(p: CirclePerson): String = listOfNotNull(
    roleLabel(p.role),
    p.scopeLine.trim().ifEmpty { null },
    if (decides(p.role)) (if (p.deferred) "Deferred" else "Instant") else null,
).joinToString(" · ")

/** The owner may pause and resume; only an active or paused arrangement. */
fun canPause(p: CirclePerson): Boolean = p.mine && (p.state == CircleLinkState.ACTIVE || p.state == CircleLinkState.PAUSED)

/** The owner ends an arrangement; a helper leaves it. */
fun endLabel(p: CirclePerson): String = if (p.mine) "End" else "Leave"

/** What a person is called on screen: their address's name part, as given ("alex" from alex@example.com). */
fun shortName(email: String): String {
    val name = email.substringBefore('@').trim()
    return name.ifEmpty { email }.replaceFirstChar { it.uppercase() }
}

/** The safety code with a space in the middle ("482 913"), easier to read out. */
fun formatCode(code: String): String {
    val digits = code.filter { it.isDigit() }
    return if (digits.length == 6) "${digits.substring(0, 3)} ${digits.substring(3)}" else code
}

// ---- The owner's card: who else decides ----------------------------------------------------------------------------

/**
 * The line on the owner's card: the core's own ("Needs Alex too", "Alex approved, waiting for you"), or, if it sent
 * none, one made from who approved and who is still to.
 */
fun progressLine(p: CircleProgress): String {
    if (p.line.isNotBlank()) return p.line.trim()
    val needs = p.needs.map(::shortName)
    val approved = p.approved.map(::shortName)
    return when {
        p.youApproved && needs.isNotEmpty() -> "You approved · waiting for ${needs.joinToString(", ")}"
        approved.isNotEmpty() && p.waitingForYou -> "${approved.joinToString(", ")} approved, waiting for you"
        needs.isNotEmpty() -> "Needs ${needs.joinToString(", ")} too"
        approved.isNotEmpty() -> "${approved.joinToString(", ")} approved"
        else -> ""
    }
}

/** The owner already approved: the request waits for the others, and Approve reads as done. */
fun approvedByMe(p: CircleProgress?): Boolean = p?.youApproved == true

// ---- For others (the helper's side) --------------------------------------------------------------------------------

/** How long a settled review stays in For others, dimmed. */
const val SETTLED_KEPT_SECS: Long = 3 * 86_400

/** One person's reviews that wait for this phone, soonest deadline first. */
data class PersonReviews(val person: String, val reviews: List<CircleReviewView>) {
    val soonest: Long get() = reviews.minOf { it.deadline }
}

/** What For others shows: the waiting reviews by person (soonest first), then the settled ones of the last days. */
data class ForOthers(val waiting: List<PersonReviews>, val settled: List<CircleReviewView>) {
    val empty: Boolean get() = waiting.isEmpty() && settled.isEmpty()
    val count: Int get() = waiting.sumOf { it.reviews.size }
}

/**
 * [reviews] for For others at [nowSeconds]: those that wait, grouped by person, the person with the soonest deadline
 * first; a waiting one past its deadline counts as expired. Settled ones made in the last [SETTLED_KEPT_SECS] follow,
 * newest first.
 */
fun forOthers(reviews: List<CircleReviewView>, nowSeconds: Long): ForOthers {
    val (open, done) = reviews.partition { it.state == CircleReviewState.WAITING && it.deadline > nowSeconds }
    val waiting = open
        .sortedWith(compareBy<CircleReviewView> { it.deadline }.thenBy { it.createdAt })
        .groupBy { it.person }
        .map { (person, items) -> PersonReviews(person, items) }
        .sortedBy { it.soonest }
    val settled = done
        .filter { nowSeconds - it.createdAt < SETTLED_KEPT_SECS }
        .sortedByDescending { it.createdAt }
    return ForOthers(waiting, settled)
}

/** A settled review's state, in a word; a waiting one past its deadline is expired. */
fun reviewStateLabel(r: CircleReviewView, nowSeconds: Long): String = when (r.state) {
    CircleReviewState.WAITING -> if (r.deadline <= nowSeconds) "Expired" else dueIn(r.deadline, nowSeconds)
    CircleReviewState.APPROVED -> "Approved"
    CircleReviewState.DENIED -> "Denied"
    CircleReviewState.EXPIRED -> "Expired"
    CircleReviewState.WITHDRAWN -> "Withdrawn"
}

/** The one Circle notification's title: "Sam: Buy headphones waiting". */
fun reviewNotificationTitle(r: CircleReviewView): String = "${shortName(r.person)}: ${r.title.trim()} waiting"

/** The card's top line: whose request it is, and their agent ("Sam · ChatGPT"). */
fun reviewWho(r: CircleReviewView): String = listOfNotNull(shortName(r.person), r.agent.trim().ifEmpty { null }).joinToString(" · ")

/** A note typed with a denial: nothing typed is no note. */
fun noteOrNull(text: String): String? = text.trim().ifEmpty { null }

// ---- Invitations ---------------------------------------------------------------------------------------------------

/** The safety code of an invitation in progress, once someone answered. */
fun inviteCode(p: CircleInviteProgress): String? = when (p) {
    is CircleInviteProgress.Compare -> p.code
    is CircleInviteProgress.Accepted -> p.code
    else -> null
}

/** Who answered the invitation (their account's email). */
fun inviteHelper(p: CircleInviteProgress): String? = when (p) {
    is CircleInviteProgress.Compare -> p.helper
    is CircleInviteProgress.Accepted -> p.helper
    else -> null
}

/** The invitation will not move any more: polling stops. */
fun inviteFinished(p: CircleInviteProgress): Boolean = when (p) {
    is CircleInviteProgress.Active, CircleInviteProgress.Declined, CircleInviteProgress.Expired, CircleInviteProgress.Cancelled -> true
    else -> false
}

/** How an invitation ended, in a few words; null while it runs. */
fun inviteEnd(p: CircleInviteProgress): String? = when (p) {
    is CircleInviteProgress.Active -> "Added"
    CircleInviteProgress.Declined -> "Declined"
    CircleInviteProgress.Expired -> "Expired"
    CircleInviteProgress.Cancelled -> "Cancelled"
    else -> null
}

/** The scheme of a Circle invitation link. */
const val CIRCLE_SCHEME = "reins-circle:"

/** A pasted or scanned invitation, trimmed; null when it is not one. */
fun circleLink(text: String): String? = text.trim().takeIf { it.startsWith(CIRCLE_SCHEME) && it.length > CIRCLE_SCHEME.length }
