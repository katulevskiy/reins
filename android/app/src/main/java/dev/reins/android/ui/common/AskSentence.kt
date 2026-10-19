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
import java.text.Normalizer

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
// ("Claude Code pushed 3 commits to …") only when they were carried out, and say what was asked otherwise.
//
// What may reach the serif or the kind line (a security rule, ASK-TEXT.md and the desktop review):
// - The verb, the counts, the kind and the badge's colour come from the core's structured fields (its label, the
//   approval view's kinds and fields), never from the AI's or a server's own words, and never from matching the ask.
// - Commands are never parsed here: the core and the guard read them. The mono line shows the guard's own match, else
//   the whole command; the full command is a fact row.
// - Every target is checked before it is set: cleaned of hidden characters and line breaks, NFC, and anything that can
//   pass for something else (letters of two scripts, look-alike letters, compatibility forms, a host that is not
//   ASCII, a repository name the core would refuse, a path that climbs out with "..") leaves the serif: the sentence
//   falls back to its generic words and the mono line shows the raw form, escaped.
// - Nothing is shortened here; a long target is cut only where it is drawn, and the page shows it whole.

/**
 * What kind of value a target is, for the look-alike rule ([lookalikeFlag]). An [Identifier] (a host, a repository,
 * a file or folder, a branch, an MCP server) is normally ASCII: any letter that can pass for another leaves the
 * serif. A [Name] (a person, a contact, a chat, free words) may be wholly Cyrillic, Greek, CJK…: only a name that
 * mixes scripts is flagged; one in a single script is shown as it is.
 */
enum class ValueKind { Identifier, Name }

/** When the request is told: still waiting, done, or refused (denied, failed, timed out). */
enum class Tense { Waiting, Done, Refused }

/**
 * A request in words. [target] is a part of [text] (or null), set in italic; [details] is the mono line under the
 * sentence (a flag about the target first, when there is one); [full] the whole command for a wrapped mono fact row.
 * [provider] and [action] pick its icon ([IntegrationIcon]) and come from the core's fields.
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

/**
 * The sentence before its tense: [head] starts with the verb and runs up to the [target]; [after] follows it. [stand]
 * is what the sentence says instead of the target when the target cannot be shown ("a repository", "a file").
 */
private data class Phrase(
    val head: String,
    val target: String? = null,
    val after: String = "",
    val details: String? = null,
    val full: String? = null,
    /** Told as it is in any tense: a question from the computer, an access that ended. */
    val question: Boolean = false,
    val stand: String? = null,
    /** What kind of value [target] (or a question's words) is: an identifier unless said otherwise. */
    val kind: ValueKind = ValueKind.Identifier,
)

// ---- from the core's label (waiting cards, rows, notifications, widgets) -------------------------------------------

/**
 * A request from the core's [RequestLabel]. [cwd] (where a command ran, as the core kept it), [full] (the whole
 * command), [matched] (the guard's own match) and [git] (the push) add what the label leaves out.
 */
fun askSentence(
    label: RequestLabel,
    tense: Tense = Tense.Waiting,
    who: String? = null,
    cwd: String? = null,
    full: String? = null,
    matched: String? = null,
    git: GitPushView? = null,
    uncertain: Boolean = false,
): AskSentence {
    val provider = label.provider
    val action = label.action
    val lead = serifLine(label.lead)
    val subject = untrusted(label.subject).trim()
    val tail = serifLine(label.tail)
    val phrase = when {
        provider == "terminal" && label.command != null -> Phrase(
            serifLine(label.command!!.heading),
            details = label.command!!.actionable.takeIf { it.isNotBlank() }?.let(::untrusted),
        )
        provider == "terminal" -> commandPhrase(untrusted(full?.takeIf { it.isNotBlank() } ?: label.subject), cwd, matched, uncertain)
        provider == "file" -> filePhrase(lead.ifBlank { "Read" }, subject, tail)
        provider == "secrets" && subject.isNotEmpty() -> Phrase("${dropPreposition(lead)} for a command", details = subject, full = subject)
        provider == "ssh" && subject.isNotEmpty() -> Phrase("Sign in to a server", details = hostLine(subject))
        provider.startsWith("mcp:") -> mcpPhrase(subject, tail.removePrefix("on ").trim(), host = null)
        provider == "desktop" && action == "ask" -> Phrase(lead, question = true, kind = ValueKind.Name)
        // With the push's view (the approval's, or the copy synced with the entry) its count and branches come from it.
        (action == "push" || (action == "delete" && GIT_DELETE.matches(lead))) && !git?.refs.isNullOrEmpty() -> pushPhrase(git!!, subject)
        action == "push" || (action == "delete" && GIT_DELETE.matches(lead)) -> gitPhrase(lead, subject, refLines(git))
        // One verb, so it reads in the past tense too ("Fetched", never "Cloned and fetch"): a clone is a fetch.
        lead == "Clone and fetch" && subject.isNotEmpty() -> subjectPhrase("Fetch", subject, label.code, tail, ValueKind.Identifier)
        provider == "gmail" && action == "send" -> sendPhrase(listOf(subject), moreOf(tail))
        lead.startsWith("Share a file") -> if (subject.isEmpty()) Phrase("Share a file") else Phrase("Share ", subject, stand = "a file")
        lead.endsWith(":") -> if (subject.isEmpty()) Phrase(lead.removeSuffix(":")) else Phrase("$lead ", subject, question = true, stand = "", kind = ValueKind.Name)
        subject.isEmpty() -> Phrase(joinTail(lead, tail))
        else -> subjectPhrase(lead, subject, label.code, tail, if (provider in NAME_PROVIDERS) ValueKind.Name else ValueKind.Identifier)
    }
    val sentence = phrase.told(tense, who, kindOf(provider, action), provider, action)
    if (provider == "terminal" && label.command != null && tense == Tense.Done) {
        // A desktop ask records permission, not an executor's report that the command actually ran.
        val heading = label.command!!.heading
        return sentence.copy(text = if (heading.startsWith("Run ")) "Approved running ${heading.drop(4)}" else "Approved a shell command", target = null)
    }
    return sentence
}

/** A waiting request: from its approval view when it has been read (with the core's label), else from its label. */
fun askSentence(item: PendingItem, view: ApprovalView? = null, tense: Tense = Tense.Waiting): AskSentence = when (item.kind) {
    PendingKind.PAIRING -> AskSentence("Reins · Connect", "Connect ${quotedName(serifLine(item.connectionLabel))}", null, provider = "desktop", action = "pair")
    PendingKind.JOIN -> AskSentence("Reins · New phone", "Add ${quotedName(serifLine(item.connectionLabel))} to your account", null, provider = "desktop", action = "join")
    PendingKind.BLOB -> {
        // The file's name from the core's label; without one, the generic words.
        val name = item.label?.subject?.takeIf { it.isNotBlank() }
        val phrase = if (name == null) Phrase("Share a file") else Phrase("Share ", untrusted(name), stand = "a file")
        phrase.told(tense, null, "Reins · File", "files", "send")
    }
    else -> when {
        view != null -> askSentence(view, tense, label = item.label)
        item.label != null -> askSentence(item.label!!, tense)
        else -> legacyPending(item, tense)
    }
}

/** A waiting request from before labels, without its view: what its kind and service say. */
private fun legacyPending(item: PendingItem, tense: Tense): AskSentence {
    val (svc, a) = legacyAction(item.service, item.action, item.op)
    val count = item.count.toInt()
    val mail = item.op.isEmpty() && svc == "gmail"
    val phrase = when {
        svc.startsWith(dev.reins.android.design.MCP_PREFIX) -> mcpPhrase(untrusted(item.op), serviceName(svc), host = null)
        a == "grant" -> Phrase("Ask for access")
        a == "accounts" -> Phrase("See your accounts")
        mail && a == "search" -> untrusted(item.subtitle).takeIf { it.isNotBlank() }?.let { forPhrase("Search Gmail for", it) } ?: Phrase("Search Gmail")
        mail && a == "read" -> Phrase("Read ${plural(count.coerceAtLeast(1), "email")}")
        mail && a == "send" -> Phrase(if (count > 1) "Send an email to $count people" else "Send an email")
        else -> Phrase(operationTitle(item.action, count, item.service, untrusted(item.opTitle), item.op))
    }
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
        // The branch line comes only from the push's structured view, kept and synced with the entry; never the text.
        return askSentence(label, tense, who, cwd = request?.cwd, full = request?.full?.takeIf { label.provider == "terminal" }, git = request?.git)
    }
    return legacy(entry, tense)
}

// ---- from the approval view (the waiting field, an entry's page) ----------------------------------------------------

/**
 * A request from what its approval shows: the branches, the guard's match, the store. A desktop question is read
 * through the core's [label] (its reading of the command or the files); without it, it is told as asked. [cwd] is
 * where a command ran, as the core kept it.
 */
fun askSentence(
    view: ApprovalView,
    tense: Tense = Tense.Waiting,
    who: String? = null,
    label: RequestLabel? = null,
    cwd: String? = null,
): AskSentence {
    val git = view.git
    val ask = view.ask
    val mcp = view.mcp
    val purchase = view.purchase
    val service = view.service.ifBlank { "gmail" }
    if (ask != null) {
        // The core read the question (its label); the phone never re-reads the command or the files.
        return when {
            label != null && (label.provider == "terminal" || label.provider == "file") ->
                askSentence(label, tense, who, cwd = cwd, matched = ask.rule?.matched, uncertain = ask.uncertain)
            else -> Phrase(untrusted(ask.question), question = true, kind = ValueKind.Name).told(tense, who, viewKind(view), "desktop", "ask")
        }
    }
    val (phrase, provider, action) = when {
        git != null -> {
            val deletes = git.refs.isNotEmpty() && git.refs.all { it.change == "delete" }
            Triple(pushPhrase(git, untrusted(git.repo)), view.service, if (deletes) "delete" else "push")
        }
        view.secrets != null -> {
            val s = view.secrets!!
            val command = untrusted(s.command).trim()
            val what = "Use ${plural(s.items.size, "secret")}"
            Triple(if (command.isEmpty()) Phrase(what) else Phrase("$what for a command", details = command, full = command), "secrets", "secrets")
        }
        view.envRelease != null -> {
            val r = view.envRelease!!
            val where = "${untrusted(r.project)} · ${untrusted(r.env)}"
            val names = r.variables.joinToString(", ") { serifLine(it) }.ifEmpty { null }
            val phrase = when {
                r.print -> Phrase(if (r.variables.size == 1) "Print a value from " else "Print ${plural(r.variables.size, "value")} from ", where, details = names, stand = "a project")
                r.allVariables -> Phrase("Use every variable of ", where, stand = "a project")
                else -> Phrase("Use ${plural(r.variables.size, "variable")} of ", where, details = names, stand = "a project")
            }
            Triple(phrase, "secrets", "secrets")
        }
        view.secretsStore != null -> Triple(Phrase("Save ${plural(view.secretsStore!!.total.toInt(), "secret")} in your ", "vault"), "vault", "write")
        view.tokenStore != null -> Triple(Phrase("Save a ", "${serviceName(view.tokenStore!!.service)} token"), "vault", "write")
        view.ssh != null -> Triple(Phrase("Sign in to a server", details = hostLine(dev.reins.android.ui.approval.sshTarget(view.ssh!!.host, view.ssh!!.hostKey))), "ssh", "secrets")
        purchase != null -> {
            // The store is its domain as Reins read it from the checkout (the merchant's name is the page's own words),
            // in ASCII: a host with other letters is shown in punycode, its raw form under it.
            val host = asciiHost(untrusted(purchase.domain))
            val phrase = if (host == null) {
                Phrase("Pay ${serifLine(purchase.total)} at a store", details = "Store: ${rawForm(untrusted(purchase.domain))}")
            } else {
                Phrase("Pay ${serifLine(purchase.total)} at ", host.ascii, details = host.flag, stand = "a store")
            }
            Triple(phrase, "payments", "pay")
        }
        mcp != null -> {
            val action = if (mcp.destructive) "delete" else if (mcp.readOnly) "read" else "write"
            Triple(mcpPhrase(untrusted(mcp.tool), untrusted(mcp.serverName), host = urlHost(mcp.serverUrl)), view.service.ifBlank { "mcp:" }, action)
        }
        view.kind == ApprovalKind.GRANT -> Triple(Phrase(if (view.grant?.action == "send") "Send emails without asking" else "Read emails without asking"), service, "grant")
        view.kind == ApprovalKind.ACCOUNTS -> Triple(Phrase("See your ${serviceName(service)} addresses"), service, "accounts")
        view.kind == ApprovalKind.SEND -> {
            val to = view.email?.let { it.to + it.cc }.orEmpty().map(::untrusted)
            Triple(sendPhrase(to.take(1), (to.size - 1).coerceAtLeast(0)), service, "send")
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
                    Phrase("Delete the $noun ", target, stand = "one")
                }
                else -> Phrase("$title in ", target, stand = "a place")
            }
            Triple(phrase, view.service, actionOfView(view, deletes))
        }
    }
    return phrase.told(tense, who, viewKind(view), provider, action)
}

// ---- the parts ------------------------------------------------------------------------------------------------------

/**
 * A command: "Run a command in *reins*" (the folder only as the core kept it), the guard's own match in mono with "and
 * more" when the command holds more, else the whole command; the full command is a fact row. The phone never splits a
 * command: only the core and the guard read it. [uncertain]: the generic words and the whole command.
 */
private fun commandPhrase(command: String, cwd: String?, matched: String?, uncertain: Boolean): Phrase {
    val whole = command.trim().ifEmpty { null }
    if (uncertain) return Phrase("Run a command", details = whole, full = whole)
    val m = matched?.let(::untrusted)?.trim()?.takeIf { it.isNotEmpty() && whole != null && whole.contains(it) }
    val line = when {
        m == null -> whole
        m == whole -> m
        else -> "$m and more"
    }
    val folder = cwd?.let(::folderName)
    return Phrase(if (folder != null) "Run a command in " else "Run a command", folder, details = line, full = whole, stand = "")
}

/** "Read *.env* and 1 more"; the path in mono. A path that climbs out ("a/../b") names no file: "Read a file". */
private fun filePhrase(verb: String, path: String, tail: String): Phrase {
    val p = path.trim()
    val name = fileName(p)
    val after = if (tail.isEmpty()) "" else " $tail"
    if (name == null) return Phrase("$verb a file$after", details = p.ifEmpty { null }?.let(::oneLine))
    return Phrase("$verb ", name, after, details = oneLine(p), stand = "a file")
}

/**
 * A tool of an MCP server: "Use a tool on *Linear*", the tool's name (and the server's host) in mono. The verb is never
 * the tool's own title: that is the server's words, not what Reins knows the call does.
 */
private fun mcpPhrase(tool: String, server: String, host: String?): Phrase {
    val on = server.trim().ifBlank { null }
    val details = listOfNotNull(serifLine(tool).ifEmpty { null }, host?.let { hostLine(it) }).joinToString(" · ").ifEmpty { null }
    return Phrase(if (on != null) "Use a tool on " else "Use a tool", on, details = details, stand = "a server")
}

private val GIT_DELETE = Regex("^Delete (branch|tag) .+ from$|^Delete \\d+ refs from$")

/**
 * A push from its structured view: "Push 3 commits to *octo/app*", "Delete a branch from", "Push 2 tags to"; the
 * count and the branches come from its refs, the branch line ("feature/x · new branch") or the names in mono.
 */
private fun pushPhrase(git: GitPushView, repo: String): Phrase {
    val refs = git.refs
    val deletes = refs.isNotEmpty() && refs.all { it.change == "delete" }
    val lead = when {
        deletes -> if (refs.size == 1) "Delete a ${if (refs[0].kind == "tag") "tag" else "branch"} from" else "Delete ${refs.size} refs from"
        refs.isNotEmpty() && refs.all { it.kind == "tag" } -> if (refs.size == 1) "Push a tag to" else "Push ${refs.size} tags to"
        else -> {
            val commits = refs.filter { it.change != "delete" }.sumOf { it.commitCount.toInt() }
            val verb = if (refs.any { it.force }) "Force-push" else "Push"
            if (commits == 0) "$verb to" else "$verb ${plural(commits, "commit")} to"
        }
    }
    val names = refs.joinToString(", ") { serifLine(it.shortName) }.ifEmpty { null }
    val details = if (deletes || refs.all { it.kind == "tag" }) names else pushLine(refLines(git))
    return repoPhrase(lead, repo, details)
}

/** A push or a deletion from the core's label: the repository is the target, branches and tags go to the mono line. */
private fun gitPhrase(lead: String, repo: String, refs: List<RefLine>): Phrase {
    val names = refs.joinToString(", ") { serifLine(it.name) }.ifEmpty { null }
    val one = Regex("^Delete (branch|tag) (.+) from$").find(lead)
    val tag = Regex("^Push tag (.+) to$").find(lead)
    val (head, details) = when {
        one != null -> "Delete a ${one.groupValues[1]} from" to one.groupValues[2]
        tag != null -> "Push a tag to" to tag.groupValues[1]
        lead.startsWith("Delete ") || lead.endsWith(" tags to") -> lead to names
        else -> lead to pushLine(refs)
    }
    return repoPhrase(head, repo, details)
}

/**
 * "Push 3 commits to *octo/app*": the repository as the core accepts one (`owner/name` of ASCII letters, digits, `-`,
 * `_` and `.`, no `..`); anything else is "a repository" with its raw form in mono.
 */
private fun repoPhrase(head: String, repo: String, details: String?): Phrase {
    val r = repo.trim()
    if (r.isEmpty()) return Phrase(dropPreposition(head), details = details)
    if (!repoOk(r)) return Phrase("$head a repository", details = listOfNotNull("Repository: ${rawForm(r)}", details).joinToString("\n"))
    return Phrase("$head ", r, details = details, stand = "a repository")
}

/** Where a push goes, in mono: "feature/x · new branch", "main", "feature/x and 1 more". */
fun pushDetails(git: GitPushView?): String? = pushLine(refLines(git))

/** One ref of a push's structured view, for the mono line. */
private data class RefLine(val name: String, val change: String, val kind: String)

private fun refLines(git: GitPushView?): List<RefLine> = git?.refs.orEmpty().map { RefLine(it.shortName, it.change, it.kind) }

private fun pushLine(all: List<RefLine>): String? {
    val refs = all.filter { it.change != "delete" }
    val first = refs.firstOrNull() ?: return null
    val name = buildString {
        append(serifLine(first.name))
        if (first.change == "create") append(if (first.kind == "tag") " · new tag" else " · new branch")
    }
    return if (refs.size > 1) "$name and ${refs.size - 1} more" else name
}

/**
 * An email: "Send an email", or "to 3 people", and the recipients as written in mono (an address's host in punycode).
 * The name in "Dana <dana@…>" is the AI's own words, so it is never the target.
 */
private fun sendPhrase(to: List<String>, more: Int): Phrase {
    val first = to.firstOrNull()?.let(::addressLine)?.ifEmpty { null } ?: return Phrase("Send an email")
    val n = more + 1
    return Phrase(if (n == 1) "Send an email" else "Send an email to $n people", details = first + if (more > 0) " and $more more" else "")
}

/** "Read 2 emails from *Stripe*" (the sender's name, as the mail says it), the address under it; a bare address is mono. */
private fun personPhrase(lead: String, from: String): Phrase {
    val name = dev.reins.android.ui.approval.displayName(from).trim()
    val address = if ('<' in from) from.substringAfter('<').substringBefore('>').trim() else from.trim()
    val line = addressLine(address).ifEmpty { null }
    return when {
        name.isBlank() || looksMachine(name) -> Phrase(dropPreposition(lead), details = line)
        else -> Phrase("$lead ", name, details = line?.takeIf { it != serifLine(name) }, stand = "a sender", kind = ValueKind.Name)
    }
}

/** A label's subject: a human one is the target; a command, URL, path or address moves to the mono line. */
private fun subjectPhrase(lead: String, subject: String, code: Boolean, tail: String, kind: ValueKind): Phrase {
    if ('<' in subject && '>' in subject && !code) return personPhrase(lead, subject).copy(after = spaced(tail))
    if (code || looksMachine(subject)) {
        return Phrase(joinTail(dropPreposition(lead), tail), details = oneLine(subject), full = if (code && '\n' in subject) subject.trim() else null)
    }
    return Phrase(withSpace(lead), subject, spaced(tail), stand = "", kind = kind)
}

/** Integrations whose subjects are people, chats, events or words (a chat's name, a contact): [ValueKind.Name]. */
private val NAME_PROVIDERS = setOf("telegram", "sms", "gmail", "gcontacts", "device_contacts", "gcalendar", "device_calendar")

/** An entry from before labels: built from what the operation is called and what the core kept of it. */
private fun legacy(entry: ActivityEntry, tense: Tense): AskSentence {
    val svc = entry.service.ifBlank { "gmail" }
    val kind = ActionKind.of(entry.action)
    val count = entry.count.toInt()
    val mail = entry.op.isEmpty() && svc == "gmail"
    val info = entry.info
    val phrase = when {
        kind == ActionKind.Grant -> {
            // Told as it ended, in any tense: "Got access: …", "Asked for access: …".
            val head = if (entry.outcome == "granted") "Got access" else "Asked for access"
            val summary = info.grantSummary?.let(::untrusted)?.takeIf { it.isNotBlank() }
            if (summary == null) Phrase(head, question = true) else Phrase("$head: ", summary, question = true, stand = "", kind = ValueKind.Name)
        }
        svc.startsWith(dev.reins.android.design.MCP_PREFIX) -> mcpPhrase(untrusted(entry.op), serviceName(svc), host = null)
        mail && kind == ActionKind.Search -> info.query?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { forPhrase("Search Gmail for", it) } ?: Phrase("Search Gmail")
        mail && kind == ActionKind.Read -> {
            val from = info.messages.map { untrusted(it.from) }.distinctBy { dev.reins.android.ui.approval.displayName(it) }.singleOrNull()
            val lead = "Read ${plural(count.coerceAtLeast(1), "email")}"
            if (from != null) personPhrase("$lead from", from) else Phrase(lead)
        }
        mail && kind == ActionKind.Send -> {
            val to = info.email?.let { it.to + it.cc }.orEmpty().map(::untrusted)
            sendPhrase(to.take(1), (to.size - 1).coerceAtLeast(0))
        }
        else -> Phrase(operationTitle(entry.action, count, entry.service, untrusted(entry.opTitle), entry.op))
    }
    val a = legacyAction(svc, entry.action, entry.op).second
    return phrase.told(tense, null, "${serviceName(svc)} · ${kindWord(a, svc)}", svc, a)
}

/**
 * Puts the tense on: present from the verb; past, with [who] first when given; refused as what was asked. The target
 * is checked here, once, for every surface: one that can pass for something else leaves the serif for its stand-in
 * words, and the mono line says why with the raw form.
 */
private fun Phrase.told(tense: Tense, who: String?, kind: String, provider: String, action: String): AskSentence {
    val raw = target?.let { untrusted(it).trim() }?.takeIf { it.isNotEmpty() }
    val shown = raw?.let(::serifLine)
    val flag = raw?.let { lookalikeFlag(it, this.kind) }
    val target = if (flag == null) shown else null
    val body = when {
        target != null -> head + target + after
        raw != null && !stand.isNullOrEmpty() -> head + stand + after
        // No stand-in: the words before the target, without the word that led to it ("in", "to", ":").
        raw != null -> dropPreposition(head.trimEnd().removeSuffix(":")) + after
        else -> head + after
    }.let(::serifLine)
    val questionFlag = if (question && target == null && raw == null) lookalikeFlag(body, this.kind) else null
    // The asker's display name is a name: left out only when it mixes scripts.
    val name = who?.let { untrustedName(serifLine(it)) }?.takeIf { it.isNotEmpty() && lookalikeFlag(it, ValueKind.Name) == null }
    val text = when {
        questionFlag != null -> if (tense == Tense.Waiting) "Answer a question from your computer" else "A question from your computer"
        question -> body
        tense == Tense.Waiting -> body.replaceFirstChar { it.uppercase() }
        tense == Tense.Done -> pastTense(body).let { if (name != null) "$name ${lowerVerb(it)}" else it.replaceFirstChar { c -> c.uppercase() } }
        else -> (if (name != null) "$name asked to " else "Asked to ") + lowerVerb(body)
    }
    val lines = listOfNotNull(flag, questionFlag, details?.let(::cleanLines)?.ifEmpty { null })
    return AskSentence(
        cleanKind(kind),
        serifLine(text),
        target?.takeIf { it in text },
        lines.joinToString("\n").ifEmpty { null },
        full?.let { untrusted(it).trim() }?.ifEmpty { null },
        provider,
        action,
    )
}

// ---- words ----------------------------------------------------------------------------------------------------------

private val PAST_VERBS = mapOf(
    "run" to "ran", "read" to "read", "send" to "sent", "see" to "saw", "get" to "got", "write" to "wrote", "buy" to "bought",
    "pay" to "paid", "sign" to "signed", "make" to "made", "find" to "found", "put" to "put", "set" to "set", "take" to "took",
    "give" to "gave", "begin" to "began", "label" to "labelled", "cancel" to "cancelled", "stop" to "stopped", "ask" to "asked",
)

/**
 * "Push 3 commits to" → "Pushed 3 commits to", "Force-push" → "Force-pushed", "Run" → "Ran"; two verbs joined by "and"
 * or "or" both turn ("Read and archive" → "Read and archived"), never only the first.
 */
fun pastTense(phrase: String): String {
    val first = phrase.substringBefore(' ')
    val past = pastWord(first) ?: return phrase
    val rest = phrase.substring(first.length)
    val joined = Regex("^ (and|or) (\\S+)").find(rest)
    val second = joined?.groupValues?.get(2)?.let(::pastWord)
    return if (joined != null && second != null) {
        past + " ${joined.groupValues[1]} " + second + rest.substring(joined.range.last + 1)
    } else {
        past + rest
    }
}

/** One word in the past tense ("push" → "pushed", "Force-push" → "Force-pushed"); null when it is not a word. */
private fun pastWord(word: String): String? {
    val verb = word.substringAfterLast('-')
    if (verb.isEmpty() || !verb.all { it.isLetter() }) return null
    val lower = verb.lowercase()
    val past = PAST_VERBS[lower] ?: when {
        lower.endsWith("e") -> lower + "d"
        lower.length > 1 && lower.endsWith("y") && lower[lower.length - 2] !in "aeiou" -> lower.dropLast(1) + "ied"
        else -> lower + "ed"
    }
    val cased = if (verb[0].isUpperCase()) past.replaceFirstChar { it.uppercase() } else past
    return word.dropLast(verb.length) + cased
}

/** The verb in lower case, a name that starts the phrase ("GitHub …") kept. */
private fun lowerVerb(s: String): String = if (s.length > 1 && s[1].isUpperCase()) s else s.replaceFirstChar { it.lowercase() }

/** The kind line from the core's provider and action: what it touches, then what it does ("GitHub · Push"). */
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
    else -> "Change"
}

/** The kind line, checked like a target: a server's name that can pass for another becomes "MCP server". */
private fun cleanKind(kind: String): String {
    val where = kind.substringBefore(" · ")
    val what = kind.substringAfter(" · ", "")
    val safe = serifLine(where).takeIf { it.isNotEmpty() && lookalikeFlag(it, ValueKind.Identifier) == null } ?: "Service"
    return if (what.isEmpty()) safe else "$safe · $what"
}

/** The approval view's kind line from its kinds and fields ("GitHub · Push", "Payments · Buy", "Vault · Secrets"). */
private fun viewKind(view: ApprovalView): String {
    val mcp = view.mcp
    val where = when {
        mcp != null -> untrustedName(mcp.serverName).let { if (it.isEmpty() || lookalikeFlag(it, ValueKind.Identifier) != null) "MCP server" else it }
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

/** The Signal field of a request's action (the core's): its badge's colour. */
fun actionField(action: String): Field = when (action) {
    "read", "search", "ask" -> Field.Read
    "delete", "pay" -> Field.Delete
    "grant" -> Field.Grant
    "secrets", "accounts" -> Field.Accounts
    "pair", "join" -> Field.Pair
    else -> Field.Push
}

// ---- what may pass for something else -------------------------------------------------------------------------------

/** A line for the serif or the kind line: hidden characters out, one line, NFC, single spaces. */
fun serifLine(s: String): String = Normalizer.normalize(oneLine(untrusted(s)), Normalizer.Form.NFC).replace(Regex(" {2,}"), " ").trim()

/** The mono line: each line cleaned the same way, line breaks kept between them. */
private fun cleanLines(s: String): String = s.lines().map(::serifLine).filter { it.isNotEmpty() }.joinToString("\n")

/**
 * Why [s] cannot be set in the serif, with its raw form ("Look-alike letters: p[U+0430]ypal"), or null when it can.
 * Any [kind]: letters of scripts that do not go together (Latin with a Cyrillic letter; Latin, Han, kana and Hangul go
 * together as Unicode TR39 allows). An [ValueKind.Identifier] also: a letter known to pass for a Latin one (Cyrillic
 * а, Greek ο, … even when the whole name is in that script) or a compatibility form (fullwidth, mathematical).
 * (Accents NFC composes are fine: the sentence is set in NFC.)
 */
fun lookalikeFlag(s: String, kind: ValueKind = ValueKind.Identifier): String? {
    val t = oneLine(untrusted(s)).trim()
    if (t.isEmpty()) return null
    val nfc = Normalizer.normalize(t, Normalizer.Form.NFC)
    val flagged = when (kind) {
        ValueKind.Name -> mixedScripts(nfc)
        ValueKind.Identifier -> mixedScripts(nfc) || nfc.any { it in CONFUSABLE } || Normalizer.normalize(nfc, Normalizer.Form.NFKC) != nfc
    }
    return if (flagged) "Look-alike letters: ${rawForm(t)}" else null
}

/** The CJK scripts, which TR39 lets a name mix with each other and with Latin (Japanese, Chinese, Korean names). */
private val CJK = setOf(
    Character.UnicodeScript.HAN, Character.UnicodeScript.HIRAGANA, Character.UnicodeScript.KATAKANA,
    Character.UnicodeScript.HANGUL, Character.UnicodeScript.BOPOMOFO,
)

/**
 * Letters of scripts that do not go together (Common and Inherited marks aside). Latin with the CJK scripts, and the
 * CJK scripts with each other, go together (TR39's highly restrictive level); anything else mixed does not.
 */
fun mixedScripts(s: String): Boolean {
    val scripts = mutableSetOf<Character.UnicodeScript>()
    var i = 0
    while (i < s.length) {
        val cp = s.codePointAt(i)
        i += Character.charCount(cp)
        if (!Character.isLetter(cp)) continue
        val script = Character.UnicodeScript.of(cp)
        if (script != Character.UnicodeScript.COMMON && script != Character.UnicodeScript.INHERITED) scripts += script
    }
    val other = scripts - CJK - Character.UnicodeScript.LATIN
    return when {
        other.isEmpty() -> false // Latin, CJK, or Latin with CJK
        else -> scripts.size > 1 // one other script alone is fine; with anything else it is mixed
    }
}

/** Letters of other scripts that pass for Latin ones (the common Cyrillic, Greek and Armenian look-alikes). */
private val CONFUSABLE: Set<Char> = (
    "аеорсуххіјѕԁһӏԛԝүАВЕКМНОРСТУХІЈЅԌ" + // Cyrillic
        "αβεικνορτυχΑΒΕΖΗΙΚΜΝΟΡΤΥΧ" + // Greek
        "օսոհցաԱՍՕ" // Armenian
    ).toSet()

/** The raw form for the mono line: ASCII as it is, anything else as [U+XXXX]. */
fun rawForm(s: String): String = buildString {
    var i = 0
    while (i < s.length) {
        val cp = s.codePointAt(i)
        i += Character.charCount(cp)
        if (cp in 0x20..0x7E) appendCodePoint(cp) else append("[U+%04X]".format(cp))
    }
}

/** A host in ASCII: [ascii] (punycode for one with other letters) and a [flag] with its raw form when it was converted. */
data class AsciiHost(val ascii: String, val flag: String?)

private val HOST_ASCII = Regex("^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*\\.?(:\\d{1,5})?$")

/** [host] as ASCII (IDNA to punycode); null when it is not a host at all. */
fun asciiHost(host: String): AsciiHost? {
    val h = oneLine(host).trim().trimEnd('.')
    if (h.isEmpty()) return null
    val port = Regex(":(\\d{1,5})$").find(h)?.value.orEmpty()
    val name = h.removeSuffix(port)
    val ascii = try {
        java.net.IDN.toASCII(Normalizer.normalize(name, Normalizer.Form.NFC), java.net.IDN.USE_STD3_ASCII_RULES or java.net.IDN.ALLOW_UNASSIGNED).lowercase()
    } catch (_: Exception) {
        return null
    }
    if (!HOST_ASCII.matches(ascii + port)) return null
    val changed = ascii != name.lowercase()
    return AsciiHost(ascii + port, if (changed) "Not ASCII, shown as punycode: ${rawForm(name)}" else null)
}

/** A host for the mono line: ASCII or punycode, with its raw form when it was converted or is not a host. */
private fun hostLine(host: String): String {
    val h = untrusted(host).trim()
    val a = asciiHost(h) ?: return rawForm(oneLine(h))
    return listOfNotNull(a.ascii, a.flag?.let { "($it)" }).joinToString(" ")
}

/** The host of a URL, or null. */
private fun urlHost(url: String): String? = runCatching { java.net.URI(untrusted(url).trim()).host }.getOrNull()?.takeIf { it.isNotBlank() }

/**
 * An address for the mono line ("Dana <dana@acme.dev>"): cleaned, its host in punycode when it has other letters (the
 * raw form after it), the name as written.
 */
fun addressLine(address: String): String {
    val a = serifLine(address)
    val at = a.lastIndexOf('@')
    if (at < 0) return a
    val end = a.indexOf('>', at).takeIf { it > 0 } ?: a.length
    val host = a.substring(at + 1, end)
    val ascii = asciiHost(host) ?: return rawForm(a)
    val shown = a.substring(0, at + 1) + ascii.ascii + a.substring(end)
    return if (ascii.flag != null) "$shown (${ascii.flag})" else shown
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
    if (looksMachine(value)) Phrase(dropPreposition(lead), details = oneLine(value)) else Phrase("$lead ", value, stand = "something", kind = ValueKind.Name)

/** A repository as the core accepts one (`connector/git.rs`, `repo_ok` with nesting): checked the same way here. */
fun repoOk(repo: String): Boolean {
    val parts = repo.split('/')
    return parts.size in 2..20 && repo.length <= 250 && !repo.contains("..") &&
        parts.all { p -> p.isNotEmpty() && p.length <= 100 && p != "." && p != ".." && p.all { it.isAsciiLetterOrDigit() || it == '-' || it == '_' || it == '.' } }
}

private fun Char.isAsciiLetterOrDigit() = this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9'

/**
 * The folder a command ran in, by its name ("/home/dana/dev/reins" → "reins"); null for none, the home folder, or a
 * path that climbs ("…/reins/../other"), which names no folder with certainty.
 */
fun folderName(cwd: String): String? = lastName(cwd)?.takeIf { it != "~" }

/** A file's name from its path; null for a path that climbs or ends without one. */
private fun fileName(path: String): String? = lastName(path)

private fun lastName(path: String): String? {
    val t = oneLine(untrusted(path)).trim()
    val segments = t.split('/', '\\')
    if (segments.any { it == ".." || it == "." }) return null
    return segments.lastOrNull { it.isNotEmpty() }
}

private fun oneLine(s: String): String = s.replace('\n', ' ').replace('\t', ' ').replace('\r', ' ')

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
