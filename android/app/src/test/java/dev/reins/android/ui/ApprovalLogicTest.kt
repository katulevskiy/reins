package dev.reins.android.ui

import dev.reins.android.ui.approval.ApprovalDraft
import dev.reins.android.ui.approval.BuildResult
import dev.reins.android.ui.approval.LifetimeKind
import dev.reins.android.ui.approval.PublicMailDomains
import dev.reins.android.ui.approval.addressOf
import dev.reins.android.ui.approval.defaultClasses
import dev.reins.android.ui.approval.defaultResources
import dev.reins.android.ui.approval.resourceCovers
import dev.reins.android.ui.approval.toggleClass
import dev.reins.android.ui.approval.toggleResource
import dev.reins.android.ui.approval.buildChoice
import dev.reins.android.ui.approval.publicDomainWarnings
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.EmailView
import dev.reins.core.MessageView
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ApprovalLogicTest {
    private fun message(id: String, from: String, covered: Boolean = false) =
        MessageView(id, from, "Subject $id", 0L, "snippet", covered, false)

    private val readView = ApprovalView(
        requestId = "r1", connectionId = "c1", connectionLabel = "Claude", kind = ApprovalKind.SEARCH, query = "from:bank",
        messages = listOf(
            message("m1", "Bank <alerts@bank.com>"),
            message("m2", "Other <x@gmail.com>"),
            message("m3", "old@bank.com", covered = true),
        ),
        email = null, createdAt = 0L, service = "gmail", account = null, waitUntil = null, grant = null, count = 3u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "", resources = emptyList(), preview = emptyList(), noStanding = false,
        opTitle = "", action = "search", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    private val sendView = ApprovalView(
        requestId = "r2", connectionId = "c1", connectionLabel = "ChatGPT", kind = ApprovalKind.SEND, query = null,
        messages = emptyList(),
        email = EmailView(listOf("Ann <ann@corp.com>"), listOf("bob@corp.com"), "Hi", "body"),
        createdAt = 0L, service = "gmail", account = null, waitUntil = null, grant = null, count = 2u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "", resources = emptyList(), preview = emptyList(), noStanding = false,
        opTitle = "", action = "search", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    @Test
    fun `showing accounts is once or for a period, never open ended`() {
        val view = dev.reins.android.TestData.accountsView()
        val all = view.accounts.toSet()
        val month = ok(buildChoice(view, ApprovalDraft(selected = all, lifetime = LifetimeKind.MONTH)))
        assertEquals(2_592_000uL, month.standing!!.durationSecs)
        assertEquals(view.accounts, month.selectedMessageIds)
        assertNull(ok(buildChoice(view, ApprovalDraft(selected = all, lifetime = LifetimeKind.ONCE))).standing)
        assertEquals(3_600uL, ok(buildChoice(view, ApprovalDraft(selected = all, lifetime = LifetimeKind.HOUR))).standing!!.durationSecs)
        assertTrue(buildChoice(view, ApprovalDraft(selected = all, lifetime = LifetimeKind.UNTIL_REVOKED)) is BuildResult.Invalid)
        assertTrue(buildChoice(view, ApprovalDraft(selected = all, lifetime = LifetimeKind.USES)) is BuildResult.Invalid)
        assertTrue("nothing ticked", buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.MONTH)) is BuildResult.Invalid)
        assertEquals(listOf("me@gmail.com"), ok(buildChoice(view, ApprovalDraft(selected = setOf("me@gmail.com", "stranger@x.com"), lifetime = LifetimeKind.MONTH))).selectedMessageIds)
        val more = dev.reins.android.TestData.accountsView(shared = listOf("me@gmail.com"))
        assertEquals("already shared ones are not picks", listOf("work@corp.example"), ok(buildChoice(more, ApprovalDraft(selected = all, lifetime = LifetimeKind.MONTH))).selectedMessageIds)
    }

    private val grantView = dev.reins.android.TestData.grantView(duration = 3600u)

    @Test
    fun `a permission request is allowed as asked or made shorter, never longer`() {
        val asIs = ok(buildChoice(grantView, ApprovalDraft()))
        assertNull(asIs.standing)
        assertTrue(asIs.selectedMessageIds.isEmpty())
        val shorter = ok(buildChoice(grantView, ApprovalDraft(grantSeconds = 600))).standing!!
        assertEquals(600UL, shorter.durationSecs)
        assertNull("asking for longer than requested is ignored", ok(buildChoice(grantView, ApprovalDraft(grantSeconds = 7200))).standing)
    }

    private fun ok(result: BuildResult) = (result as BuildResult.Ok).choice

    @Test
    fun `addresses are pulled out of display names and lower-cased`() {
        assertEquals("a@b.com", addressOf("Name <A@B.com>"))
        assertEquals("a@b.com", addressOf("a@b.com"))
        assertNull(addressOf("(unknown sender)"))
        assertNull(addressOf("Name <not an address>"))
        assertNull(addressOf("a@b@c.com"))
    }

    @Test
    fun `once creates no standing grant and always includes covered messages`() {
        val choice = ok(buildChoice(readView, ApprovalDraft(selected = setOf("m1"))))
        assertEquals(listOf("m1", "m3"), choice.selectedMessageIds)
        assertNull(choice.standing)
    }

    @Test
    fun `reads need at least one message`() {
        val plain = readView.copy(messages = listOf(message("m1", "a@b.com")))
        assertTrue(buildChoice(plain, ApprovalDraft()) is BuildResult.Invalid)
    }

    @Test
    fun `lifetimes map to seconds or uses`() {
        fun standing(kind: LifetimeKind, uses: Int = 5) =
            ok(buildChoice(readView, ApprovalDraft(selected = setOf("m1"), lifetime = kind, uses = uses))).standing!!
        assertEquals(3_600UL, standing(LifetimeKind.HOUR).durationSecs)
        assertEquals(86_400UL, standing(LifetimeKind.DAY).durationSecs)
        assertEquals(604_800UL, standing(LifetimeKind.WEEK).durationSecs)
        assertNull(standing(LifetimeKind.UNTIL_REVOKED).durationSecs)
        assertNull(standing(LifetimeKind.UNTIL_REVOKED).maxUses)
        val uses = standing(LifetimeKind.USES, 7)
        assertEquals(7u, uses.maxUses)
        assertNull(uses.durationSecs)
    }

    @Test
    fun `uses are bounded`() {
        val draft = ApprovalDraft(selected = setOf("m1"), lifetime = LifetimeKind.USES)
        assertTrue(buildChoice(readView, draft.copy(uses = 0)) is BuildResult.Invalid)
        assertTrue(buildChoice(readView, draft.copy(uses = 1001)) is BuildResult.Invalid)
        assertTrue(buildChoice(readView, draft.copy(uses = 1000)) is BuildResult.Ok)
    }

    @Test
    fun `a standing read grant defaults to the selected messages only`() {
        val scope = ok(buildChoice(readView, ApprovalDraft(setOf("m1"), LifetimeKind.DAY))).standing!!.scope
        assertTrue(scope.selectedMessagesOnly)
        assertTrue(scope.senderAddresses.isEmpty() && scope.senderDomains.isEmpty() && scope.subjectPattern == null)
    }

    @Test
    fun `similar mail needs a rule and carries it`() {
        val base = ApprovalDraft(setOf("m1"), LifetimeKind.WEEK, similar = true)
        assertTrue(buildChoice(readView, base) is BuildResult.Invalid)
        val scope = ok(
            buildChoice(readView, base.copy(senderDomains = setOf("bank.com"), subject = "  statement ")),
        ).standing!!.scope
        assertEquals(false, scope.selectedMessagesOnly)
        assertEquals(listOf("bank.com"), scope.senderDomains)
        assertEquals("statement", scope.subjectPattern)
    }

    @Test
    fun `a standing send grant allows every recipient exactly by default`() {
        val choice = ok(buildChoice(sendView, ApprovalDraft(lifetime = LifetimeKind.HOUR)))
        assertTrue(choice.selectedMessageIds.isEmpty())
        val scope = choice.standing!!.scope
        assertEquals(listOf("ann@corp.com", "bob@corp.com"), scope.recipientAddresses)
        assertTrue(scope.recipientDomains.isEmpty())
    }

    @Test
    fun `a recipient can be widened to its domain`() {
        val draft = ApprovalDraft(lifetime = LifetimeKind.HOUR, domainRecipients = setOf("ann@corp.com"))
        val scope = ok(buildChoice(sendView, draft)).standing!!.scope
        assertEquals(listOf("bob@corp.com"), scope.recipientAddresses)
        assertEquals(listOf("corp.com"), scope.recipientDomains)
    }

    @Test
    fun `allowing all mail is explicit, time-boxed and releases everything shown`() {
        val draft = ApprovalDraft(allMail = LifetimeKind.HOUR)
        val choice = ok(buildChoice(readView, draft.copy(selected = setOf("m1", "m2"))))
        assertEquals(listOf("m1", "m2", "m3"), choice.selectedMessageIds)
        val standing = choice.standing!!
        assertEquals(3_600UL, standing.durationSecs)
        assertNull(standing.maxUses)
        assertTrue(standing.scope.allMail)
        assertTrue(!standing.scope.selectedMessagesOnly && standing.scope.senderDomains.isEmpty())
        assertEquals(604_800UL, ok(buildChoice(readView, draft.copy(selected = setOf("m1"), allMail = LifetimeKind.WEEK))).standing!!.durationSecs)
    }

    @Test
    fun `all mail refuses open-ended lifetimes and sending`() {
        val picked = setOf("m1")
        for (kind in listOf(LifetimeKind.ONCE, LifetimeKind.UNTIL_REVOKED, LifetimeKind.USES)) {
            assertTrue(buildChoice(readView, ApprovalDraft(selected = picked, allMail = kind)) is BuildResult.Invalid)
        }
        assertTrue(buildChoice(sendView, ApprovalDraft(allMail = LifetimeKind.HOUR)) is BuildResult.Invalid)
    }

    @Test
    fun `all mail needs no ticks because everything shown is released`() {
        val choice = ok(buildChoice(readView, ApprovalDraft(allMail = LifetimeKind.DAY)))
        assertEquals(listOf("m1", "m2", "m3"), choice.selectedMessageIds)
    }

    @Test
    fun `public mail domains are flagged`() {
        assertTrue(PublicMailDomains.isPublic("Gmail.com"))
        assertTrue(PublicMailDomains.isPublic("@outlook.com"))
        assertTrue(!PublicMailDomains.isPublic("bank.com"))
        val choice = ok(
            buildChoice(
                readView,
                ApprovalDraft(setOf("m2"), LifetimeKind.DAY, similar = true, senderDomains = setOf("gmail.com", "bank.com")),
            ),
        )
        assertEquals(listOf("gmail.com"), publicDomainWarnings(choice))
        assertTrue(publicDomainWarnings(ok(buildChoice(readView, ApprovalDraft(setOf("m2"))))).isEmpty())
    }

    // ---- another integration ----------------------------------------------------------------------------------

    @Test
    fun `a fetch releases the ticked items and the ones a grant already covers`() {
        val view = dev.reins.android.TestData.fetchView()
        val once = ok(buildChoice(view, ApprovalDraft(selected = setOf("100:2"))))
        assertEquals(listOf("100:2"), once.selectedMessageIds)
        assertNull(once.standing)
        assertTrue(buildChoice(view, ApprovalDraft()) is BuildResult.Invalid)
    }

    @Test
    fun `a fetch can be remembered for the chats named, or for everything for a short time`() {
        val view = dev.reins.android.TestData.fetchView()
        val named = ok(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), lifetime = LifetimeKind.DAY, resources = setOf("100"))))
        assertEquals(listOf("100"), named.standing!!.scope.resources)
        assertEquals(86_400uL, named.standing!!.durationSecs)
        assertTrue(!named.standing!!.scope.allMail)
        assertTrue(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), lifetime = LifetimeKind.DAY, resources = setOf("nope"))) is BuildResult.Invalid)
        val everything = ok(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), allMail = LifetimeKind.WEEK)))
        assertTrue(everything.standing!!.scope.allMail)
        assertTrue(everything.standing!!.scope.resources.isEmpty())
        assertTrue(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), allMail = LifetimeKind.MONTH)) is BuildResult.Invalid)
        val uses = ok(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), lifetime = LifetimeKind.USES, uses = 3, resources = setOf("100"))))
        assertEquals(3u, uses.standing!!.maxUses)
        assertTrue(buildChoice(view, ApprovalDraft(selected = setOf("100:2"), lifetime = LifetimeKind.USES, uses = 0, resources = setOf("100"))) is BuildResult.Invalid)
    }

    @Test
    fun `a password is never remembered whatever the draft says`() {
        val view = dev.reins.android.TestData.vaultView()
        val choice = ok(buildChoice(view, ApprovalDraft(selected = setOf("git:password"), lifetime = LifetimeKind.WEEK, resources = setOf("git"))))
        assertNull(choice.standing)
        assertEquals(listOf("git:password"), choice.selectedMessageIds)
    }

    @Test
    fun `a change is approved as shown and can be remembered for its chat but not everywhere`() {
        val view = dev.reins.android.TestData.writeView()
        val once = ok(buildChoice(view, ApprovalDraft()))
        assertTrue(once.selectedMessageIds.isEmpty())
        assertNull(once.standing)
        val hour = ok(buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.HOUR, resources = setOf("100"))))
        assertEquals(listOf("100"), hour.standing!!.scope.resources)
        assertTrue(buildChoice(view, ApprovalDraft(allMail = LifetimeKind.HOUR)) is BuildResult.Invalid)
        assertTrue(buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.HOUR)) is BuildResult.Invalid)
    }

    // ---- kinds of change and wider permissions ----------------------------------------------------------------

    @Test
    fun `the kind of the request starts ticked and the last kind cannot be unticked`() {
        val view = dev.reins.android.TestData.repoWriteView()
        assertEquals(setOf("code"), defaultClasses(view))
        assertEquals(setOf("code"), toggleClass(setOf("code"), "code", false))
        assertEquals(setOf("code", "issues"), toggleClass(setOf("code"), "issues", true))
        assertEquals(setOf("issues"), toggleClass(setOf("code", "issues"), "code", false))
        assertTrue(defaultClasses(dev.reins.android.TestData.fetchView()).isEmpty())
    }

    @Test
    fun `the kinds ticked travel with the permission, in the order they are offered`() {
        val view = dev.reins.android.TestData.repoWriteView()
        val choice = ok(
            buildChoice(
                view,
                ApprovalDraft(lifetime = LifetimeKind.DAY, resources = setOf("octo/app@main"), classes = setOf("releases", "code")),
            ),
        )
        assertEquals(listOf("code", "releases"), choice.standing!!.scope.classes)
        assertEquals(listOf("octo/app@main"), choice.standing!!.scope.resources)
        // Without a permission nothing about kinds is sent.
        assertNull(ok(buildChoice(view, ApprovalDraft(classes = setOf("code")))).standing)
        // Reads have no kinds.
        val read = ok(buildChoice(dev.reins.android.TestData.repoReadView(), ApprovalDraft(selected = setOf("README.md"), lifetime = LifetimeKind.HOUR, resources = setOf("octo/app"), classes = setOf("code"))))
        assertTrue(read.standing!!.scope.classes.isEmpty())
    }

    @Test
    fun `the ids follow the rule that A covers A, A at x and A slash x`() {
        assertTrue(resourceCovers("octo/app", "octo/app"))
        assertTrue(resourceCovers("octo/app", "octo/app@main"))
        assertTrue(resourceCovers("octo", "octo/app"))
        assertTrue(resourceCovers("git", "git/login/7"))
        assertFalse(resourceCovers("octo/app", "octo/app2"))
        assertFalse(resourceCovers("octo/app@main", "octo/app"))
        assertFalse(resourceCovers("oct", "octo/app"))
    }

    @Test
    fun `only what the request touches starts ticked, the wider things are not`() {
        assertEquals(setOf("octo/app@main"), defaultResources(dev.reins.android.TestData.repoWriteView()))
        assertEquals(setOf("100"), defaultResources(dev.reins.android.TestData.fetchView()))
    }

    @Test
    fun `ticking a wider thing unticks the narrower ones and the other way round`() {
        assertEquals(setOf("octo/app"), toggleResource(setOf("octo/app@main"), "octo/app", true))
        assertEquals(setOf("octo"), toggleResource(setOf("octo/app@main", "octo/app"), "octo", true))
        assertEquals(setOf("octo/app@main"), toggleResource(setOf("octo/app"), "octo/app@main", true))
        assertEquals(setOf("octo/app@main", "octo/other"), toggleResource(setOf("octo/app@main"), "octo/other", true))
        assertEquals(emptySet<String>(), toggleResource(setOf("octo/app@main"), "octo/app@main", false))
    }

    @Test
    fun `a wider permission is sent alone even if a narrower one is still in the draft`() {
        val view = dev.reins.android.TestData.repoWriteView()
        val choice = ok(buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.HOUR, resources = setOf("octo/app@main", "octo/app"))))
        assertEquals(listOf("octo/app"), choice.standing!!.scope.resources)
        assertTrue(buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.HOUR, resources = setOf("elsewhere/repo"))) is BuildResult.Invalid)
    }

    @Test
    fun `a change that is asked for every time is never remembered`() {
        val view = dev.reins.android.TestData.onceOnlyWriteView()
        val choice = ok(buildChoice(view, ApprovalDraft(lifetime = LifetimeKind.WEEK, resources = setOf("octo"), classes = setOf("settings"))))
        assertNull(choice.standing)
        assertTrue(choice.selectedMessageIds.isEmpty())
    }
}
