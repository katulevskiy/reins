package dev.reins.android.ui

import dev.reins.android.TestData
import dev.reins.android.TestData.attention
import dev.reins.android.ui.approval.attentionMark
import dev.reins.android.ui.approval.attentionOrder
import dev.reins.android.ui.approval.attentionSummary
import dev.reins.android.ui.approval.hiddenApart
import dev.reins.android.ui.approval.mailHeader
import dev.reins.android.ui.approval.mailMark
import dev.reins.android.ui.approval.rewritesHistory
import dev.reins.core.AttentionLevel
import dev.reins.core.GitCommitView
import dev.reins.core.MessageView
import dev.reins.core.ReasonCode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The iOS app's AttentionTextTests, case for case (the ticks are in ApprovalLogicTest). */
class AttentionTest {
    private fun commit(n: Int, attention: dev.reins.core.Attention? = null) =
        GitCommitView("c$n", "Commit $n", "Ada <ada@example.com>", attention = attention)

    private fun mail(id: String, sensitive: Boolean = false, attention: dev.reins.core.Attention? = null) =
        MessageView(id, "a@b.com", "Subject $id", 0, "", false, sensitive, attention)

    private val eight = (0 until 8).map { commit(it, if (it == 2) TestData.deletesAttention else if (it == 5) TestData.ciAttention else null) }

    @Test
    fun `the summary counts flagged items and says when not all are listed`() {
        assertEquals("2 of 8 to look at", attentionSummary(2, 8, 8))
        assertEquals("2 of 50 shown to look at", attentionSummary(2, 50, 200))
        assertNull("nothing flagged, no line", attentionSummary(0, 8, 8))
        assertEquals("2 of 8 to look at", attentionSummary(TestData.gitRef(commitCount = 8u, commits = eight)))
        assertEquals("2 of 8 shown to look at", attentionSummary(TestData.gitRef(commitCount = 9u, commits = eight)))
    }

    @Test
    fun `a history rewrite is said even with nothing flagged`() {
        val plain = (0 until 3).map { commit(it) }
        assertNull(attentionSummary(TestData.gitRef(commitCount = 3u, commits = plain, force = true)))
        assertTrue(rewritesHistory(TestData.gitRef(force = true)))
        assertTrue(rewritesHistory(TestData.gitRef(forceUnknown = true)))
        assertFalse(rewritesHistory(TestData.gitRef()))
    }

    @Test
    fun `the email header says how many to look at`() {
        val messages = TestData.attentionEmails()
        assertEquals("12 found · 2 to look at", mailHeader(messages, "gmail"))
        assertEquals("12 found · 10 shared · 2 to look at", mailHeader(messages, "gmail", shared = 10))
        assertEquals("1 found", mailHeader(listOf(mail("a")), "gmail"))
    }

    @Test
    fun `flagged rows come first and keep their original index`() {
        val rows = attentionOrder(eight) { it.attention?.level }
        // Close, then Look, then the rest in their order.
        assertEquals(listOf(5, 2, 0, 1, 3, 4, 6, 7), rows.map { it.index })
        // Each row opens the commit at its own index in the push, which is not reordered.
        rows.forEach { assertEquals(eight[it.index], it.value) }
        assertEquals((0 until 8).map { "c$it" }, eight.map { it.shortSha })
    }

    @Test
    fun `the order is stable within a band`() {
        val commits = listOf(commit(0, TestData.deletesAttention), commit(1, TestData.ciAttention), commit(2, TestData.deletesAttention), commit(3, TestData.ciAttention))
        assertEquals(listOf(1, 3, 0, 2), attentionOrder(commits) { it.attention?.level }.map { it.index })
    }

    @Test
    fun `the chip joins the reasons and TalkBack says how closely`() {
        val mark = attentionMark(attention(AttentionLevel.CLOSE, ReasonCode.CHANGES_CI to "Changes CI", ReasonCode.DELETES_LINES to "Deletes 1,240 lines"))!!
        assertEquals("Changes CI · Deletes 1,240 lines", mark.text)
        assertEquals("Look closer: Changes CI, Deletes 1,240 lines", mark.spoken)
        assertEquals("Worth a look: Deletes a file", attentionMark(TestData.deletesAttention)!!.spoken)
        assertNull(attentionMark(null))
        // The commit page keeps its own "Hidden characters" tag, so its chip leaves that reason out.
        assertNull(attentionMark(attention(AttentionLevel.CLOSE, ReasonCode.HIDDEN_CHARS to "Hidden characters"), setOf(ReasonCode.HIDDEN_CHARS)))
    }

    @Test
    fun `hidden characters stay shown when heavier reasons fill the chip`() {
        // The core keeps three reasons: a secret file, a key and CI outweigh hidden characters, which drop out.
        val full = attention(
            AttentionLevel.CLOSE,
            ReasonCode.SECRET_FILE to "Secret file", ReasonCode.LOOKS_LIKE_KEY to "Looks like a key", ReasonCode.CHANGES_CI to "Changes CI",
        )
        assertTrue("the row keeps its own Hidden tag", hiddenApart(true, full))
        assertFalse("Hidden characters" in attentionMark(full)!!.labels)
        // When the chip names them, one tag is enough; without hidden characters, none.
        assertFalse(hiddenApart(true, attention(AttentionLevel.CLOSE, ReasonCode.CHANGES_CI to "Changes CI", ReasonCode.HIDDEN_CHARS to "Hidden characters")))
        assertTrue(hiddenApart(true, null))
        assertFalse(hiddenApart(false, full))
    }

    @Test
    fun `a code row has one chip`() {
        val code = mail("c", sensitive = true, attention = attention(AttentionLevel.CLOSE, ReasonCode.SIGN_IN_CODE to "Looks like a sign-in code"))
        assertEquals(listOf("Code"), mailMark(code, "gmail")!!.labels)
        val both = mail(
            "b", sensitive = true,
            attention = attention(AttentionLevel.CLOSE, ReasonCode.SIGN_IN_CODE to "Looks like a sign-in code", ReasonCode.PASSWORD_RESET to "Password reset"),
        )
        assertEquals("Code · Password reset", mailMark(both, "gmail")!!.text)
        val vault = mail("v", sensitive = true, attention = attention(AttentionLevel.CLOSE, ReasonCode.PASSWORD to "Password"))
        assertEquals(listOf("Password"), mailMark(vault, "vault")!!.labels)
        // A sensitive message from an older core, with no reasons, keeps its chip.
        assertEquals(listOf("Code"), mailMark(mail("o", sensitive = true), "telegram")!!.labels)
        assertNull(mailMark(mail("p"), "gmail"))
    }
}
