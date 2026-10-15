package dev.reins.android.ui.approval

import dev.reins.android.design.serviceName
import dev.reins.android.ui.common.operationTitle
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.GitRefView

// The approval screen in words and colours, kept apart from the views so every rule is tested (ApprovalWordsTest).

/** A run of the hero's sentence; the key nouns are [bold]. */
data class Span(val text: String, val bold: Boolean = false)

/** The hero's colour: what kind of thing is asked. */
enum class HeroTone { Write, Search, Send, Grant, Danger }

/** The small badge on the service's tile: the action. */
enum class Badge { Pen, Search, Read, List, Send, Trash, Shield, People, Key, Terminal, Question, Cart }

/** A change that cannot be undone or reaches far: asked every time, shown in red. */
fun destructive(view: ApprovalView): Boolean {
    // A desktop question says itself: one asked every time for a key or a command it cannot read is not red.
    view.ask?.let { return it.destructive }
    return view.noStanding && view.kind == ApprovalKind.WRITE && view.purchase == null && !isSecretsMove(view)
}

/** Secrets moved between the computer and the phone: asked every time for its own reasons, not as a danger. */
fun isSecretsMove(view: ApprovalView): Boolean = view.envRelease != null || view.secretsStore != null || view.tokenStore != null

/** The request removes something (a repository, an issue, a branch). */
fun deletes(view: ApprovalView): Boolean =
    view.op.contains("delete") || view.opTitle.startsWith("Delete") || view.mcp?.tool?.startsWith("delete") == true ||
        view.git?.refs?.let { refs -> refs.isNotEmpty() && refs.all { it.change == "delete" } } == true

fun heroTone(view: ApprovalView): HeroTone = when {
    destructive(view) -> HeroTone.Danger
    view.kind == ApprovalKind.GRANT || view.kind == ApprovalKind.ACCOUNTS -> HeroTone.Grant
    view.kind == ApprovalKind.SEND -> HeroTone.Send
    view.kind == ApprovalKind.SEARCH || view.kind == ApprovalKind.READ || view.kind == ApprovalKind.FETCH -> HeroTone.Search
    view.action == "send" -> HeroTone.Send
    view.action in setOf("read", "search", "list") -> HeroTone.Search
    else -> HeroTone.Write
}

fun badge(view: ApprovalView): Badge = when {
    destructive(view) && deletes(view) -> Badge.Trash
    view.ask != null -> Badge.Question
    view.secrets != null || isSecretsMove(view) -> Badge.Key
    view.ssh != null -> Badge.Terminal
    view.purchase != null -> Badge.Cart
    view.kind == ApprovalKind.GRANT -> Badge.Shield
    view.kind == ApprovalKind.ACCOUNTS -> Badge.People
    view.kind == ApprovalKind.SEND -> Badge.Send
    view.kind == ApprovalKind.SEARCH -> Badge.Search
    view.kind == ApprovalKind.READ -> Badge.Read
    view.action == "list" -> Badge.List
    view.action == "search" -> Badge.Search
    view.action == "read" -> Badge.Read
    view.action == "send" -> Badge.Send
    else -> Badge.Pen
}

/** The service whose logo the hero shows ("mcp" for any MCP server); empty for none. */
fun heroService(view: ApprovalView): String = when {
    view.service.startsWith("mcp:") || view.mcp != null -> "mcp"
    else -> view.service
}

/** "Read Telegram messages" → "read Telegram messages"; a name that starts with capitals ("GitHub") stays. */
fun lowerFirst(text: String): String =
    if (text.length > 1 && text[0].isUpperCase() && text[1].isLowerCase()) text[0].lowercase() + text.substring(1) else text

/** "Bank <alerts@bank.com>" → "Bank"; a bare address stays as it is. */
fun displayName(from: String): String {
    val name = from.substringBefore('<').trim().trim('"')
    return name.ifEmpty { from.trim().removePrefix("<").removeSuffix(">") }
}

private fun plural(n: Int, one: String, many: String = one + "s") = if (n == 1) "1 $one" else "$n $many"

/** The hero's one sentence: "Wants to **push 8 commits**\nto **octo/app**". Everything quoted is untrusted. */
fun heroSentence(view: ApprovalView): List<Span> {
    val wants = Span("Wants to ")
    val git = view.git
    val purchase = view.purchase
    val mcp = view.mcp
    return when {
        git != null -> {
            val repo = Span(untrusted(git.repo), true)
            val refs = git.refs
            when {
                refs.isNotEmpty() && refs.all { it.change == "delete" } ->
                    listOf(wants, Span(deleteWords(refs), true), Span("\nfrom "), repo)
                refs.isNotEmpty() && refs.all { it.kind == "tag" } ->
                    listOf(wants, Span(if (refs.size == 1) "push tag ${untrusted(refs[0].shortName)}" else "push ${refs.size} tags", true), Span("\nto "), repo)
                else -> {
                    val commits = refs.filter { it.change != "delete" }.sumOf { it.commitCount.toInt() }
                    val what = if (commits == 0) "push ${untrusted(refs.firstOrNull()?.shortName.orEmpty())}".trim() else "push ${plural(commits, "commit")}"
                    listOf(wants, Span(what, true), Span("\nto "), repo)
                }
            }
        }
        view.ask != null -> listOf(Span(untrusted(view.ask!!.question), true))
        view.secrets != null -> {
            val s = view.secrets!!
            listOf(Span("Wants "), Span(plural(s.items.size, "secret"), true), Span("\nfor "), Span(untrusted(s.command), true))
        }
        view.envRelease != null -> envHero(view.envRelease!!)
        view.secretsStore != null -> listOf(Span("Wants to save "), Span(plural(view.secretsStore!!.total.toInt(), "secret"), true), Span("\nin your "), Span("vault", true))
        view.tokenStore != null -> {
            val t = view.tokenStore!!
            listOf(Span("Wants to save a\n"), Span("${serviceName(t.service)} token", true))
        }
        view.ssh != null -> {
            val s = view.ssh!!
            listOf(wants, Span("sign in", true), Span(" to\n"), Span(untrusted(sshTarget(s.host, s.hostKey)), true))
        }
        purchase != null -> listOf(wants, Span("pay ${purchase.total}", true), Span("\nat "), Span(untrusted(purchase.domain), true))
        mcp != null -> listOf(
            wants,
            Span(lowerFirst(untrusted(mcp.title).ifBlank { untrusted(mcp.tool) }), true),
            Span("\non "),
            Span(untrusted(mcp.serverName), true),
        )
        view.kind == ApprovalKind.GRANT -> listOf(
            wants,
            Span(if (view.grant?.action == "send") "send emails" else "read emails", true),
            Span("\n"),
            Span("without asking", true),
        )
        view.kind == ApprovalKind.ACCOUNTS ->
            listOf(Span("Wants to see your\n"), Span("${serviceName(view.service.ifBlank { "gmail" })} addresses", true))
        view.kind == ApprovalKind.SEND -> {
            val to = view.email?.let { it.to + it.cc }.orEmpty()
            val first = to.firstOrNull()?.let { untrusted(displayName(it)) }
            val rest = if (to.size > 1) " and ${to.size - 1} more" else ""
            listOfNotNull(wants, Span("send an email", true), first?.let { Span("\nto ") }, first?.let { Span(it, true) }, Span(rest).takeIf { rest.isNotEmpty() })
        }
        view.kind == ApprovalKind.SEARCH || view.kind == ApprovalKind.READ -> {
            val n = view.messages.size
            val senders = view.messages.map { displayName(it.from) }.distinct()
            val head = listOf(wants, Span("read ${plural(n, "email")}", true))
            when {
                n > 0 && senders.size == 1 -> head + listOf(Span("\nfrom "), Span(untrusted(senders[0]), true))
                !view.query.isNullOrBlank() -> head + listOf(Span("\nfound for "), Span(untrusted(view.query!!), true))
                else -> head
            }
        }
        else -> {
            val title = lowerFirst(operationTitle(view.action, view.messages.size.coerceAtLeast(1), view.service, view.opTitle, view.op))
            val target = view.resources.firstOrNull { !it.wider }?.label?.let(::untrusted)
            if (title.startsWith("delete ") && target != null) {
                // "Wants to **delete** the repository\n**octo/old-api**": the verb is what matters.
                val service = serviceName(view.service)
                val noun = title.removePrefix("delete ").removePrefix("a ").removePrefix("an ").removePrefix("the ").removePrefix("$service ")
                return listOf(wants, Span("delete", true), Span(" the $noun\n"), Span(target, true))
            }
            listOfNotNull(wants, Span(title, true), target?.let { Span("\n") }, target?.let { Span(it, true) })
        }
    }
}

/** "Wants **3 variables**\nof **shop · dev**"; a printed value: "Wants to **print API_KEY**\nfrom **shop · prod**". */
private fun envHero(r: dev.reins.core.EnvReleaseView): List<Span> {
    val where = Span("${untrusted(r.project)} · ${untrusted(r.env)}", true)
    return when {
        r.print -> listOf(Span("Wants to "), Span("print ${r.variables.singleOrNull()?.let(::untrusted) ?: plural(r.variables.size, "value")}", true), Span("\nfrom "), where)
        r.allVariables -> listOf(Span("Wants "), Span("every variable", true), Span("\nof "), where)
        else -> listOf(Span("Wants "), Span(plural(r.variables.size, "variable"), true), Span("\nof "), where)
    }
}

private fun deleteWords(refs: List<GitRefView>): String = when {
    refs.size > 1 -> "delete ${refs.size} refs"
    refs[0].kind == "tag" -> "delete tag ${untrusted(refs[0].shortName)}"
    else -> "delete branch ${untrusted(refs[0].shortName)}"
}

/** The items a search or read releases as the draft stands (ticked, already covered, or all mail but codes). */
fun sharedCount(view: ApprovalView, draft: ApprovalDraft): Int = view.messages.count { m ->
    m.coveredByGrant || m.id in draft.selected || (draft.allMail != null && !m.sensitive)
}

/** What Approve says it does: "Approve", "Share 2", "Allow", "Yes", "Send", "Delete". */
fun approveLabel(view: ApprovalView, draft: ApprovalDraft): String = when {
    view.ask != null -> "Yes"
    view.kind == ApprovalKind.GRANT || view.kind == ApprovalKind.ACCOUNTS -> "Allow"
    destructive(view) && deletes(view) -> "Delete"
    view.kind == ApprovalKind.SEND -> "Send"
    view.secretsStore != null || view.tokenStore != null -> "Save"
    (view.kind == ApprovalKind.SEARCH || view.kind == ApprovalKind.READ || view.kind == ApprovalKind.FETCH) && view.messages.isNotEmpty() ->
        "Share ${sharedCount(view, draft)}"
    else -> "Approve"
}

fun denyLabel(view: ApprovalView): String = if (view.ask != null) "No" else "Deny"

/** The remembered approval is "anything like this" (the user's explicit choice), wider than the screen shows. */
fun likeThisOn(view: ApprovalView, draft: ApprovalDraft): Boolean =
    view.envRelease == null && draft.likeThis && view.quick?.allow != null && draft.lifetime != LifetimeKind.ONCE && draft.allMail == null && !view.noStanding

/** Nothing here may be remembered: the dock says "Asked every time" instead of offering how long. */
fun rememberLocked(view: ApprovalView, draft: ApprovalDraft = ApprovalDraft()): Boolean =
    view.noStanding || view.purchase != null || (view.envRelease != null && !envRememberable(view, draft))

/** 3600 → "1 hour", 28800 → "8 hours", 86400 → "1 day", 604800 → "1 week", 2592000 → "30 days". */
fun periodWords(secs: Long): String = when {
    secs % 604_800L == 0L && secs / 604_800L in 1..4 -> plural((secs / 604_800L).toInt(), "week")
    secs % 86_400L == 0L -> plural((secs / 86_400L).toInt(), "day")
    secs % 3_600L == 0L -> plural((secs / 3_600L).toInt(), "hour")
    else -> plural((secs / 60L).toInt().coerceAtLeast(1), "minute")
}

/** The dock's second segment: the core's "allow for a while" period ("1 hour", "8 hours"), else an hour. */
fun shortLabel(view: ApprovalView): String = periodWords(lifetimeSeconds(view, ApprovalDraft(lifetime = LifetimeKind.HOUR))!!.toLong())

/** What the last segment says: "Custom" until something is picked in it, then what ("1 week", "3 days", "Forever"). */
fun customLabel(draft: ApprovalDraft): String = when {
    draft.allMail != null -> "All mail"
    else -> sheetChoiceLabel(draft) ?: "Custom"
}

/** The lifetime picked in the "Remember for" sheet in a few words; null for the dock's own (once, 1 hour, 1 day). */
fun sheetChoiceLabel(draft: ApprovalDraft): String? = when (draft.lifetime) {
    LifetimeKind.WEEK -> "1 week"
    LifetimeKind.MONTH -> "30 days"
    LifetimeKind.CUSTOM -> draft.customUnit.label(draft.customAmount)
    LifetimeKind.USES -> plural(draft.uses, "use")
    LifetimeKind.UNTIL_REVOKED -> "Forever"
    else -> null
}

/** The lifetimes the last segment's sheet offers (until revoked is set apart, see [offersUntilRevoked]). */
fun sheetLifetimes(view: ApprovalView): List<LifetimeKind> = when (view.kind) {
    ApprovalKind.ACCOUNTS -> listOf(LifetimeKind.WEEK, LifetimeKind.MONTH)
    else -> listOf(LifetimeKind.WEEK, LifetimeKind.MONTH, LifetimeKind.CUSTOM, LifetimeKind.USES)
}

fun offersUntilRevoked(view: ApprovalView): Boolean = view.kind != ApprovalKind.ACCOUNTS

/** The sheet's confirm button for [draft]'s lifetime: "Remember for 1 week", "Remember until I revoke". */
fun rememberButton(view: ApprovalView, draft: ApprovalDraft): String = when (draft.lifetime) {
    LifetimeKind.UNTIL_REVOKED -> "Remember until I revoke"
    LifetimeKind.ONCE -> "Done"
    LifetimeKind.HOUR -> "Remember for ${shortLabel(view)}"
    LifetimeKind.DAY -> "Remember for 1 day"
    else -> "Remember for ${sheetChoiceLabel(draft)}"
}

/** The dock's segments, in order, for [view] (the last opens the sheet). */
enum class Segment { Once, Short, Day, Custom }

fun segmentOf(draft: ApprovalDraft): Segment = when {
    draft.allMail != null -> Segment.Custom
    draft.lifetime == LifetimeKind.ONCE -> Segment.Once
    draft.lifetime == LifetimeKind.HOUR -> Segment.Short
    draft.lifetime == LifetimeKind.DAY -> Segment.Day
    else -> Segment.Custom
}

/** "+412 −96"; a count not made is left out. */
fun countsLabel(additions: ULong?, deletions: ULong?): String =
    listOfNotNull(additions?.let { "+$it" }, deletions?.let { "−$it" }).joinToString(" ")

/** `src/auth/login.ts` → (`src/auth/`, `login.ts`). */
fun splitPath(path: String): Pair<String, String> {
    val cut = path.lastIndexOf('/')
    return if (cut < 0) "" to path else path.substring(0, cut + 1) to path.substring(cut + 1)
}

/** "Ada Lovelace <ada@example.com>" → "Ada Lovelace". */
fun authorName(author: String): String = displayName(author)

/**
 * What the dock says a remembered release of an environment covers when it is wider than this run: the command pattern
 * ("npm run *"), and "every variable" when chosen; null when it is just this command and these variables.
 */
fun envWiderLabel(view: ApprovalView, draft: ApprovalDraft): String? {
    val release = view.envRelease ?: return null
    if (draft.lifetime == LifetimeKind.ONCE || !envRememberable(view, draft)) return null
    val command = envCommandOf(view, draft)?.takeIf { it.id != "exact" }
    val every = draft.envAllVariables && !release.allVariables
    if (command == null && !every) return null
    return listOfNotNull(command?.let { untrusted(it.label) }, "every variable".takeIf { every }).joinToString(" · ")
}
