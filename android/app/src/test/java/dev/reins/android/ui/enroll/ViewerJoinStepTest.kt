package dev.reins.android.ui.enroll

import dev.reins.core.CoreException
import dev.reins.core.EnrollProgress
import dev.reins.core.EnrollView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

/** The joining phone's steps, from what each poll of the core says, and when the approving phone may approve. */
class ViewerJoinStepTest {
    @Test
    fun pollsMoveFromWaitingToTheCodeToTheEnd() {
        var step: ViewerJoinStep = ViewerJoinStep.Waiting
        step = step.after(EnrollProgress.Waiting)
        assertEquals(ViewerJoinStep.Waiting, step)
        step = step.after(EnrollProgress.Compare("482193"))
        assertEquals(ViewerJoinStep.Compare("482 193"), step)
        assertTrue(step.polling)
        step = step.after(EnrollProgress.Compare("482 193"))
        assertEquals(ViewerJoinStep.Compare("482 193"), step)
        step = step.after(EnrollProgress.Enrolled)
        assertEquals(ViewerJoinStep.Enrolled, step)
        assertFalse(step.polling)
        assertFalse("an enrollment is not asked again", step.ended)
    }

    @Test
    fun aCodeShownIsNotTakenBackByALateWaiting() {
        val compare = ViewerJoinStep.Compare("482 193")
        assertSame(compare, compare.after(EnrollProgress.Waiting))
    }

    @Test
    fun denialAndExpiryEndAndMayBeAskedAgain() {
        for ((progress, end) in listOf(EnrollProgress.Denied to ViewerJoinStep.Denied, EnrollProgress.Expired to ViewerJoinStep.Expired)) {
            val step = ViewerJoinStep.Compare("482 193").after(progress)
            assertEquals(end, step)
            assertTrue(step.ended)
            assertFalse(step.polling)
        }
    }

    @Test
    fun aLostConnectionKeepsWaitingAndAGoneRequestHasRunOut() {
        val compare = ViewerJoinStep.Compare("482 193")
        assertSame(compare, compare.afterFailure(CoreException.Network("offline")))
        assertEquals(ViewerJoinStep.Expired, compare.afterFailure(CoreException.NotFound()))
        val failed = ViewerJoinStep.Waiting.afterFailure(CoreException.Invalid("The challenge changed."))
        assertEquals(ViewerJoinStep.Failed("The challenge changed."), failed)
        assertTrue(failed.ended)
    }

    @Test
    fun theApprovingPhoneApprovesOnlyOnceBothShowTheCode() {
        val waiting = EnrollRequestUi(loading = false, view = EnrollView("j1", "Pixel 9", null, false, 1))
        assertFalse(waiting.canApprove)
        val shown = waiting.copy(view = waiting.view!!.copy(code = "482 193"))
        assertTrue(shown.canApprove)
        assertFalse(shown.copy(busy = true).canApprove)
        assertFalse(shown.copy(finished = true).canApprove)
    }
}
