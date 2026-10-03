package dev.reins.android.design

import dev.reins.android.TestData
import dev.reins.android.ui.common.endedLine
import dev.reins.android.ui.common.findConnection
import dev.reins.android.ui.common.entryTitle
import dev.reins.android.ui.common.inactiveWord
import dev.reins.android.ui.grants.CustomUnit
import dev.reins.android.ui.grants.ResumePeriod
import dev.reins.android.ui.grants.ResumeResult
import dev.reins.android.ui.grants.buildResume
import dev.reins.android.ui.grants.initialResumeDraft
import dev.reins.android.ui.grants.resumePeriods
import dev.reins.android.ui.common.operationTitle
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.grants.NewGrantDraft
import dev.reins.android.ui.grants.NewGrantLifetime
import dev.reins.android.ui.grants.NewGrantResult
import dev.reins.android.ui.grants.buildNewGrant
import dev.reins.android.ui.grants.parseParties
import dev.reins.core.ApprovalKind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DesignLogicTest {
    @Test
    fun `operations are named in a few words`() {
        assertEquals("Search Gmail", operationTitle("search", 12, "gmail"))
        assertEquals("Read 1 email", operationTitle("read", 1, "gmail"))
        assertEquals("Read email", operationTitle("read", 0, "gmail"))
        assertEquals("See Gmail accounts", operationTitle("accounts", 2, "gmail"))
        assertEquals("See integrations", operationTitle("accounts", 1, ""))
        assertEquals("Read 5 emails", operationTitle("read", 5, "gmail"))
        assertEquals("Send email", operationTitle("send", 1, "gmail"))
        assertEquals("Send email to 3", operationTitle("send", 3, "gmail"))
        assertEquals("Claude: Search Gmail", dev.reins.android.ui.common.fullTitle("Claude", "search", 2, "gmail"))
    }

    @Test
    fun `an operation of another integration is named by the core, with a generic name as fallback`() {
        assertEquals("Read Telegram messages", operationTitle("read", 3, "telegram", "Read Telegram messages"))
        assertEquals("Commit a file to GitHub", operationTitle("write", 1, "github", "Commit a file to GitHub"))
        assertEquals("Claude: Delete a GitHub repository", dev.reins.android.ui.common.fullTitle("Claude", "write", 1, "github", "Delete a GitHub repository"))
        assertEquals("Hermes: Send a text message", entryTitle("Hermes", "send", 1, "sms", "sent", "Send a text message"))
        assertEquals("Read GitHub", operationTitle("read", 3, "github"))
        assertEquals("Change GitHub", operationTitle("write", 1, "github", ""))
        assertEquals("List Telegram", operationTitle("list", 1, "telegram", "  "))
        assertEquals("Send with Telegram", operationTitle("send", 1, "telegram"))
    }

    @Test
    fun `a permission reads as what happened to it`() {
        assertEquals("Hermes: Access granted", entryTitle("Hermes", "grant", 1, "gmail", "granted"))
        assertEquals("Hermes: Access refused", entryTitle("Hermes", "grant", 1, "gmail", "denied"))
        assertEquals("Hermes: Read 2 emails", entryTitle("Hermes", "read", 2, "gmail", "released"))
    }

    @Test
    fun `each kind of operation has its own colour`() {
        val c = RColors.Dark
        val tints = listOf(ActionKind.Search, ActionKind.Read, ActionKind.Send, ActionKind.Pair).map { it.tint(c) }
        assertEquals("search, read, send and connect must all differ", tints.size, tints.toSet().size)
        assertEquals(ActionKind.Grant, ActionKind.of("grant"))
        assertEquals(ActionKind.Other, ActionKind.of("something-new"))
    }

    @Test
    fun `known AIs are recognised from their names and others get a shape`() {
        assertEquals("claude", Providers.infer("My Claude")?.key)
        assertEquals("openai", Providers.infer("ChatGPT (work)")?.key)
        assertEquals("grok", Providers.infer("xAI Grok")?.key)
        assertEquals("hermes", Providers.infer("hermes agent")?.key)
        assertNull(Providers.infer("notes-bot"))
        assertNotNull(Providers.byKey("gemini"))
        assertNull(Providers.byKey("blob"))
    }

    @Test
    fun `the countdown runs from the request to the moment the AI stops waiting`() {
        val created = 1_000L
        val until = 1_045L
        val fresh = urgency(created, until, 1_005_000)!!
        assertEquals(40L, fresh.remainingSeconds)
        assertFalse(fresh.urgent || fresh.stale)
        assertEquals(40f / 45f, fresh.fraction, 0.001f)
        val urgent = urgency(created, until, 1_031_000)!!
        assertTrue("red for the last 15 seconds", urgent.urgent && !urgent.stale)
        assertFalse(urgency(created, until, 1_029_000)!!.urgent)
        val late = urgency(created, until, 1_046_000)!!
        assertTrue(late.stale && late.fraction == 0f && !late.urgent)
        assertNull("no deadline, no countdown", urgency(created, null, 1_000_000))
    }

    @Test
    fun `relative times are short`() {
        val now = 100_000L
        assertEquals("just now", relativeTime(now - 10, now))
        assertEquals("5 min ago", relativeTime(now - 300, now))
        assertEquals("3 h ago", relativeTime(now - 3 * 3600, now))
        assertEquals("yesterday", relativeTime(now - 90_000, now))
    }

    @Test
    fun `new grant text is split into addresses and domains`() {
        val (addresses, domains) = parseParties("Alerts@Bank.com, @statements.bank.com  news.example\nalerts@bank.com")
        assertEquals(listOf("alerts@bank.com"), addresses)
        assertEquals(listOf("statements.bank.com", "news.example"), domains)
    }

    @Test
    fun `a new read grant needs someone, a one-time grant has no expiry`() {
        assertTrue(buildNewGrant(NewGrantDraft(connectionId = "c1")) is NewGrantResult.Invalid)
        assertTrue(buildNewGrant(NewGrantDraft(partiesText = "a@b.com")) is NewGrantResult.Invalid) // no connection
        val ok = buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", partiesText = "@bank.com", lifetime = NewGrantLifetime.ONE_TIME)) as NewGrantResult.Ok
        assertEquals(ApprovalKind.READ, ok.kind)
        assertEquals(1u, ok.standing.maxUses)
        assertNull(ok.standing.durationSecs)
        assertEquals(listOf("bank.com"), ok.standing.scope.senderDomains)
    }

    @Test
    fun `all mail for a new grant is time-boxed to a week at most and never for sending`() {
        val week = buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", anyMail = true, lifetime = NewGrantLifetime.WEEK)) as NewGrantResult.Ok
        assertTrue(week.standing.scope.allMail)
        assertEquals(604_800UL, week.standing.durationSecs)
        assertTrue(buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", anyMail = true, lifetime = NewGrantLifetime.MONTH)) is NewGrantResult.Invalid)
        assertTrue(buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", anyMail = true, lifetime = NewGrantLifetime.ONE_TIME)) is NewGrantResult.Invalid)
        assertTrue(buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", send = true, anyMail = true, lifetime = NewGrantLifetime.DAY)) is NewGrantResult.Invalid)
    }

    @Test
    fun `a send grant lists its recipients`() {
        assertTrue(buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", send = true)) is NewGrantResult.Invalid)
        val ok = buildNewGrant(NewGrantDraft("c1", account = "me@gmail.com", send = true, partiesText = "ann@corp.com @corp.com", subject = " report ", lifetime = NewGrantLifetime.HOUR)) as NewGrantResult.Ok
        assertEquals(ApprovalKind.SEND, ok.kind)
        assertEquals(listOf("ann@corp.com"), ok.standing.scope.recipientAddresses)
        assertEquals(listOf("corp.com"), ok.standing.scope.recipientDomains)
        assertEquals("report", ok.standing.scope.subjectPattern)
        assertTrue(ok.standing.scope.senderAddresses.isEmpty())
    }

    // ---- grant clocks --------------------------------------------------------------------------------------------

    @Test
    fun `an entry without a connection id finds its connection by label, when that is unambiguous`() {
        val claude = TestData.connection("c1", "Claude")
        val other = TestData.connection("c2", "Other")
        assertEquals("c1", findConnection(listOf(claude, other), "c1", "x")?.id)
        assertEquals("c1", findConnection(listOf(claude, other), "", "Claude")?.id)
        assertNull(findConnection(listOf(claude, claude.copy(id = "c3")), "", "Claude"))
        assertNull(findConnection(listOf(claude), "gone", "Claude"))
    }

    @Test
    fun `time left is written in its largest unit`() {
        assertEquals("0s", compactDuration(0))
        assertEquals("45s", compactDuration(45))
        assertEquals("1m", compactDuration(60))
        assertEquals("47m", compactDuration(47 * 60 + 59))
        assertEquals("1h", compactDuration(3_600))
        assertEquals("3h", compactDuration(3 * 3_600 + 1_799))
        assertEquals("23h", compactDuration(86_399))
        assertEquals("1d", compactDuration(86_400))
        assertEquals("5d", compactDuration(5 * 86_400 + 100))
        assertEquals("6d", compactDuration(7 * 86_400 - 1))
        assertEquals("1w", compactDuration(7 * 86_400))
        assertEquals("10w", compactDuration(10 * 7 * 86_400 + 3))
        assertEquals("52w", compactDuration(364 * 86_400))
        assertEquals("1y", compactDuration(365 * 86_400))
        assertEquals("2y", compactDuration(2 * 365 * 86_400 + 5))
        assertEquals("0s", compactDuration(-5))
    }

    @Test
    fun `a grant is about to end in the last tenth of its life, between five minutes and an hour`() {
        assertEquals(300, expiryLeadSeconds(0, 600))
        assertEquals(360, expiryLeadSeconds(0, 3_600))
        assertEquals(3_600, expiryLeadSeconds(0, 86_400))
        assertEquals(3_600, expiryLeadSeconds(0, 30 * 86_400))
        val g = TestData.grant("g", leftSeconds = 3_000, ageSeconds = 600)
        assertEquals(g.expiresAt!! - 360, reminderAt(g))
        assertNull("nothing to remind about", reminderAt(g.copy(expiresAt = null)))
    }

    @Test
    fun `the clock of a grant knows the fraction left, whether it is ending soon, and never ends when it has no end`() {
        val g = TestData.grant("g", leftSeconds = 900, ageSeconds = 2_700)
        val now = g.createdAt + 2_700
        val clock = grantClock(g, now)
        assertEquals(900L, clock.remaining)
        assertEquals(0.25f, clock.fraction, 0.001f)
        assertEquals("15m", clock.label)
        assertFalse(clock.soon)
        assertTrue(grantClock(g, now + 600).soon)
        assertEquals(0L, grantClock(g, g.expiresAt!! + 10).remaining)
        val open = grantClock(g.copy(expiresAt = null), now)
        assertEquals(Triple(null, 1f, false), Triple(open.remaining, open.fraction, open.soon))
        assertEquals("\u221e", open.label)
        assertFalse("an ended grant is never ending soon", grantClock(g.copy(active = false, state = "expired"), now + 800).soon)
    }

    @Test
    fun `an ended grant says how it ended and can be resumed for a sensible period`() {
        assertEquals("Deleted", endedLine(TestData.grant("a", active = false, state = "revoked")))
        assertEquals("All 3 uses spent", endedLine(TestData.grant("a", active = false, state = "used_up", maxUses = 3u, uses = 3u)))
        assertTrue(endedLine(TestData.grant("a", active = false)).startsWith("Expired "))
        assertEquals(listOf("Deleted", "Used up", "Expired"), listOf("revoked", "used_up", "expired").map { inactiveWord(TestData.grant("a", active = false, state = it)) })
        assertEquals(4, resumePeriods(allMail = false).size)
        assertEquals(listOf(ResumePeriod.HOUR, ResumePeriod.DAY, ResumePeriod.WEEK), resumePeriods(allMail = true))
    }

    // ---- resuming with changes ----------------------------------------------------------------------------------

    private fun ended(action: String = "read", allMail: Boolean = false, maxUses: UInt? = null, editable: dev.reins.core.GrantScopeChoice? = TestData.defaultScope(action)) =
        TestData.grant("old", active = false, action = action, allMail = allMail, maxUses = maxUses, editable = editable)

    private fun ok(result: ResumeResult) = result as ResumeResult.Ok

    @Test
    fun `resuming as it was is a plain resume for the chosen period`() {
        val grant = ended()
        val draft = initialResumeDraft(grant)
        assertEquals("@bank.com", draft.partiesText)
        val plain = ok(buildResume(grant, draft.copy(period = ResumePeriod.WEEK)))
        assertEquals(Pair(604_800L, null), Pair(plain.seconds, plain.standing))
    }

    @Test
    fun `a custom time wins over the quick choice and is checked`() {
        val grant = ended()
        val base = initialResumeDraft(grant)
        assertEquals(5_400L, ok(buildResume(grant, base.copy(customAmount = "90", customUnit = CustomUnit.MINUTES))).seconds)
        assertEquals(3 * 86_400L, ok(buildResume(grant, base.copy(customAmount = "3", customUnit = CustomUnit.DAYS))).seconds)
        assertTrue(buildResume(grant, base.copy(customAmount = "0")) is ResumeResult.Invalid)
        assertTrue(buildResume(grant, base.copy(customAmount = "31", customUnit = CustomUnit.DAYS)) is ResumeResult.Invalid)
        assertTrue(buildResume(grant, base.copy(customAmount = "59", customUnit = CustomUnit.MINUTES)).let { it is ResumeResult.Ok })
        assertTrue(buildResume(grant, base.copy(customAmount = "x")) is ResumeResult.Invalid)
    }

    @Test
    fun `changing who it covers or how often makes an edited resume`() {
        val grant = ended()
        val base = initialResumeDraft(grant)
        val moved = ok(buildResume(grant, base.copy(partiesText = "ann@corp.com, @statements.example", subject = " invoice ")))
        val scope = moved.standing!!.scope
        assertEquals(Triple(listOf("ann@corp.com"), listOf("statements.example"), "invoice"), Triple(scope.senderAddresses, scope.senderDomains, scope.subjectPattern))
        assertEquals(86_400uL, moved.standing!!.durationSecs)
        val limited = ok(buildResume(grant, base.copy(limitUses = true, uses = "3")))
        assertEquals(3u, limited.standing!!.maxUses)
        assertTrue(buildResume(grant, base.copy(limitUses = true, uses = "0")) is ResumeResult.Invalid)
        assertTrue(buildResume(grant, base.copy(partiesText = "", subject = "")) is ResumeResult.Invalid)
        val same = ok(buildResume(grant, base.copy(partiesText = "@BANK.com")))
        assertNull("the same senders in other spelling changed nothing", same.standing)
    }

    @Test
    fun `all mail can be resumed for a week at most and never for sending`() {
        val grant = ended()
        val base = initialResumeDraft(grant).copy(allMail = true, partiesText = "")
        assertTrue(ok(buildResume(grant, base.copy(period = ResumePeriod.WEEK))).standing!!.scope.allMail)
        assertTrue(buildResume(grant, base.copy(period = ResumePeriod.MONTH)) is ResumeResult.Invalid)
        assertTrue(buildResume(grant, base.copy(customAmount = "8", customUnit = CustomUnit.DAYS)) is ResumeResult.Invalid)
        val send = ended("send")
        assertTrue(buildResume(send, initialResumeDraft(send).copy(partiesText = "")) is ResumeResult.Invalid)
        assertEquals(listOf("corp.example"), ok(buildResume(send, initialResumeDraft(send).copy(partiesText = "@corp.example, bob@corp.example"))).standing!!.scope.recipientDomains)
    }

    @Test
    fun `a grant the editor cannot describe only comes back as it was`() {
        val grant = ended(editable = null)
        val result = ok(buildResume(grant, initialResumeDraft(grant).copy(partiesText = "x@y.com", limitUses = true, uses = "2")))
        assertNull(result.standing)
    }
}
