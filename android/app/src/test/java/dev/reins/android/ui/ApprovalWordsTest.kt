package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.approval.ApprovalDraft
import dev.reins.android.ui.approval.Badge
import dev.reins.android.ui.approval.BuildResult
import dev.reins.android.ui.approval.CustomUnit
import dev.reins.android.ui.approval.HeroTone
import dev.reins.android.ui.approval.LifetimeKind
import dev.reins.android.ui.approval.Segment
import dev.reins.android.ui.approval.approveLabel
import dev.reins.android.ui.approval.badge
import dev.reins.android.ui.approval.buildDecision
import dev.reins.android.ui.approval.customLabel
import dev.reins.android.ui.approval.dangerTag
import dev.reins.android.ui.approval.denyLabel
import dev.reins.android.ui.approval.heroSentence
import dev.reins.android.ui.approval.heroTone
import dev.reins.android.ui.approval.lifetimeSeconds
import dev.reins.android.ui.approval.periodWords
import dev.reins.android.ui.approval.rememberButton
import dev.reins.android.ui.approval.rememberLocked
import dev.reins.android.ui.approval.secondsLabel
import dev.reins.android.ui.approval.segmentOf
import dev.reins.android.ui.approval.shortLabel
import dev.reins.android.ui.approval.splitPath
import dev.reins.core.ApprovalView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The approval screen's words, colours and lifetimes (ApprovalWords.kt, ApprovalLogic.kt). */
class ApprovalWordsTest {
    private fun sentence(view: ApprovalView) = heroSentence(view).joinToString("") { if (it.bold) "**${it.text}**" else it.text }

    private fun ok(r: BuildResult) = (r as BuildResult.Ok).choice

    @Test
    fun `the hero says what is asked in one sentence, the key nouns bold`() {
        assertEquals("Wants to **push 8 commits**\nto **octo/app**", sentence(TestData.designPushView()))
        assertEquals("Wants to **read 3 emails**\nfrom **Stripe**", sentence(TestData.designMailView(null)))
        assertEquals("Wants to **read 3 emails**\nfound for **from:bank**", sentence(TestData.searchView()), )
        assertEquals("Wants to **delete** the repository\n**octo/old-api**", sentence(TestData.designDeleteView()))
        assertEquals("Wants to **send an email**\nto **Ann** and 1 more", sentence(TestData.sendView()))
        assertEquals("**Force-push main?**", sentence(TestData.askView()))
        assertEquals("Wants to **create issue**\non **Linear**", sentence(TestData.mcpCallView()))
        assertEquals("Wants to **read Telegram messages**\n**Family**", sentence(TestData.fetchView()))
        assertEquals("Wants to **read emails**\n**without asking**", sentence(TestData.grantView()))
        val tag = TestData.gitPushView(TestData.gitPush(TestData.gitRef("v1.2", kind = "tag", change = "create")), tags = true)
        assertEquals("Wants to **push tag v1.2**\nto **octo/app**", sentence(tag))
        val gone = TestData.gitPushView(TestData.gitPush(TestData.gitRef("old", change = "delete", commitCount = 0u, commits = emptyList())))
        assertEquals("Wants to **delete branch old**\nfrom **octo/app**", sentence(gone))
    }

    @Test
    fun `untrusted text cannot reorder the sentence`() {
        val view = TestData.designMailView(null).copy(messages = listOf(TestData.message("m1", "Ev‮il <a@b.c>")))
        assertFalse(heroSentence(view).any { s -> s.text.any { it == '‮' } })
    }

    @Test
    fun `the hero is tinted by what is asked`() {
        assertEquals(HeroTone.Write, heroTone(TestData.designPushView()))
        assertEquals(HeroTone.Search, heroTone(TestData.searchView()))
        assertEquals(HeroTone.Send, heroTone(TestData.sendView()))
        assertEquals(HeroTone.Grant, heroTone(TestData.grantView()))
        assertEquals(HeroTone.Grant, heroTone(TestData.accountsView()))
        assertEquals(HeroTone.Danger, heroTone(TestData.designDeleteView()))
        assertEquals(HeroTone.Search, heroTone(TestData.vaultView()), "a password is asked every time but changes nothing")
        assertEquals(Badge.Trash, badge(TestData.designDeleteView()))
        assertEquals(Badge.Pen, badge(TestData.designPushView()))
        assertEquals(Badge.Search, badge(TestData.searchView()))
        assertEquals(Badge.Question, badge(TestData.askView()))
    }

    private fun assertEquals(expected: Any?, actual: Any?, message: String) = org.junit.Assert.assertEquals(message, expected, actual)

    @Test
    fun `approve says what it does`() {
        val draft = ApprovalDraft(selected = setOf("m1", "m2"))
        assertEquals("Share 3", approveLabel(TestData.searchView(), draft), )
        assertEquals("Share 2", approveLabel(TestData.designMailView(null), draft))
        assertEquals("Yes", approveLabel(TestData.askView(), draft))
        assertEquals("No", denyLabel(TestData.askView()))
        assertEquals("Allow", approveLabel(TestData.grantView(), draft))
        assertEquals("Delete", approveLabel(TestData.designDeleteView(), draft))
        assertEquals("Send", approveLabel(TestData.sendView(), draft))
        assertEquals("Approve", approveLabel(TestData.designPushView(), draft))
        assertTrue(rememberLocked(TestData.designDeleteView()))
        assertFalse(rememberLocked(TestData.designPushView()))
    }

    @Test
    fun `dangerous pushes and destructive tools carry a tag with its explanation`() {
        assertEquals("forceWarning:0", dangerTag(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true)), noStanding = true))?.tag)
        assertEquals("forceUnknown:0", dangerTag(TestData.gitPushView(TestData.gitPush(TestData.gitRef(force = true, forceUnknown = true)), noStanding = true))?.tag)
        assertEquals("mcpDestructive", dangerTag(TestData.mcpCallView(destructive = true))?.tag)
        assertEquals("onceWarning", dangerTag(TestData.designDeleteView())?.tag)
        assertNull(dangerTag(TestData.designPushView()))
        val hidden = TestData.gitRef(commits = listOf(TestData.gitCommit(1).copy(hiddenChars = true)))
        assertEquals("hiddenChars", dangerTag(TestData.gitPushView(TestData.gitPush(hidden)))?.tag)
    }

    @Test
    fun `the dock's segments and the custom choice read as short periods`() {
        val view = TestData.designPushView()
        assertEquals("1 hour", shortLabel(view))
        assertEquals("8 hours", shortLabel(TestData.quick(TestData.searchView(), repeats = 3u)))
        assertEquals(Segment.Once, segmentOf(ApprovalDraft()))
        assertEquals(Segment.Short, segmentOf(ApprovalDraft(lifetime = LifetimeKind.HOUR)))
        assertEquals(Segment.Custom, segmentOf(ApprovalDraft(lifetime = LifetimeKind.WEEK)))
        assertEquals("Custom", customLabel(ApprovalDraft()))
        assertEquals("3 days", customLabel(ApprovalDraft(lifetime = LifetimeKind.CUSTOM)))
        assertEquals("1 week", customLabel(ApprovalDraft(lifetime = LifetimeKind.WEEK)))
        assertEquals("10 uses", customLabel(ApprovalDraft(lifetime = LifetimeKind.USES, uses = 10)))
        assertEquals("Forever", customLabel(ApprovalDraft(lifetime = LifetimeKind.UNTIL_REVOKED)))
        assertEquals("All mail", customLabel(ApprovalDraft(allMail = LifetimeKind.HOUR)))
        assertEquals("Remember until I revoke", rememberButton(view, ApprovalDraft(lifetime = LifetimeKind.UNTIL_REVOKED)))
        assertEquals("Remember for 2 weeks", rememberButton(view, ApprovalDraft(lifetime = LifetimeKind.CUSTOM, customAmount = 2, customUnit = CustomUnit.WEEKS)))
        assertEquals("1 day", periodWords(86_400))
        assertEquals("30 days", periodWords(2_592_000))
        assertEquals("40s", secondsLabel(40))
        assertEquals("2m", secondsLabel(101))
        assertEquals("src/auth/" to "login.ts", splitPath("src/auth/login.ts"))
        assertEquals("" to "README.md", splitPath("README.md"))
    }

    @Test
    fun `a custom time is counted in its unit and bounded`() {
        val view = TestData.writeView()
        assertEquals(259_200uL, lifetimeSeconds(view, ApprovalDraft(lifetime = LifetimeKind.CUSTOM)))
        assertEquals(7_200uL, lifetimeSeconds(view, ApprovalDraft(lifetime = LifetimeKind.CUSTOM, customAmount = 2, customUnit = CustomUnit.HOURS)))
        assertNull(lifetimeSeconds(view, ApprovalDraft(lifetime = LifetimeKind.UNTIL_REVOKED)))
        val read = TestData.searchView()
        assertTrue(buildDecision(read, ApprovalDraft(setOf("m1"), LifetimeKind.CUSTOM, customAmount = 0)) is BuildResult.Invalid)
        assertTrue(buildDecision(read, ApprovalDraft(setOf("m1"), LifetimeKind.CUSTOM, customAmount = 53, customUnit = CustomUnit.WEEKS)) is BuildResult.Invalid)
        assertEquals(1_209_600uL, ok(buildDecision(read, ApprovalDraft(setOf("m1"), LifetimeKind.CUSTOM, customAmount = 2, customUnit = CustomUnit.WEEKS))).standing!!.durationSecs)
    }

    @Test
    fun `remembering covers anything like this unless the user picks what it covers`() {
        val view = TestData.quick(TestData.searchView())
        val like = view.quick!!.allow!!
        // "1 hour" is the core's own "allow for a while": its scope and its period.
        val hour = ok(buildDecision(view, ApprovalDraft(setOf("m1", "m2"), LifetimeKind.HOUR)))
        assertEquals(like, hour.standing)
        assertEquals(listOf("m1", "m2", "m3"), hour.selectedMessageIds)
        assertEquals(28_800uL, ok(buildDecision(TestData.quick(TestData.searchView(), repeats = 3u), ApprovalDraft(setOf("m1"), LifetimeKind.HOUR))).standing!!.durationSecs)
        // A day of the same, then the picked scope once the user chose it.
        assertEquals(like.scope, ok(buildDecision(view, ApprovalDraft(setOf("m1"), LifetimeKind.DAY))).standing!!.scope)
        assertTrue(ok(buildDecision(view, ApprovalDraft(setOf("m1"), LifetimeKind.DAY, likeThis = false))).standing!!.scope.selectedMessagesOnly)
        // All mail is never open-ended: past a week, the ticked emails only.
        val forever = ok(buildDecision(view, ApprovalDraft(setOf("m1"), LifetimeKind.UNTIL_REVOKED))).standing!!
        assertFalse(forever.scope.allMail)
        assertTrue(forever.scope.selectedMessagesOnly)
        assertNull("once remembers nothing", ok(buildDecision(view, ApprovalDraft(setOf("m1")))).standing)
        // Nothing is ever remembered for what is asked every time.
        assertNull("asked every time", ok(buildDecision(TestData.quick(TestData.vaultView()), ApprovalDraft(setOf("git:password"), LifetimeKind.HOUR))).standing)
    }
}
