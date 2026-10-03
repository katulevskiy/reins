package dev.reins.android.ui

import dev.reins.android.ui.services.GITHUB_TOKEN_URL
import dev.reins.android.ui.services.GITHUB_CLASSIC_TOKEN_URL
import dev.reins.android.ui.services.githubClassicTokenUrl
import dev.reins.android.ui.services.githubTokenUrl
import dev.reins.android.ui.services.looksLikeGithubToken
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class GithubTokenTest {
    @Test
    fun `only things that look like a github token are picked up from the clipboard`() {
        assertTrue(looksLikeGithubToken("github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz_0123456789"))
        assertTrue(looksLikeGithubToken("  ghp_abcdefghijklmnopqrstuvwxyz0123456789\n"))
        for (other in listOf(null, "", "hello", "https://github.com", "ghp_short", "my password is hunter2 and more words here", "xghp_abcdefghijklmnopqrstuvwxyz0123456789")) {
            assertFalse("$other", looksLikeGithubToken(other))
        }
    }

    @Test
    fun `the fine-grained page asks for what the tools need and nothing outside a repository`() {
        for (permission in listOf(
            "contents=write", "issues=write", "pull_requests=write", "actions=write", "workflows=write", "administration=write",
            "repository_hooks=write", "secrets=write", "variables=write", "environments=write", "checks=write", "statuses=write",
            "security_events=read", "vulnerability_alerts=read", "secret_scanning_alerts=read", "metadata=read",
        )) {
            assertTrue(permission, "&$permission" in GITHUB_TOKEN_URL)
        }
        assertFalse("gist" in GITHUB_TOKEN_URL || "notifications" in GITHUB_TOKEN_URL)
        assertTrue(GITHUB_TOKEN_URL.startsWith("https://github.com/settings/personal-access-tokens/new?name=Reins&"))
    }

    @Test
    fun `the classic page asks for every scope the tools use`() {
        assertEquals(
            "https://github.com/settings/tokens/new?description=Reins&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo",
            GITHUB_CLASSIC_TOKEN_URL,
        )
    }

    @Test
    fun `every token page gets its own name so a second token never clashes`() {
        assertTrue("name=Reins-123456&" in githubTokenUrl(123456))
        val names = (1..20).map { githubTokenUrl().substringAfter("name=").substringBefore("&") }.toSet()
        assertTrue(names.size > 1)
        assertTrue(names.all { Regex("Reins-\\d{6}").matches(it) })
    }

    @Test
    fun `the classic page gets its own name too`() {
        assertEquals(
            "https://github.com/settings/tokens/new?description=Reins-123456&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo",
            githubClassicTokenUrl(123456),
        )
        val names = (1..20).map { githubClassicTokenUrl().substringAfter("description=").substringBefore("&") }.toSet()
        assertTrue(names.size > 1)
        assertTrue(names.all { Regex("Reins-\\d{6}").matches(it) })
    }
}
