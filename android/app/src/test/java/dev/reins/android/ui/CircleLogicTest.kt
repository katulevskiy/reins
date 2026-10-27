package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.ui.circles.AddFlow
import dev.reins.android.ui.circles.AddPage
import dev.reins.android.ui.circles.approvedByMe
import dev.reins.android.ui.circles.canPause
import dev.reins.android.ui.circles.circleLink
import dev.reins.android.ui.circles.classLine
import dev.reins.android.ui.circles.endLabel
import dev.reins.android.ui.circles.formatCode
import dev.reins.android.ui.circles.forOthers
import dev.reins.android.ui.circles.groupPeople
import dev.reins.android.ui.circles.inviteCode
import dev.reins.android.ui.circles.inviteFinished
import dev.reins.android.ui.circles.inviteHelper
import dev.reins.android.ui.circles.noteOrNull
import dev.reins.android.ui.circles.personLine
import dev.reins.android.ui.circles.preset
import dev.reins.android.ui.circles.progressLine
import dev.reins.android.ui.circles.reviewNotificationTitle
import dev.reins.android.ui.circles.reviewStateLabel
import dev.reins.android.ui.circles.roleLabel
import dev.reins.android.ui.circles.shortName
import dev.reins.android.ui.circles.stateBadge
import dev.reins.android.ui.circles.windowLabel
import dev.reins.core.CircleInviteProgress
import dev.reins.core.CircleLinkState
import dev.reins.core.CircleReviewState
import dev.reins.core.CircleRole
import dev.reins.core.CircleScopePreset
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CircleLogicTest {
    private val now = 1_700_000_100L

    @Test
    fun `roles read as the screens name them`() {
        assertEquals(
            listOf("Delegate", "Co-sign", "Guardian", "Watcher"),
            listOf(CircleRole.DELEGATE, CircleRole.COSIGN, CircleRole.GUARDIAN, CircleRole.WATCHER).map(::roleLabel),
        )
    }

    @Test
    fun `people split into helping you and you help, ended ones gone, active first`() {
        val groups = groupPeople(
            listOf(
                TestData.circlePerson("a", "zoe@example.com"),
                TestData.circlePerson("b", "bob@example.com", state = CircleLinkState.PAUSED),
                TestData.circlePerson("c", "amy@example.com"),
                TestData.circlePerson("d", "sam@example.com", mine = false, role = CircleRole.GUARDIAN),
                TestData.circlePerson("e", "old@example.com", state = CircleLinkState.ENDED),
            ),
        )
        assertEquals(listOf("c", "a", "b"), groups.helpingYou.map { it.linkId })
        assertEquals(listOf("d"), groups.youHelp.map { it.linkId })
        assertFalse(groups.empty)
        assertTrue(groupPeople(listOf(TestData.circlePerson(state = CircleLinkState.ENDED))).empty)
    }

    @Test
    fun `a person's line says the role, the scope and how fast, a watcher has no class`() {
        assertEquals("Co-sign · Money · Deferred", personLine(TestData.circlePerson()))
        assertEquals("Delegate · Everything · Instant", personLine(TestData.circlePerson(role = CircleRole.DELEGATE, scopeLine = "Everything", deferred = false)))
        assertEquals("Watcher · Money", personLine(TestData.circlePerson(role = CircleRole.WATCHER)))
    }

    @Test
    fun `badges for paused and suspended, none while active`() {
        assertNull(stateBadge(CircleLinkState.ACTIVE))
        assertEquals("Paused", stateBadge(CircleLinkState.PAUSED))
        assertEquals("Suspended", stateBadge(CircleLinkState.SUSPENDED))
    }

    @Test
    fun `only the owner pauses, the owner ends and a helper leaves`() {
        assertTrue(canPause(TestData.circlePerson()))
        assertTrue(canPause(TestData.circlePerson(state = CircleLinkState.PAUSED)))
        assertFalse(canPause(TestData.circlePerson(state = CircleLinkState.SUSPENDED)))
        assertFalse(canPause(TestData.circlePerson(mine = false)))
        assertEquals("End", endLabel(TestData.circlePerson()))
        assertEquals("Leave", endLabel(TestData.circlePerson(mine = false)))
    }

    @Test
    fun `the owner's card takes the core's line, or makes one`() {
        assertEquals("Needs Alex too", progressLine(TestData.circleProgress()))
        assertEquals("Needs Alex too", progressLine(TestData.circleProgress(line = "")))
        assertEquals(
            "Alex approved, waiting for you",
            progressLine(TestData.circleProgress(needs = emptyList(), approved = listOf("alex@example.com"), line = "")),
        )
        assertEquals(
            "You approved · waiting for Alex",
            progressLine(TestData.circleProgress(waitingForYou = false, youApproved = true, line = "")),
        )
        assertTrue(approvedByMe(TestData.circleProgress(youApproved = true)))
        assertFalse(approvedByMe(TestData.circleProgress()))
        assertFalse(approvedByMe(null))
    }

    @Test
    fun `for others groups the waiting by person, the soonest deadline first`() {
        val shown = forOthers(
            listOf(
                TestData.circleReview("s1", "sam@example.com", left = 5 * 3_600),
                TestData.circleReview("d1", "dana@example.com", left = 600),
                TestData.circleReview("s2", "sam@example.com", left = 3_600),
                TestData.circleReview("d2", "dana@example.com", left = 7_200),
            ),
            now,
        )
        assertEquals(listOf("dana@example.com", "sam@example.com"), shown.waiting.map { it.person })
        assertEquals(listOf("d1", "d2"), shown.waiting[0].reviews.map { it.id })
        assertEquals(listOf("s2", "s1"), shown.waiting[1].reviews.map { it.id })
        assertEquals(4, shown.count)
        assertTrue(shown.settled.isEmpty())
    }

    @Test
    fun `settled ones stay a few days newest first, and a waiting one past its deadline is expired`() {
        val shown = forOthers(
            listOf(
                TestData.circleReview("ok", state = CircleReviewState.APPROVED, createdAt = now - 3_600),
                TestData.circleReview("no", state = CircleReviewState.DENIED, createdAt = now - 60),
                TestData.circleReview("old", state = CircleReviewState.WITHDRAWN, createdAt = now - 5 * 86_400),
                TestData.circleReview("late", left = -10, createdAt = now - 7_200),
            ),
            now,
        )
        assertTrue(shown.waiting.isEmpty())
        assertEquals(listOf("no", "ok", "late"), shown.settled.map { it.id })
        assertEquals("Expired", reviewStateLabel(shown.settled.last(), now))
        assertEquals("Approved", reviewStateLabel(shown.settled[1], now))
        assertEquals("Due in 3 h", reviewStateLabel(TestData.circleReview(), now))
    }

    @Test
    fun `the notification is titled by person`() {
        assertEquals("Sam: Buy headphones waiting", reviewNotificationTitle(TestData.circleReview()))
        assertEquals("Sam", shortName("sam@example.com"))
    }

    @Test
    fun `an empty note is none`() {
        assertNull(noteOrNull("   "))
        assertEquals("Not this one", noteOrNull(" Not this one "))
    }

    @Test
    fun `windows and the class read short`() {
        assertEquals("2 min", windowLabel(120))
        assertEquals("4 h", windowLabel(14_400))
        assertEquals("1 h 30 min", windowLabel(5_400))
        assertEquals("2 d", windowLabel(172_800))
        assertEquals("Deferred · 4 h", classLine(true, 14_400))
        assertEquals("Instant · 2 min", classLine(false, 120))
    }

    @Test
    fun `the code reads in two halves`() {
        assertEquals("482 913", formatCode("482913"))
        assertEquals("12345", formatCode("12345"))
    }

    @Test
    fun `an invitation's code and helper, and when it stops moving`() {
        val compare = CircleInviteProgress.Compare("482913", "alex@example.com")
        assertEquals("482913", inviteCode(compare))
        assertEquals("alex@example.com", inviteHelper(CircleInviteProgress.Accepted("482913", "alex@example.com")))
        assertNull(inviteCode(CircleInviteProgress.Waiting))
        assertFalse(inviteFinished(compare))
        assertTrue(inviteFinished(CircleInviteProgress.Active("l1")))
        assertTrue(inviteFinished(CircleInviteProgress.Cancelled))
    }

    @Test
    fun `presets map to the core's, one AI with its connection`() {
        assertEquals(CircleScopePreset.Money, preset("money"))
        assertEquals(CircleScopePreset.MessagesOutside, preset("messages"))
        assertEquals(CircleScopePreset.Agent("c1", "Claude"), preset("agent", "c1", "Claude"))
        assertNull(preset("agent"))
    }

    @Test
    fun `the add sheet's pages follow what was picked`() {
        assertEquals(AddPage.Role, AddFlow().page)
        assertEquals(AddPage.Scope, AddFlow(role = CircleRole.COSIGN).page)
        assertEquals(AddPage.Scope, AddFlow(role = CircleRole.COSIGN, preset = "agent").page)
        assertEquals(AddPage.Class, AddFlow(role = CircleRole.COSIGN, preset = "agent", agentId = "c1").page)
        assertEquals(AddPage.Class, AddFlow(role = CircleRole.COSIGN, preset = "money").page)
    }

    @Test
    fun `only a circle link is opened`() {
        assertEquals("reins-circle:host/abc", circleLink("  reins-circle:host/abc \n"))
        assertNull(circleLink("https://example.com"))
        assertNull(circleLink("reins-circle:"))
    }
}
