package dev.reins.android.ui.common

import dev.reins.android.design.ActionKind
import dev.reins.android.design.Field
import dev.reins.android.design.serviceName
import dev.reins.core.ActivityEntry
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.GitPushView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.RequestLabel

// How a request is written (design/signal/ASK-TEXT.md), in one place for every surface: the waiting field, waiting
// cards, Activity rows, entry pages, notifications and widgets. Screens never put the sentence together themselves.
//
//   kind     GITHUB · PUSH                              mono capitals, above
//   text     Push 3 commits to katulevskiy/reins        serif roman; who + verb + count
//   target                     ^^^^^^^^^^^^^^^^^        the one italic run: short and human
//   details  feature/x · new branch                     mono, its own line: commands, branches, hosts, paths
//   full     the whole command                          a wrapped mono fact row, never cut
//
// Waiting requests are present tense and start with the verb (the asker is shown apart); decided ones are past tense
// ("Claude Code pushed 3 commits to …"), and a refused one says what was asked ("Asked to push …"). Everything that
// came from outside goes through [untrusted]; nothing that is approved is shortened.

/** When the request is told: still waiting, done, or refused (denied, failed). */
enum class Tense { Waiting, Done, Refused }

/**
 * A request in words. [target] is a part of [text] (or null), set in italic; [details] is the mono line under the
 * sentence; [full] the whole command for a wrapped mono fact row. [provider] and [action] pick its icon
 * ([dev.reins.android.design.IntegrationIcon]).
 */
data class AskSentence(
    val kind: String,
    val text: String,
    val target: String?,
    val details: String? = null,
    val full: String? = null,
    val provider: String = "",
    val action: String = "",
)

/** The sentence before its tense: [head] starts with the verb and runs up to the [target]; [after] follows it. */
private data class Phrase(
    val head: String,
    val target: String? = null,
    val after: String = "",
    val details: String? = null,
    val full: String? = null,
    /** Told as it is in any tense: a question from the computer, an access that ended. */
    val question: Boolean = false,
)

// ---- from the core's label (waiting cards, rows, notifications, widgets) -------------------------------------------

/**
 * A request from the core's [RequestLabel]. [cwd] (where a command ran), [full] (the whole command), [matched] (the
 * part a rule matched) and [git] (the push) add what the label leaves out.
 */
fun askSentence(
    label: RequestLabel,
    tense: Tense = Tense.Waiting,
    who: String? = null,
    cwd: String? = null,
    full: String? = null,
    matched: String? = null,
    git: GitPushView? = null,
): AskSentence {
    val provider = label.provider
    val action = label.action
    val lead = oneLine(untrusted(label.lead))
    val subject = untrusted(label.subject)
    val tail = oneLine(untrusted(label.tail))
    val phrase = when {
        provider == "terminal" -> commandPhrase(untrusted(full?.takeIf { it.isNotBlank() } ?: label.subject), cwd, matched)
        provider == "file" -> filePhrase(lead.ifBlank { "Read" }, subject, moreOf(tail))
        provider == "secrets" && subject.isNotEmpty() -> Phrase(withSpace(lead), program(subject), details = subject, full = subject)
        provider == "ssh" && subject.isNotEmpty() -> Phrase("Sign in to a server", details = subject)
        provider.startsWith("mcp:") -> mcpPhrase(oneLine(subject), tail.removePrefix("on ").trim())
        provider == "desktop" && action == "ask" -> Phrase(lead, question = true)
        action == "push" || (action == "delete" && GIT_DELETE.matches(lead)) -> gitPhrase(lead, subject, git)
        provider == "gmail" && action == "send" -> sendPhrase(subject, moreOf(tail))
        lead.startsWith("Share a file") -> if (subject.isEmpty()) Phrase("Share a file") else Phrase("Share ", whole(subject))
        lead.endsWith(":") -> Phrase(if (subject.isEmpty()) lead.removeSuffix(":") else "$lead ${oneLine(subject)}")
        subject.isEmpty() -> Phrase(joinTail(lead, tail))
        else -> subjectPhrase(lead, subject, label.code, tail)
    }
    return phrase.told(tense, who, kindOf(provider, action), provider, action)
}

/** A waiting request: from its approval view when it has been read, else from its label. */
fun askSentence(item: PendingItem, view: ApprovalView? = null, tense: Tense = Tense.Waiting): AskSentence = when (item.kind) {
    PendingKind.PAIRING -> AskSentence("Reins · Connect", "Connect ${quotedName(item.connectionLabel)}", null, provider = "desktop", action = "pair")
    PendingKind.JOIN -> AskSentence("Reins · New phone", "Add ${quotedName(item.connectionLabel)} to your account", null, provider = "desktop", action = "join")
    PendingKind.BLOB -> {
        // The file's name: the core's label, else what the title says after "share a file: ".
        val name = whole(item.label?.subject?.takeIf { it.isNotBlank() } ?: item.title.substringAfter("share a file: ", ""))
        if (name.isEmpty()) AskSentence("Reins · File", "Share a file", null, provider = "files", action = "send")
        else AskSentence("Reins · File", "Share $name", name, provider = "files", action = "send")
    }
    else -> when {
        view != null -> askSentence(view, tense)
        item.label != null -> askSentence(item.label!!, tense)
        else -> legacyPending(item, tense)
    }
}

/** A waiting request from before labels, without its view: what its title and subtitle say. */
private fun legacyPending(item: PendingItem, tense: Tense): AskSentence {
    val s = dev.reins.android.ui.activity.pendingSentence(item, null)
    val plain = s.plain
    val text = when {
        plain.startsWith("Wants to ") -> plain.removePrefix("Wants to ")
        plain.startsWith("Asks for ") -> "Ask for " + plain.removePrefix("Asks for ")
        else -> plain
    }
    val bold = s.parts.filter { it.bold }
    val target = bold.lastOrNull()?.text?.takeIf { bold.size >= 2 && it.isNotBlank() }
    val at = target?.let { text.lastIndexOf(it) } ?: -1
    val phrase = when {
        target == null || at < 0 -> Phrase(text)
        looksMachine(target) -> Phrase(joinTail(dropPreposition(text.substring(0, at)), text.substring(at + target.length).trim()), details = target)
        else -> Phrase(text.substring(0, at), target, text.substring(at + target.length))
    }
    val (svc, a) = legacyAction(item.service, item.action, item.op)
    return phrase.told(tense, null, "${serviceName(svc)} · ${kindWord(a, svc)}", svc, a)
}

private fun legacyAction(service: String, action: String, op: String): Pair<String, String> {
    val svc = service.ifBlank { "gmail" }
    return svc to when (ActionKind.of(action)) {
        ActionKind.Search, ActionKind.List -> "search"
        ActionKind.Read, ActionKind.Headlines, ActionKind.Upload -> "read"
        ActionKind.Send -> "send"
        ActionKind.Grant -> "grant"
        ActionKind.Accounts -> "accounts"
        else -> if (op == "git_push" || op == "git_tag_push") "push" else "write"
    }
}

/**
 * How an entry is told: done only when it was carried out (released, sent, granted); anything else (denied, failed,
 * timed out, cancelled, an outcome this build does not know) says what was asked, never "ran" or "sent".
 */
fun entryTense(outcome: String): Tense = if (outcome in DONE_OUTCOMES) Tense.Done else Tense.Refused

private val DONE_OUTCOMES = setOf("released", "sent", "granted")

/** A decided entry, in the past tense. [who] starts the sentence where the asker is not shown beside it. */
fun askSentence(entry: ActivityEntry, who: String? = null): AskSentence {
    val tense = entryTense(entry.outcome)
    val label = entry.label
    if (label != null) {
        val request = entry.request
        return askSentence(label, tense, who, cwd = request?.cwd, full = request?.full?.takeIf { label.provider == "terminal" })
    }
    return legacy(entry.service, entry.action, entry.count.toInt(), entry.opTitle, entry.op, tense, "", entry)
}

// ---- from the approval view (the waiting field, an entry's page) ----------------------------------------------------

/** A request from what its approval shows: the richest form (branches, the rule's match, the folder). */
fun askSentence(view: ApprovalView, tense: Tense = Tense.Waiting, who: String? = null): AskSentence {
    val git = view.git
    val ask = view.ask
    val mcp = view.mcp
    val purchase = view.purchase
    val service = view.service.ifBlank { "gmail" }
    val (phrase, provider, action) = when {
        git != null -> {
            val repo = untrusted(git.repo)
            val refs = git.refs
            val lead = when {
                refs.isNotEmpty() && refs.all { it.change == "delete" } ->
                    if (refs.size == 1) "Delete ${refs[0].kind} ${untrusted(refs[0].shortName)} from" else "Delete ${refs.size} refs from"
                refs.isNotEmpty() && refs.all { it.kind == "tag" } ->
                    if (refs.size == 1) "Push tag ${untrusted(refs[0].shortName)} to" else "Push ${refs.size} tags to"
                else -> {
                    val commits = refs.filter { it.change != "delete" }.sumOf { it.commitCount.toInt() }
                    val verb = if (refs.any { it.force }) "Force-push" else "Push"
                    if (commits == 0) "$verb to" else "$verb ${plural(commits, "commit")} to"
                }
            }
            val deletes = refs.isNotEmpty() && refs.all { it.change == "delete" }
            Triple(gitPhrase(lead, repo, git), view.service, if (deletes) "delete" else "push")
        }
        ask != null -> {
            val hook = hookOf(ask.question, ask.detail)
            when {
                hook?.command != null -> Triple(
                    // The match is the guard's own (the rule's), never a guess from the text.
                    commandPhrase(untrusted(hook.command), hook.cwd, ask.rule?.matched, uncertain = ask.uncertain),
                    "terminal",
                    if (ask.destructive) "delete" else "run",
                )
                hook != null && hook.files.isNotEmpty() -> Triple(
                    filePhrase(
                        if (ask.uncertain) null else when (hook.write) { true -> "Change"; false -> "Read"; null -> null },
                        untrusted(hook.files[0]),
                        hook.files.size - 1,
                    ),
                    "file",
                    if (hook.write == false) "read" else "write",
                )
                else -> Triple(Phrase(untrusted(ask.question), question = true), "desktop", "ask")
            }
        }
        view.secrets != null -> {
            val s = view.secrets!!
            val command = untrusted(s.command)
            val what = "Use ${plural(s.items.size, "secret")}"
            Triple(if (command.isEmpty()) Phrase(what) else Phrase("$what for ", program(command), details = command, full = command), "secrets", "secrets")
        }
        view.envRelease != null -> {
            val r = view.envRelease!!
            val where = "${untrusted(r.project)} · ${untrusted(r.env)}"
            val names = r.variables.joinToString(", ") { untrusted(it) }.ifEmpty { null }
            val phrase = when {
                r.print -> Phrase(if (r.variables.size == 1) "Print a value from " else "Print ${plural(r.variables.size, "value")} from ", where, details = names)
                r.allVariables -> Phrase("Use every variable of ", where)
                else -> Phrase("Use ${plural(r.variables.size, "variable")} of ", where, details = names)
            }
            Triple(phrase, "secrets", "secrets")
        }
        view.secretsStore != null -> Triple(Phrase("Save ${plural(view.secretsStore!!.total.toInt(), "secret")} in your ", "vault"), "vault", "write")
        view.tokenStore != null -> Triple(Phrase("Save a ", "${serviceName(view.tokenStore!!.service)} token"), "vault", "write")
        view.ssh != null -> Triple(Phrase("Sign in to a server", details = untrusted(dev.reins.android.ui.approval.sshTarget(view.ssh!!.host, view.ssh!!.hostKey))), "ssh", "secrets")
        purchase != null -> {
            // The store is its domain, as Reins read it from the checkout; the merchant's name is the page's own words.
            // The one host set in the serif: here it is the thing the money goes to.
            Triple(Phrase("Pay ${purchase.total} at ", oneLine(untrusted(purchase.domain))), "payments", "pay")
        }
        mcp != null -> {
            val action = if (mcp.destructive) "delete" else if (mcp.readOnly) "read" else "write"
            Triple(mcpPhrase(oneLine(untrusted(mcp.tool)), oneLine(untrusted(mcp.serverName))), view.service.ifBlank { "mcp:" }, action)
        }
        view.kind == ApprovalKind.GRANT -> Triple(Phrase(if (view.grant?.action == "send") "Send emails without asking" else "Read emails without asking"), service, "grant")
        view.kind == ApprovalKind.ACCOUNTS -> Triple(Phrase("See your ${serviceName(service)} addresses"), service, "accounts")
        view.kind == ApprovalKind.SEND -> {
            val to = view.email?.let { it.to + it.cc }.orEmpty()
            Triple(sendPhrase(to.firstOrNull()?.let(::untrusted).orEmpty(), (to.size - 1).coerceAtLeast(0)), service, "send")
        }
        view.kind == ApprovalKind.SEARCH || view.kind == ApprovalKind.READ -> {
            val n = view.messages.size
            val senders = view.messages.map { untrusted(it.from) }.distinctBy { dev.reins.android.ui.approval.displayName(it) }
            val query = view.query?.let(::untrusted)?.takeIf { it.isNotBlank() }
            val phrase = when {
                n > 0 && senders.size == 1 -> personPhrase("Read ${plural(n, "email")} from", senders[0])
                n == 0 && query != null -> forPhrase("Search ${serviceName(service)} for", query)
                query != null -> forPhrase("Read ${plural(n, "email")} found for", query)
                else -> Phrase("Read ${plural(n, "email")}")
            }
            Triple(phrase, service, if (n == 0 && query != null) "search" else "read")
        }
        else -> {
            val title = operationTitle(view.action, view.messages.size.coerceAtLeast(1), view.service, untrusted(view.opTitle), view.op)
            val target = view.resources.firstOrNull { !it.wider }?.label?.let(::untrusted)?.takeIf { it.isNotBlank() }
            val deletes = dev.reins.android.ui.approval.deletes(view)
            val phrase = when {
                target == null -> Phrase(title)
                looksMachine(target) -> Phrase(title, details = target)
                deletes && title.startsWith("Delete ") -> {
                    // "Delete the repository *octo/old-api*": the noun stays roman, the thing is the target.
                    val noun = title.removePrefix("Delete ").removePrefix("a ").removePrefix("an ").removePrefix("the ").removePrefix("${serviceName(view.service)} ")
                    Phrase("Delete the $noun ", target)
                }
                else -> Phrase("$title in ", target)
            }
            Triple(phrase, view.service, actionOfView(view, deletes))
        }
    }
    val kind = when {
        ask != null && provider != "desktop" -> kindOf(provider, action)
        else -> viewKind(view)
    }
    return phrase.told(tense, who, kind, provider, action)
}

// ---- the parts ------------------------------------------------------------------------------------------------------

/**
 * "Run a command in *reins*", the guard's match and "and 2 more commands" in mono, the whole command in full. [uncertain]
 * (the phone could not read it with certainty): the generic sentence, and the whole command under it.
 */
private fun commandPhrase(command: String, cwd: String?, matched: String?, uncertain: Boolean = false): Phrase {
    val whole = command.trim().ifEmpty { null }
    if (uncertain) return Phrase("Run a command", details = whole, full = whole)
    val folder = cwd?.let(::folderName)
    val line = commandLine(command, matched)
    return Phrase(
        if (folder != null) "Run a command in " else "Run a command",
        folder,
        details = line.ifEmpty { null },
        full = whole,
    )
}

/**
 * "Read *.env*", "Change *a.pem* and 1 more file"; the path in mono. [verb] null: it is not known whether the files are
 * read or changed, so the sentence says neither.
 */
private fun filePhrase(verb: String?, path: String, more: Int): Phrase {
    val p = path.trim()
    if (verb == null) return Phrase("Use ${plural(more + 1, "file")}", details = if (more > 0) "$p and $more more" else p.ifEmpty { null })
    if (p.isEmpty()) return Phrase("$verb ${if (more > 0) plural(more + 1, "file") else "a file"}")
    return Phrase("$verb ", fileName(p), if (more > 0) " and $more more ${if (more == 1) "file" else "files"}" else "", details = p)
}

/**
 * A tool of an MCP server: "Use a tool on *Linear*", the tool in mono. The verb is never the tool's own title: that is
 * the server's words, not what Reins knows the call does.
 */
private fun mcpPhrase(tool: String, server: String): Phrase {
    val on = server.ifBlank { null }
    return Phrase(if (on != null) "Use a tool on " else "Use a tool", on, details = tool.ifBlank { null })
}

private val GIT_DELETE = Regex("^Delete (branch|tag|\\d+ refs) .*from$|^Delete \\d+ refs from$")

/** A push or a deletion on a repository: the repository is the target, branches and tags go to the mono line. */
private fun gitPhrase(lead: String, repo: String, git: GitPushView?): Phrase {
    val names = git?.refs?.joinToString(", ") { untrusted(it.shortName) }?.ifEmpty { null }
    val one = Regex("^Delete (branch|tag) (.+) from$").find(lead)
    val tag = Regex("^Push tag (.+) to$").find(lead)
    val (head, details) = when {
        one != null -> "Delete a ${one.groupValues[1]} from" to one.groupValues[2]
        tag != null -> "Push a tag to" to tag.groupValues[1]
        lead.startsWith("Delete ") || lead.endsWith(" tags to") -> lead to names
        else -> lead to pushDetails(git)
    }
    return if (repo.isBlank()) Phrase(dropPreposition(head), details = details) else Phrase("$head ", repo, details = details)
}

/** Where a push goes, in mono: "feature/x · new branch", "main", "feature/x and 1 more". */
fun pushDetails(git: GitPushView?): String? {
    val refs = git?.refs?.filter { it.change != "delete" }.orEmpty()
    val first = refs.firstOrNull() ?: return null
    val name = buildString {
        append(untrusted(first.shortName))
        if (first.change == "create") append(if (first.kind == "tag") " · new tag" else " · new branch")
    }
    return if (refs.size > 1) "$name and ${refs.size - 1} more" else name
}

/**
 * An email: "Send an email", or "to 3 people", and the recipients as written in mono. The name in "Dana <dana@…>" is
 * the AI's own words, so it is never the target: the address below is what the mail goes to.
 */
private fun sendPhrase(to: String, more: Int): Phrase {
    val first = oneLine(to).trim()
    if (first.isEmpty()) return Phrase("Send an email")
    val n = more + 1
    return Phrase(if (n == 1) "Send an email" else "Send an email to $n people", details = first + if (more > 0) " and $more more" else "")
}

/** "Read 2 emails from *Stripe*" (the sender's name, as the mail says it), the address under it; a bare address is mono. */
private fun personPhrase(lead: String, from: String): Phrase {
    val name = oneLine(dev.reins.android.ui.approval.displayName(from)).trim()
    val address = if ('<' in from) from.substringAfter('<').substringBefore('>').trim() else from.trim()
    return when {
        name.isBlank() || looksMachine(name) -> Phrase(dropPreposition(lead), details = address)
        else -> Phrase("$lead ", name, details = address.takeIf { it.isNotEmpty() && it != name })
    }
}

/** A label's subject: a human one is the target; a command, URL, path or address moves to the mono line. */
private fun subjectPhrase(lead: String, subject: String, code: Boolean, tail: String): Phrase {
    if ('<' in subject && '>' in subject && !code) return personPhrase(lead, subject).copy(after = spaced(tail))
    if (code || looksMachine(subject)) {
        return Phrase(joinTail(dropPreposition(lead), tail), details = subject, full = if (code && '\n' in subject) subject else null)
    }
    return Phrase(withSpace(lead), oneLine(subject), spaced(tail))
}

/** An entry or a request from before labels: built from what the operation is called. */
private fun legacy(service: String, action: String, count: Int, opTitle: String, op: String, tense: Tense, subtitle: String, entry: ActivityEntry? = null): AskSentence {
    val svc = service.ifBlank { "gmail" }
    val kind = ActionKind.of(action)
    val mail = op.isEmpty() && svc == "gmail"
    val info = entry?.info
    val phrase = when {
        kind == ActionKind.Grant -> {
            // Told as it ended, in any tense: "Got access: …", "Asked for access: …".
            val summary = info?.grantSummary?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { ": ${oneLine(it)}" }.orEmpty()
            val head = when {
                tense == Tense.Waiting -> "Ask for access"
                entry?.outcome == "granted" -> "Got access"
                else -> "Asked for access"
            }
            Phrase(head + summary, question = true)
        }
        mail && kind == ActionKind.Search -> info?.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { forPhrase("Search Gmail for", it) } ?: Phrase("Search Gmail")
        mail && kind == ActionKind.Read -> {
            val from = info?.messages?.map { untrusted(it.from) }?.distinctBy { dev.reins.android.ui.approval.displayName(it) }?.singleOrNull()
            val lead = "Read ${plural(count.coerceAtLeast(1), "email")}"
            if (from != null) personPhrase("$lead from", from) else Phrase(lead)
        }
        mail && kind == ActionKind.Send -> {
            val to = info?.email?.let { it.to + it.cc }.orEmpty()
            sendPhrase(to.firstOrNull()?.let(::untrusted).orEmpty(), (to.size - 1).coerceAtLeast(0))
        }
        else -> {
            val title = operationTitle(action, count, service, untrusted(opTitle), op)
            val where = if (svc.startsWith(dev.reins.android.design.MCP_PREFIX)) "" else subtitle
            if (where.isNotBlank() && !looksMachine(where) && !where.equals(title, ignoreCase = true)) Phrase("$title in ", where) else Phrase(title)
        }
    }
    val a = legacyAction(svc, action, op).second
    return phrase.told(tense, null, "${serviceName(svc)} · ${kindWord(a, svc)}", svc, a)
}

/** Puts the tense on: present from the verb; past, with [who] first when given; refused as what was asked. */
private fun Phrase.told(tense: Tense, who: String?, kind: String, provider: String, action: String): AskSentence {
    val target = target?.trim()?.takeIf { it.isNotEmpty() }
    val body = (head + (target ?: "") + after).replace(Regex(" {2,}"), " ").trim()
    val name = who?.let { untrustedName(it) }?.takeIf { it.isNotEmpty() }
    val text = when {
        question -> body
        tense == Tense.Waiting -> body.replaceFirstChar { it.uppercase() }
        tense == Tense.Done -> pastTense(body).let { if (name != null) "$name ${lowerVerb(it)}" else it.replaceFirstChar { c -> c.uppercase() } }
        else -> (if (name != null) "$name asked to " else "Asked to ") + lowerVerb(body)
    }
    return AskSentence(kind, text, target?.takeIf { it in text }, details?.trim()?.ifEmpty { null }, full, provider, action)
}

// ---- words ----------------------------------------------------------------------------------------------------------

private val PAST_VERBS = mapOf(
    "run" to "ran", "read" to "read", "send" to "sent", "see" to "saw", "get" to "got", "write" to "wrote", "buy" to "bought",
    "pay" to "paid", "sign" to "signed", "make" to "made", "find" to "found", "put" to "put", "set" to "set", "take" to "took",
    "give" to "gave", "begin" to "began", "label" to "labelled", "cancel" to "cancelled", "stop" to "stopped", "ask" to "asked",
)

/** "Push 3 commits to" → "Pushed 3 commits to", "Force-push" → "Force-pushed", "Run" → "Ran". */
fun pastTense(phrase: String): String {
    val first = phrase.substringBefore(' ')
    val verb = first.substringAfterLast('-')
    if (verb.isEmpty() || !verb.all { it.isLetter() }) return phrase
    val lower = verb.lowercase()
    val past = PAST_VERBS[lower] ?: when {
        lower.endsWith("e") -> lower + "d"
        lower.length > 1 && lower.endsWith("y") && lower[lower.length - 2] !in "aeiou" -> lower.dropLast(1) + "ied"
        else -> lower + "ed"
    }
    val cased = if (verb[0].isUpperCase()) past.replaceFirstChar { it.uppercase() } else past
    return first.dropLast(verb.length) + cased + phrase.substring(first.length)
}

/** The verb in lower case, a name that starts the phrase ("GitHub …") kept. */
private fun lowerVerb(s: String): String = if (s.length > 1 && s[1].isUpperCase()) s else s.replaceFirstChar { it.lowercase() }

/** The kind line: what it touches, then what it does ("GitHub · Push", "Terminal · Run"). */
fun kindOf(provider: String, action: String): String = "${providerTitle(provider)} · ${kindWord(action, provider)}"

private fun providerTitle(provider: String): String = when (provider) {
    "terminal" -> "Terminal"
    "file", "files" -> "Files"
    "secrets" -> "Vault"
    "ssh" -> "SSH"
    "desktop" -> "Computer"
    else -> serviceName(provider)
}

private fun kindWord(action: String, provider: String): String = when (action) {
    "run" -> "Run"
    "read" -> "Read"
    "write" -> "Change"
    "send" -> if (provider == "files") "Share" else "Send"
    "delete" -> "Delete"
    "push" -> "Push"
    "secrets" -> if (provider == "ssh") "Sign in" else "Secrets"
    "search" -> "Search"
    "grant" -> "Access"
    "accounts" -> "Accounts"
    "ask" -> "Asks"
    "pay" -> "Buy"
    else -> action.replaceFirstChar { it.uppercase() }
}

/** The approval view's kind line ("GitHub · Push", "Payments · Buy", "Vault · Secrets"). */
private fun viewKind(view: ApprovalView): String {
    val mcp = view.mcp
    val where = when {
        mcp != null -> untrustedName(mcp.serverName)
        view.purchase != null -> "Payments"
        view.ask != null -> "Computer"
        view.secrets != null || dev.reins.android.ui.approval.isSecretsMove(view) -> "Vault"
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
        dev.reins.android.ui.approval.deletes(view) -> "Delete"
        view.git != null -> "Push"
        view.kind == ApprovalKind.SEND -> "Send"
        view.kind == ApprovalKind.SEARCH -> "Search"
        view.kind == ApprovalKind.READ -> "Read"
        view.kind == ApprovalKind.FETCH -> "Fetch"
        else -> when (view.action) {
            "search" -> "Search"
            "read" -> "Read"
            "list" -> "List"
            "send" -> "Send"
            else -> "Change"
        }
    }
    return "$where · $what"
}

private fun actionOfView(view: ApprovalView, deletes: Boolean): String = when {
    deletes -> "delete"
    view.kind == ApprovalKind.FETCH || view.action == "read" -> "read"
    view.action == "search" || view.action == "list" -> "search"
    view.action == "send" -> "send"
    else -> "write"
}

/** The Signal field of a request's action: its badge's colour. */
fun actionField(action: String): Field = when (action) {
    "read", "search", "ask" -> Field.Read
    "delete", "pay" -> Field.Delete
    "grant" -> Field.Grant
    "secrets", "accounts" -> Field.Accounts
    "pair", "join" -> Field.Pair
    else -> Field.Push
}

// ---- machine values -------------------------------------------------------------------------------------------------

private val EMAIL = Regex("^[^\\s@<>]+@[^\\s@<>]+\\.[^\\s@<>]+$")
private val HOST = Regex("^[A-Za-z0-9-]+(\\.[A-Za-z0-9-]+)+(:\\d+)?$")

/** A URL, a path, an address or a host: machine text, kept out of the serif. */
fun looksMachine(s: String): Boolean {
    val t = s.trim()
    return "://" in t || t.startsWith("/") || t.startsWith("~") || t.startsWith("./") || '\n' in t || EMAIL.matches(t) || HOST.matches(t) ||
        OPERATOR.containsMatchIn(t)
}

/** A search's operators ("from:bank newer_than:7d"): machine text too. */
private val OPERATOR = Regex("(^|\\s)[A-Za-z_]+:\\S")

/** "Search Gmail for *invoice*"; a query of operators goes to the mono line ("Search Gmail", `from:bank`). */
private fun forPhrase(lead: String, value: String): Phrase =
    if (looksMachine(value)) Phrase(dropPreposition(lead), details = value) else Phrase("$lead ", oneLine(value))

/** The commands of a command line: split at `&&`, `||`, `;` and new lines. */
fun commandParts(command: String): List<String> =
    command.replace(Regex("[ \\t]*\\\\\\r?\\n\\s*"), " ").split(Regex("&&|\\|\\||;|\n")).map { it.trim() }.filter { it.isNotEmpty() }

/** Quotes, a subshell, a here-document or a variable: where `;` and `&&` may not split commands. */
private val UNSURE_SPLIT = Regex("[\"'`]|\\$\\(|<<|\\$\\{")

/**
 * "rm -rf jarx and 2 more commands": the part the guard matched (its own match, else the first command) and how many
 * more there are. A line the phone cannot split with certainty (quotes, subshells) is shown whole, without a count.
 */
fun commandLine(command: String, matched: String?): String {
    val whole = command.trim()
    if (UNSURE_SPLIT.containsMatchIn(whole)) return whole
    val parts = commandParts(command)
    if (parts.isEmpty()) return whole
    val m = matched?.trim()?.takeIf { it.isNotEmpty() && command.contains(it) }
    val shown = m ?: parts.first()
    val covered = if (m == null) 1 else parts.count { m.contains(it) || it.contains(m) }.coerceAtLeast(1)
    val more = parts.size - covered
    return if (more > 0) "$shown and $more more ${if (more == 1) "command" else "commands"}" else shown
}

/** The folder a command ran in, by its name ("/home/dana/dev/reins" → "reins"); null for none or the home folder. */
fun folderName(cwd: String): String? {
    val t = whole(cwd).trimEnd('/', '\\')
    val name = t.substringAfterLast('/').substringAfterLast('\\')
    return name.takeIf { it.isNotEmpty() && it != "~" }
}

/** The program a command starts ("FOO=1 ./scripts/deploy.sh --prod" → "deploy.sh"). */
fun program(command: String): String? {
    val word = command.trim().split(Regex("\\s+")).firstOrNull { it.isNotEmpty() && '=' !in it && it != "sudo" } ?: return null
    return whole(word.substringAfterLast('/')).ifEmpty { null }
}

private fun fileName(path: String): String = whole(path.trimEnd('/').substringAfterLast('/').ifEmpty { path })

/**
 * A target as given, on one line, cleaned and never shortened: a long one is cut only where it is drawn, and the page
 * shows it whole.
 */
private fun whole(s: String): String = oneLine(untrusted(s)).trim()

/**
 * What a desktop question holds, read back from its detail's own blocks ("Command:", "Files:", "In:", "Rule:"), as
 * the core does. [write]: the files are changed (true) or read (false); null when the question does not say.
 */
data class Hook(val command: String?, val files: List<String>, val write: Boolean?, val cwd: String?, val rule: String?)

/**
 * Only the detail's blocks are read: a question without them stays the question, told as asked. Nothing is taken from
 * the question's words but whether files are read or changed.
 */
fun hookOf(question: String, detail: String?): Hook? {
    var d = detail.orEmpty()
    // A hook's own rule lines come first ("Rule: label · pattern", "Matched: …", "Group: …").
    if (d.startsWith("Rule: ") && d.substringBefore('\n').contains(" · ")) {
        d = d.lines().dropWhile { it.startsWith("Rule: ") || it.startsWith("Matched: ") || it.startsWith("Group: ") }.joinToString("\n")
    }
    val at = d.lastIndexOf("\n\nIn: ").takeIf { it >= 0 } ?: d.lastIndexOf("\nRule: ").takeIf { it >= 0 }
    val body = if (at != null) d.substring(0, at) else d
    val tail = if (at != null) d.substring(at) else ""
    var cwd: String? = null
    var rule: String? = null
    tail.lines().forEach { l ->
        if (l.startsWith("In: ")) cwd = l.removePrefix("In: ").trim()
        if (l.startsWith("Rule: ")) rule = l.removePrefix("Rule: ").trim()
    }
    val command = body.takeIf { it.startsWith("Command:\n") }?.removePrefix("Command:\n")?.trimEnd()?.takeIf { it.isNotBlank() }
    val files = body.takeIf { it.startsWith("Files:\n") }?.removePrefix("Files:\n")?.lines()?.map { it.trim() }?.filter { it.isNotEmpty() }.orEmpty()
    if (command == null && files.isEmpty()) return null
    val q = oneLine(question)
    val write = when {
        " wants to change " in q && " wants to read " !in q -> true
        " wants to read " in q && " wants to change " !in q -> false
        else -> null
    }
    return Hook(command, files, write, cwd?.ifEmpty { null }, rule?.ifEmpty { null })
}

private fun oneLine(s: String): String = s.replace('\n', ' ').replace('\t', ' ')

private fun plural(n: Int, one: String) = if (n == 1) "1 $one" else "$n ${one}s"

private fun moreOf(tail: String): Int = Regex("and (\\d+) more").find(tail)?.groupValues?.get(1)?.toIntOrNull() ?: 0

private fun withSpace(s: String) = if (s.isEmpty() || s.endsWith(' ')) s else "$s "

private fun spaced(tail: String) = if (tail.isEmpty()) "" else " $tail"

private fun joinTail(lead: String, tail: String) = if (tail.isEmpty()) lead else "$lead $tail"

private val PREPOSITIONS = listOf(" for", " in", " to", " from", " on", " at", " of", " with")

/** "Search Gmail for" → "Search Gmail": the subject it led to moved to the mono line. */
private fun dropPreposition(lead: String): String {
    val t = lead.trim()
    return PREPOSITIONS.firstOrNull { t.endsWith(it) }?.let { t.removeSuffix(it) } ?: t
}
