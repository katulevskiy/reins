package dev.rewarden.android.ui

import dev.rewarden.android.TestData
import dev.rewarden.android.ui.approval.GitHistory
import dev.rewarden.android.ui.approval.gitChipLabel
import dev.rewarden.android.ui.approval.gitFileCounts
import dev.rewarden.android.ui.approval.gitFileLetter
import dev.rewarden.android.ui.approval.gitHistory
import dev.rewarden.android.ui.approval.gitTotalsLabel
import dev.rewarden.android.ui.approval.packSizeLabel
import org.junit.Assert.assertEquals
import org.junit.Test

class GitPushFormatTest {
    @Test
    fun `each kind of ref change has its own chip`() {
        assertEquals("New branch", gitChipLabel(TestData.gitRef(change = "create")))
        assertEquals("Update", gitChipLabel(TestData.gitRef(change = "update")))
        assertEquals("Delete", gitChipLabel(TestData.gitRef(change = "delete")))
        assertEquals("New tag", gitChipLabel(TestData.gitRef("v1.2", kind = "tag", change = "create")))
        assertEquals("Tag", gitChipLabel(TestData.gitRef("v1.2", kind = "tag", change = "update")))
        assertEquals("Delete", gitChipLabel(TestData.gitRef("v1.2", kind = "tag", change = "delete")))
        assertEquals("New", gitChipLabel(TestData.gitRef("refs/notes/x", kind = "other", change = "create")))
    }

    @Test
    fun `a force push is told apart from one whose history could not be checked`() {
        assertEquals(GitHistory.Adds, gitHistory(TestData.gitRef()))
        assertEquals(GitHistory.Rewrites, gitHistory(TestData.gitRef(force = true)))
        assertEquals(GitHistory.Unknown, gitHistory(TestData.gitRef(force = true, forceUnknown = true)))
        // The core sets both together; an unknown history alone still warns.
        assertEquals(GitHistory.Unknown, gitHistory(TestData.gitRef(forceUnknown = true)))
    }

    @Test
    fun `line totals leave out what could not be counted`() {
        assertEquals("+120 −14 in 12 files", gitTotalsLabel(TestData.gitRef()))
        assertEquals("+1 −0 in 1 file", gitTotalsLabel(TestData.gitRef(filesChanged = 1u, additions = 1u, deletions = 0u)))
        assertEquals("+5 in 2 files", gitTotalsLabel(TestData.gitRef(filesChanged = 2u, additions = 5u, deletions = null)))
        assertEquals("12 files changed", gitTotalsLabel(TestData.gitRef(additions = null, deletions = null)))
        assertEquals(null, gitTotalsLabel(TestData.gitRef(filesChanged = 0u, files = emptyList())))
    }

    @Test
    fun `files show a status letter and their own counts`() {
        assertEquals("A", gitFileLetter("added"))
        assertEquals("M", gitFileLetter("modified"))
        assertEquals("D", gitFileLetter("deleted"))
        assertEquals("T", gitFileLetter("type_changed"))
        assertEquals("?", gitFileLetter("something new"))
        assertEquals("+3 −1", gitFileCounts(TestData.gitFile("a")))
        assertEquals("binary", gitFileCounts(TestData.gitFile("logo.png", additions = null, deletions = null, binary = true)))
        assertEquals("+7", gitFileCounts(TestData.gitFile("a", additions = 7u, deletions = null)))
        assertEquals("", gitFileCounts(TestData.gitFile("huge.json", additions = null, deletions = null)))
    }

    @Test
    fun `pack sizes are human readable`() {
        assertEquals("0 B", packSizeLabel(0u))
        assertEquals("999 B", packSizeLabel(999u))
        assertEquals("12.4 KB", packSizeLabel(12_700u))
        assertEquals("3.0 MB", packSizeLabel(3_145_728u))
        assertEquals("1.5 GB", packSizeLabel(1_610_612_736u))
    }
}
