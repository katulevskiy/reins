package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.design.Field
import dev.reins.android.ui.common.AskSentence
import dev.reins.android.ui.common.Tense
import dev.reins.android.ui.common.actionField
import dev.reins.android.ui.common.actionGlyph
import dev.reins.android.ui.common.askSentence
import dev.reins.android.ui.common.folderName
import dev.reins.android.ui.common.pastTense
import dev.reins.android.ui.common.providerAsset
import dev.reins.android.ui.common.providerGlyph
import dev.reins.android.design.Glyph
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** How every kind of request is written (design/signal/ASK-TEXT.md): one test per kind, and the rules across them. */
class AskSentenceTest {
    private val command = "rm -rf jarx && npm run build && npm test"

    /** The rules every sentence keeps: one italic run inside the text, no machine value in the serif. */
    private fun AskSentence.keepsTheRules(vararg machine: String) {
        target?.let { assertTrue("the target \"$it\" is part of \"$text\"", it in text) }
        target?.let { assertFalse("the target is not the verb", text.startsWith(it)) }
        machine.forEach { assertFalse("\"$it\" stays out of the serif: \"$text\"", it in text) }
    }

    // ---- commands -----------------------------------------------------------------------------------------------------

    @Test
    fun aWaitingCommandRunsInItsFolderWithTheGuardsMatchInMono() {
        val s = askSentence(TestData.label("terminal", "delete", "Run", command, code = true), cwd = "/home/dana/dev/reins", matched = "rm -rf jarx")
        assertEquals("Run a command in reins", s.text)
        assertEquals("reins", s.target)
        assertEquals("the guard's own match, never a split made here", "rm -rf jarx and more", s.details)
        assertEquals(command, s.full)
        assertEquals("Terminal · Delete", s.kind)
        s.keepsTheRules(command, "rm -rf", "/home/dana")
    }

    @Test
    fun aCommandOnTheFieldIsTheCoresReading() {
        val view = TestData.askView(
            detail = "Command:\n$command\n\nIn: /home/dana/dev/reins\nRule: rm -rf*",
            rule = dev.reins.core.AskRuleView("Deletes files", "rm -rf*", "rm -rf jarx", null, true, false),
        ).let { it.copy(ask = it.ask!!.copy(question = "Claude Code wants to run: $command", destructive = true)) }
        val s = askSentence(view, label = TestData.label("terminal", "delete", "Run", command, code = true))
        assertEquals("Run a command", s.text)
        assertEquals("rm -rf jarx and more", s.details)
        assertEquals(command, s.full)
        assertEquals("terminal" to "delete", s.provider to s.action)
        assertEquals("Terminal · Delete", s.kind)
    }

    @Test
    fun aLongCommandIsNeverCut() {
        val long = "echo " + "x".repeat(10_000) + " END"
        val s = askSentence(TestData.label("terminal", "run", "Run", long, code = true))
        assertTrue(s.full!!.endsWith(" END"))
        assertEquals(long, s.full)
        assertTrue(s.details!!.endsWith(" END"))
    }

    @Test
    fun aCommandIsNeverSplitHere() {
        val s = askSentence(TestData.label("terminal", "run", "Run", "make a && make b; make c", code = true))
        assertEquals("make a && make b; make c", s.details)
        assertEquals("make a", askSentence(TestData.label("terminal", "run", "Run", "make a", code = true), matched = "make a").details)
    }

    @Test
    fun aFolderIsItsLastName() {
        assertEquals("reins", folderName("/home/dana/dev/reins/"))
        assertEquals("api", folderName("~/dev/api"))
        assertNull(folderName("~"))
        assertNull(folderName(""))
        assertNull("a path that climbs names no folder", folderName("/home/dana/reins/../other"))
        assertNull(folderName("/home/dana/reins/."))
    }

    // ---- files --------------------------------------------------------------------------------------------------------

    @Test
    fun aFileReadNamesTheFileAndKeepsThePathInMono() {
        val s = askSentence(TestData.label("file", "read", "Read", "~/dev/api/.env", code = true, tail = " and 1 more"))
        assertEquals("Read .env and 1 more", s.text)
        assertEquals(".env", s.target)
        assertEquals("~/dev/api/.env", s.details)
        assertEquals("Files · Read", s.kind)
        s.keepsTheRules("~/dev")
        val change = askSentence(TestData.label("file", "write", "Change", "a.pem", code = true, tail = " and 1 more"))
        assertEquals("Change a.pem and 1 more", change.text)
        assertEquals("file" to "write", change.provider to change.action)
    }

    // ---- git ----------------------------------------------------------------------------------------------------------

    @Test
    fun aPushNamesTheRepositoryAndPutsTheBranchInMono() {
        val view = TestData.gitPushView(TestData.gitPush(TestData.gitRef("feature/x", change = "create", commitCount = 3u)))
        val s = askSentence(view)
        assertEquals("Push 3 commits to octo/app", s.text)
        assertEquals("octo/app", s.target)
        assertEquals("feature/x · new branch", s.details)
        assertEquals("GitHub · Push", s.kind)
        s.keepsTheRules("feature/x")
        val force = askSentence(TestData.gitPushView(TestData.gitPush(TestData.gitRef("main", force = true, commitCount = 2u))))
        assertEquals("Force-push 2 commits to octo/app", force.text)
        assertEquals("main", force.details)
    }

    @Test
    fun aTagOrABranchDeletionKeepsItsNameOutOfTheSerif() {
        val tag = askSentence(TestData.label("github", "push", "Push tag v1.2 to", "octo/app"))
        assertEquals("Push a tag to octo/app", tag.text)
        assertEquals("v1.2", tag.details)
        val delete = askSentence(TestData.label("github", "delete", "Delete branch old/experiment from", "octo/app"))
        assertEquals("Delete a branch from octo/app", delete.text)
        assertEquals("old/experiment", delete.details)
        assertEquals("octo/app", delete.target)
        delete.keepsTheRules("old/experiment")
    }

    // ---- mail ---------------------------------------------------------------------------------------------------------

    @Test
    fun aMailSearchIsForItsQuery() {
        val s = askSentence(TestData.label("gmail", "search", "Search Gmail for", "invoice"))
        assertEquals("Search Gmail for invoice", s.text)
        assertEquals("invoice", s.target)
        assertEquals("Gmail · Search", s.kind)
        val operators = askSentence(TestData.entry(4, "search", "released", info = TestData.info("from:bank newer_than:7d", emptyList(), null, null, null)))
        assertEquals("Searched Gmail", operators.text)
        assertEquals("from:bank newer_than:7d", operators.details)
    }

    @Test
    fun aMailReadIsFromItsSender() {
        val s = askSentence(TestData.label("gmail", "read", "Read 2 emails from", "Stripe"))
        assertEquals("Read 2 emails from Stripe", s.text)
        assertEquals("Stripe", s.target)
        val bare = askSentence(TestData.label("gmail", "read", "Read 2 emails from", "receipts@stripe.com"))
        assertEquals("Read 2 emails", bare.text)
        assertEquals("receipts@stripe.com", bare.details)
    }

    @Test
    fun anEmailSaysHowManyAndTheRecipientsAreMono() {
        val s = askSentence(TestData.label("gmail", "send", "Send an email to", "Dana Katul <dana@katul.dev>", tail = " and 1 more"))
        assertEquals("the name is the AI's words: never the target", "Send an email to 2 people", s.text)
        assertNull(s.target)
        assertEquals("Dana Katul <dana@katul.dev> and 1 more", s.details)
        val bare = askSentence(TestData.label("gmail", "send", "Send an email to", "billing@acme-pay.co"))
        assertEquals("Send an email", bare.text)
        assertEquals("billing@acme-pay.co", bare.details)
        bare.keepsTheRules("billing@")
    }

    // ---- secrets, SSH, payments, MCP, questions --------------------------------------------------------------------------

    @Test
    fun secretsAreForACommandShownWhole() {
        val s = askSentence(TestData.label("secrets", "secrets", "Use 2 secrets for", "npm run deploy", code = true))
        assertEquals("Use 2 secrets for a command", s.text)
        assertNull("no program read out of the command", s.target)
        assertEquals("npm run deploy", s.details)
        assertEquals("Vault · Secrets", s.kind)
        val view = askSentence(TestData.secretsView())
        assertEquals("Use 2 secrets for a command", view.text)
        assertEquals("npm run deploy", view.full)
    }

    @Test
    fun aProjectsEnvironmentIsTheTarget() {
        val s = askSentence(TestData.envReleaseView())
        assertEquals("Use 3 variables of shop · dev", s.text)
        assertEquals("shop · dev", s.target)
        assertEquals("DATABASE_URL, STRIPE_KEY, SESSION_SECRET", s.details)
        val print = askSentence(TestData.envReleaseView(TestData.envRelease(variables = listOf("API_KEY"), print = true)))
        assertEquals("Print a value from shop · dev", print.text)
        assertEquals("API_KEY", print.details)
    }

    @Test
    fun anSshSignInKeepsTheHostInMono() {
        val s = askSentence(TestData.label("ssh", "secrets", "Sign in to", "build.example.com", code = true))
        assertEquals("Sign in to a server", s.text)
        assertNull(s.target)
        assertEquals("build.example.com", s.details)
        assertEquals("SSH · Sign in", s.kind)
        assertEquals("build.example.com", askSentence(TestData.sshView()).details)
    }

    @Test
    fun aPurchaseIsAtTheStoresDomainWithTheAmountRoman() {
        val s = askSentence(TestData.purchaseView())
        assertEquals("Pay $34.97 at amazon.com", s.text)
        assertEquals("amazon.com", s.target)
        assertEquals("Payments · Buy", s.kind)
        assertEquals(Field.Delete, actionField(s.action))
    }

    @Test
    fun anMcpToolIsUsedOnItsServer() {
        val named = askSentence(TestData.label("mcp:linear", "write", "Use", "Create issue", tail = " on Linear"))
        assertEquals("the tool's title is the server's words, not the verb", "Use a tool on Linear", named.text)
        assertEquals("Linear", named.target)
        assertEquals("Create issue", named.details)
        val code = askSentence(TestData.label("mcp:linear", "write", "Use", "create_issue", code = true, tail = " on Linear"))
        assertEquals("Use a tool on Linear", code.text)
        assertEquals("create_issue", code.details)
    }

    @Test
    fun aQuestionStaysAsAsked() {
        val s = askSentence(TestData.label("desktop", "ask", "Deploy the site now?"))
        assertEquals("Deploy the site now?", s.text)
        assertNull(s.target)
        assertEquals("Deploy the site now?", askSentence(TestData.label("desktop", "ask", "Deploy the site now?"), Tense.Done, who = "Codex").text)
        assertEquals("Computer · Asks", s.kind)
    }

    @Test
    fun aGenericToolCallInAPlaceTargetsThePlace() {
        val s = askSentence(TestData.label("telegram", "read", "Read messages in", "Family"))
        assertEquals("Read messages in Family", s.text)
        assertEquals("Family", s.target)
        val url = askSentence(TestData.label("github", "read", "Read a file in", "https://github.com/octo/app"))
        assertEquals("Read a file", url.text)
        assertEquals("https://github.com/octo/app", url.details)
    }

    @Test
    fun aGroupNeverSetsAVerbInItalic() {
        val items = listOf(TestData.pending("r1", "search", 1u), TestData.pending("r2", "read", 2u))
        val ask = dev.reins.android.ui.activity.groupAsk(items) { null }
        assertEquals("Search Gmail and read 2 emails", ask.text)
        assertNull(ask.target)
    }

    @Test
    fun aPairingQuotesTheName() {
        val s = askSentence(TestData.pairingItem())
        assertEquals("Connect “Claude”", s.text)
        assertNull(s.target)
    }

    // ---- tense --------------------------------------------------------------------------------------------------------

    @Test
    fun decidedRequestsArePastAndStartWithWhoAsked() {
        val push = TestData.label("github", "push", "Push 3 commits to", "katulevskiy/reins")
        assertEquals("Pushed 3 commits to katulevskiy/reins", askSentence(push, Tense.Done).text)
        assertEquals("git pushed 3 commits to katulevskiy/reins", askSentence(push, Tense.Done, who = "git").text)
        assertEquals("Asked to push 3 commits to katulevskiy/reins", askSentence(push, Tense.Refused).text)
        assertEquals("Claude Code ran a command in reins", askSentence(TestData.label("terminal", "run", "Run", "ls", code = true), Tense.Done, "Claude Code", cwd = "/x/reins").text)
    }

    @Test
    fun anEntryIsToldByHowItEnded() {
        val ran = TestData.entry(1, "write", "released", label = TestData.label("terminal", "run", "Run", "ls", code = true), request = TestData.asked("ls && pwd", cwd = "/home/me/dev/reins"))
        val s = askSentence(ran)
        assertEquals("Ran a command in reins", s.text)
        assertEquals("ls && pwd", s.details)
        assertEquals("ls && pwd", s.full)
        val denied = askSentence(TestData.entry(2, "send", "denied", label = TestData.label("gmail", "send", "Send an email to", "Ann <ann@corp.example>")))
        assertEquals("Asked to send an email", denied.text)
        val legacy = askSentence(TestData.entry(3, "read", "released", 3u))
        assertEquals("Read 3 emails", legacy.text)
    }

    @Test
    fun pastTenseKnowsTheVerbs() {
        assertEquals("Ran a command", pastTense("Run a command"))
        assertEquals("Force-pushed 2 commits", pastTense("Force-push 2 commits"))
        assertEquals("Signed in to a server", pastTense("Sign in to a server"))
        assertEquals("Read 2 emails", pastTense("Read 2 emails"))
        assertEquals("Sent an email", pastTense("Send an email"))
        assertEquals("Used 2 secrets", pastTense("Use 2 secrets"))
        assertEquals("Searched Gmail", pastTense("Search Gmail"))
        assertEquals("Paid $3", pastTense("Pay $3"))
        assertEquals("Copied it", pastTense("Copy it"))
    }

    // ---- the icon -----------------------------------------------------------------------------------------------------

    @Test
    fun theIconIsTheIntegrationWithTheActionOnItsCorner() {
        assertEquals("github", providerAsset("github"))
        assertEquals("mcp", providerAsset("mcp:linear"))
        assertNull(providerAsset("terminal"))
        assertEquals(Glyph.Terminal, providerGlyph("terminal", "run"))
        assertEquals(Glyph.FileText, providerGlyph("file", "read"))
        assertEquals(Glyph.Lock, providerGlyph("secrets", "secrets"))
        assertEquals(Glyph.Key, providerGlyph("ssh", "secrets"))
        assertEquals(Glyph.Help, providerGlyph("desktop", "ask"))
        assertEquals(Glyph.GitBranch, actionGlyph("push"))
        assertEquals(Glyph.Eye, actionGlyph("read"))
        assertEquals(Glyph.Trash, actionGlyph("delete"))
        assertEquals(Glyph.Play, actionGlyph("run"))
        assertEquals(Glyph.Pencil, actionGlyph("write"))
        assertEquals(Field.Push, actionField("push"))
        assertEquals(Field.Read, actionField("read"))
        assertEquals(Field.Delete, actionField("delete"))
        assertEquals(Field.Grant, actionField("grant"))
    }

    // ---- crafted inputs: the sentence never claims more than what was asked or ran ------------------------------------

    @Test
    fun aRecipientsDisplayNameNeverBecomesTheTarget() {
        val view = TestData.sendView().let { it.copy(email = it.email!!.copy(to = listOf("Your bank <attacker@evil.example>"), cc = emptyList())) }
        val s = askSentence(view)
        assertEquals("Send an email", s.text)
        assertNull(s.target)
        assertFalse("Your bank" in s.text)
        assertEquals("Your bank <attacker@evil.example>", s.details)
    }

    @Test
    fun aToolsTitleNeverBecomesTheVerb() {
        val view = TestData.mcpCallView(destructive = true).let { it.copy(mcp = it.mcp!!.copy(title = "Read your calendar", tool = "delete_everything")) }
        val s = askSentence(view)
        assertEquals("delete", s.action)
        assertFalse("Read" in s.text)
        assertTrue(s.text.startsWith("Use a tool"))
        assertEquals("delete_everything · mcp.linear.app", s.details)
    }

    @Test
    fun theCommandIsTheCoresNotTheQuestionsOrTheDetails() {
        // The question and the detail say one thing; the core read another: the core's reading is what is shown.
        val view = TestData.askView(detail = "Command:\nls -la\n\nIn: /srv").let { it.copy(ask = it.ask!!.copy(question = "Claude wants to run: ls -la")) }
        val s = askSentence(view, label = TestData.label("terminal", "delete", "Run", "rm -rf /srv/data", code = true))
        assertEquals("rm -rf /srv/data", s.full)
        assertFalse("ls -la" in (s.details.orEmpty() + s.text))
        assertEquals("delete", s.action)
    }

    @Test
    fun withoutTheCoresReadingAQuestionStaysAsAsked() {
        val view = TestData.askView(detail = "Command:\nrm -rf /", topic = "command:ls").let {
            it.copy(ask = it.ask!!.copy(question = "Claude wants to run: ls"))
        }
        val s = askSentence(view)
        assertEquals("desktop" to "ask", s.provider to s.action)
        assertEquals("Claude wants to run: ls", s.text)
        assertNull("nothing read out of the detail here", s.full)
        assertEquals("Computer · Asks", s.kind)
    }

    @Test
    fun anUncertainReadingIsGenericWithTheWholeCommandUnderIt() {
        val view = TestData.askView(
            detail = "Command:\nrm -rf jarx && npm test\n\nIn: /home/dana/dev/reins\nRule: rm -rf*",
            rule = dev.reins.core.AskRuleView("Deletes files", "rm -rf*", "rm -rf jarx", null, true, false),
        ).let { it.copy(ask = it.ask!!.copy(question = "Claude wants to run: rm -rf jarx && npm test", uncertain = true)) }
        val s = askSentence(view, label = TestData.label("terminal", "delete", "Run", "rm -rf jarx && npm test", code = true), cwd = "/home/dana/dev/reins")
        assertEquals("Run a command", s.text)
        assertNull("no folder claimed", s.target)
        assertEquals("rm -rf jarx && npm test", s.details)
        assertEquals("rm -rf jarx && npm test", s.full)
    }

    @Test
    fun aMatchTheCommandDoesNotHoldIsIgnored() {
        val s = askSentence(TestData.label("terminal", "run", "Run", "make build && rm -rf /", code = true), matched = "ls")
        assertEquals("make build && rm -rf /", s.details)
    }

    @Test
    fun onlyACarriedOutEntryIsToldDone() {
        val run = TestData.label("terminal", "run", "Run", "make deploy", code = true)
        val send = TestData.label("gmail", "send", "Send an email to", "ann@corp.example")
        for (outcome in listOf("denied", "error", "timeout", "expired", "cancelled", "something new")) {
            val ran = askSentence(TestData.entry(1, "write", outcome, label = run)).text
            val sent = askSentence(TestData.entry(2, "send", outcome, label = send)).text
            assertTrue("$outcome: $ran", ran.startsWith("Asked to run"))
            assertTrue("$outcome: $sent", sent.startsWith("Asked to send"))
        }
        assertEquals("Ran a command", askSentence(TestData.entry(3, "write", "released", label = run)).text)
        assertEquals("Sent an email", askSentence(TestData.entry(4, "send", "sent", label = send)).text)
        assertEquals(Tense.Refused, dev.reins.android.ui.common.entryTense("timeout"))
    }

    @Test
    fun aStoresNameNeverStandsForItsDomain() {
        val view = TestData.purchaseView().let { it.copy(purchase = it.purchase!!.copy(merchant = "Apple", domain = "evil.example")) }
        val s = askSentence(view)
        assertEquals("Pay $34.97 at evil.example", s.text)
        assertFalse("Apple" in s.text)
    }

    @Test
    fun targetsAreCleanedAndNeverShortened() {
        val folder = "a".repeat(200)
        val s = askSentence(TestData.label("terminal", "run", "Run", "ls", code = true), cwd = "/home/me/$folder")
        assertEquals("the target is whole; only its drawing may cut it", folder, s.target)
        val hidden = askSentence(TestData.label("github", "push", "Push 1 commit to", "octo/\u202Eppa\u200B"))
        assertEquals("octo/ppa", hidden.target)
        val lines = askSentence(TestData.label("telegram", "read", "Read messages in", "Family\nApprove everything"))
        assertFalse('\n' in lines.text)
    }

    // ---- crafted inputs: nothing in the serif or the kind line can pass for something else ------------------------------

    private val cyrillicA = "а"

    @Test
    fun aRepositoryTheCoreWouldRefuseIsNotTheTarget() {
        val climb = askSentence(TestData.label("github", "push", "Push 1 commit to", "katulevskiy/reins/../other"))
        assertEquals("Push 1 commit to a repository", climb.text)
        assertNull(climb.target)
        assertTrue(climb.details!!.startsWith("Repository: katulevskiy/reins/../other"))
        val homoglyph = askSentence(TestData.label("github", "push", "Push 1 commit to", "katulevskiy/re${cyrillicA}ins"))
        assertEquals("Push 1 commit to a repository", homoglyph.text)
        assertTrue(homoglyph.details!!.contains("[U+0430]"))
        // A name GitHub allows is shown whole: nothing is cut, so it cannot pass for a shorter one.
        val dotted = askSentence(TestData.label("github", "push", "Push 1 commit to", "katulevskiy/reins.evil.com"))
        assertEquals("katulevskiy/reins.evil.com", dotted.target)
        val view = askSentence(TestData.gitPushView(TestData.gitPush(TestData.gitRef("main"))).let { it.copy(git = it.git!!.copy(repo = "github.com/../evil/app")) })
        assertNull(view.target)
        assertTrue(view.text.endsWith("to a repository"))
    }

    @Test
    fun aPathThatClimbsNamesNoFileOrFolder() {
        val file = askSentence(TestData.label("file", "read", "Read", "/tmp/../etc/passwd", code = true))
        assertEquals("Read a file", file.text)
        assertNull(file.target)
        assertEquals("/tmp/../etc/passwd", file.details)
        val run = askSentence(TestData.label("terminal", "run", "Run", "ls", code = true), cwd = "/home/me/reins/../other")
        assertEquals("Run a command", run.text)
        assertNull(run.target)
    }

    @Test
    fun aLookAlikeFileOrFolderNameLeavesTheSerifWithItsRawForm() {
        val file = askSentence(TestData.label("file", "read", "Read", "/home/me/p${cyrillicA}ypal.pdf", code = true))
        assertEquals("Read a file", file.text)
        assertNull(file.target)
        assertEquals("Look-alike letters: p[U+0430]ypal.pdf", file.details!!.lines().first())
        val folder = askSentence(TestData.label("terminal", "run", "Run", "ls", code = true), cwd = "/home/me/r${cyrillicA}ins")
        assertEquals("Run a command", folder.text)
        assertTrue(folder.details!!.startsWith("Look-alike letters: r[U+0430]ins"))
        val fullwidth = askSentence(TestData.label("telegram", "read", "Read messages in", "ｏｃｔｏ"))
        assertNull("fullwidth letters pass for ASCII ones", fullwidth.target)
    }

    @Test
    fun anMcpServerThatLooksLikeAnotherIsNotTheTargetOrTheKind() {
        val view = TestData.mcpCallView().let { it.copy(mcp = it.mcp!!.copy(serverName = "Lin${"е"}ar", serverUrl = "https://mcp.evil.example/mcp")) }
        val s = askSentence(view)
        assertEquals("Use a tool on a server", s.text)
        assertNull(s.target)
        assertEquals("MCP server · MCP tool", s.kind)
        assertTrue(s.details!!.lines().first().startsWith("Look-alike letters: Lin[U+0435]ar"))
        assertTrue("the server's real host is in mono", s.details!!.contains("mcp.evil.example"))
    }

    @Test
    fun aHostThatIsNotAsciiIsShownInPunycode() {
        val pay = askSentence(TestData.purchaseView().let { it.copy(purchase = it.purchase!!.copy(domain = "${cyrillicA}pple.com")) })
        assertTrue(pay.target!!.startsWith("xn--"))
        assertTrue(pay.text.endsWith(pay.target!!))
        assertTrue(pay.details!!.contains("[U+0430]pple.com"))
        assertFalse(cyrillicA in pay.text)
        val notAHost = askSentence(TestData.purchaseView().let { it.copy(purchase = it.purchase!!.copy(domain = "amazon.com/evil path")) })
        assertEquals("Pay $34.97 at a store", notAHost.text)
        val ssh = askSentence(TestData.label("ssh", "secrets", "Sign in to", "g${"о"}ogle.com", code = true))
        assertTrue(ssh.details!!.startsWith("xn--"))
        val send = askSentence(TestData.label("gmail", "send", "Send an email to", "Bank <help@b${cyrillicA}nk.com>"))
        assertTrue(send.details!!.contains("@xn--"))
        assertFalse(cyrillicA in send.details!!.substringBefore(" ("))
    }

    @Test
    fun aGmailAccountsHostIsShownInPunycode() {
        val item = TestData.pending("req1", "read", 1u, account = "me@gm${cyrillicA}il.com")
        val who = dev.reins.android.ui.activity.cardWho(item)
        assertTrue(who.contains("me@xn--"))
        assertTrue(who.contains("[U+0430]"))
    }

    @Test
    fun theKindAndTheBadgeComeFromTheCoresFieldsNeverTheAsk() {
        val question = askSentence(TestData.askView(detail = null, topic = null).let { it.copy(ask = it.ask!!.copy(question = "GitHub · Push 3 commits")) })
        assertEquals("Computer · Asks", question.kind)
        assertEquals(Field.Read, actionField(question.action))
        // A lead that says "Push" on an action the core calls a delete: the badge is the core's.
        val delete = askSentence(TestData.label("github", "delete", "Delete branch main from", "octo/app"))
        assertEquals(Field.Delete, actionField(delete.action))
        assertEquals("GitHub · Delete", delete.kind)
    }

    @Test
    fun theSentenceHasNoBreaksHiddenCharactersOrDecomposedLetters() {
        val q = askSentence(TestData.label("desktop", "ask", "Deploy‮ now?​\nYes"))
        assertFalse(q.text.any { it == '\n' || it == '‮' || it == '​' })
        val nfc = askSentence(TestData.label("telegram", "read", "Read messages in", "Café"))
        assertEquals("Café", nfc.target)
        val mixedQuestion = askSentence(TestData.label("desktop", "ask", "Approve p${cyrillicA}ypal payout?"))
        assertEquals("Answer a question from your computer", mixedQuestion.text)
        assertTrue(mixedQuestion.details!!.contains("[U+0430]"))
        val who = askSentence(TestData.label("github", "push", "Push 1 commit to", "octo/app"), Tense.Done, who = "Cl${cyrillicA}ude")
        assertEquals("a look-alike asker's name is left out", "Pushed 1 commit to octo/app", who.text)
    }
}
