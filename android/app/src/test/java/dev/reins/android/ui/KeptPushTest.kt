package dev.reins.android.ui

import dev.reins.android.ui.common.KeptCommit
import dev.reins.android.ui.common.KeptRef
import dev.reins.android.ui.common.keptPush
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** A push as an entry kept it in words (the core's `push_text`), read back without making anything up. */
class KeptPushTest {
    private val full = """
        git push katulevskiy/reins
        create branch signal/desktop-release: 3 commits
          1a2b3c4 Signal on the desktop
          5d6e7f8 Release notes
          9a0b1c2 Bump the version
        update branch main: 12 commits, force
          aaaaaaa Rewrite
        delete tag v0.1

        github_git_push
        repo: katulevskiy/reins
    """.trimIndent()

    @Test
    fun readsTheBranchesAndTheCommitsListed() {
        val push = keptPush(full)!!
        assertEquals("katulevskiy/reins", push.repo)
        assertEquals(3, push.refs.size)
        assertEquals(
            KeptRef(
                "create", "branch", "signal/desktop-release", 3, false,
                listOf(KeptCommit("1a2b3c4", "Signal on the desktop", false), KeptCommit("5d6e7f8", "Release notes", false), KeptCommit("9a0b1c2", "Bump the version", false)),
            ),
            push.refs[0],
        )
        assertEquals("12 commits gained, 1 listed", 12 to 1, push.refs[1].commitCount to push.refs[1].commits.size)
        assertTrue(push.refs[1].force)
        assertEquals(KeptRef("delete", "tag", "v0.1", 0, false, emptyList()), push.refs[2])
    }

    @Test
    fun aSubjectWithHiddenCharactersIsFlagged() {
        val push = keptPush("git push o/r\ncreate branch x: 1 commit\n  1234567 Fix �login\n\nrest")!!
        assertTrue(push.refs[0].commits[0].hidden)
        // Bidi controls never reach the screen.
        val bidi = keptPush("git push o/r\ncreate branch x: 1 commit\n  1234567 a‮b\n\nrest")!!
        assertEquals("ab", bidi.refs[0].commits[0].subject)
    }

    @Test
    fun anythingElseIsNoPush() {
        assertNull(keptPush(null))
        assertNull(keptPush(""))
        assertNull("not a push", keptPush("github_git_fetch\nrepo: o/r"))
        assertNull("a block the core cut short", keptPush("git push o/r\ncreate branch x: 3 commits\n  1234567 one…"))
        assertNull("a line it does not know", keptPush("git push o/r\ncreate branch x: 1 commit\nsomething else\n\nrest"))
        assertNull("a commit under a deletion", keptPush("git push o/r\ndelete branch x\n  1234567 one\n\nrest"))
        assertNull("a ref name with a space", keptPush("git push o/r\ncreate branch a b: 1 commit\n\nrest"))
        assertNull("no refs", keptPush("git push o/r\n\nrest"))
    }
}
