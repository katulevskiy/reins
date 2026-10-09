package dev.reins.android

import dev.reins.core.ActivityEntry
import dev.reins.core.ActivityInfo
import dev.reins.core.ActivityMessage
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.AskView
import dev.reins.core.AutoDecisionView
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotNote
import dev.reins.core.ClassView
import dev.reins.core.ModelState
import dev.reins.core.ModelStatus
import dev.reins.core.NeighbourView
import dev.reins.core.Preset
import dev.reins.core.ProfileView
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict
import dev.reins.core.BlobView
import dev.reins.core.ClassOption
import dev.reins.core.ConnectionView
import dev.reins.core.EmailView
import dev.reins.core.GitCommitView
import dev.reins.core.GitFileView
import dev.reins.core.GitPushView
import dev.reins.core.GitRefView
import dev.reins.core.GrantRequestView
import dev.reins.core.GrantView
import dev.reins.core.GrantScopeChoice
import dev.reins.core.StandingGrant
import dev.reins.core.McpCallView
import dev.reins.core.McpServerView
import dev.reins.core.McpToolView
import dev.reins.core.MessageView
import dev.reins.core.PairingView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.ResourceView
import dev.reins.core.SecretReleaseView
import dev.reins.core.SshSignView
import dev.reins.core.VaultField
import dev.reins.core.VaultItemDetail
import dev.reins.core.VaultItemKind
import dev.reins.core.VaultUse

/** Builders for the records the UI shows, with sensible defaults. */
object TestData {
    fun message(id: String, from: String, covered: Boolean = false, sensitive: Boolean = false) =
        MessageView(id, from, "Subject $id", 1_700_000_000, "snippet of $id", covered, sensitive)

    fun pending(
        id: String = "req1",
        action: String = "search",
        count: UInt = 2u,
        waitUntil: Long? = null,
        label: String = "Claude",
        conn: String = "c1",
        createdAt: Long = (waitUntil ?: 1_700_000_245) - 45,
        service: String = "gmail",
        account: String? = "me@gmail.com",
        op: String = "",
        opTitle: String = "",
        suggestion: String? = null,
        headline: String = "",
        quick: Boolean = false,
    ) = PendingItem(
        PendingKind.REQUEST, id, "$label wants to $action", "in:inbox", createdAt, conn, label, action, count,
        service, account, waitUntil, op, opTitle, suggestion, headline, quick,
    )

    fun pairingItem(id: String = "pair1") = PendingItem(
        PendingKind.PAIRING, id, "Connect Claude to Reins?", "claude.ai", 1_700_000_200, "", "Claude", "pair", 1u, "", null, null, "", "", null,
    )

    /** Another phone asking this one for the account's keys, as the core parks it. */
    fun joinItem(id: String = "join1", device: String = "Pixel 9") = PendingItem(
        PendingKind.JOIN, id, "Add $device to your account?", "Another phone asks for this account's keys", 1_700_000_200, "", device, "join", 1u, "",
        null, null, "", "", null,
    )

    fun joinView(id: String = "join1", device: String = "Pixel 9", code: String = "482 193") =
        dev.reins.core.JoinView(id, device, code, 1_700_000_200)

    /** A pairing; the Reins desktop app's carries the eight digits of its key. */
    fun pairingView(id: String = "pair1", name: String = "Claude", host: String = "claude.ai", keyFingerprint: String? = null) =
        PairingView(id, name, host, byteArrayOf(7, 42, 99), 1_700_000_200, keyFingerprint)

    fun searchView(waitUntil: Long? = null) = ApprovalView(
        requestId = "req1", connectionId = "c1", connectionLabel = "Claude", kind = ApprovalKind.SEARCH, query = "from:bank",
        messages = listOf(
            message("m1", "Bank <alerts@bank.com>"),
            message("m2", "Friend <pal@gmail.com>"),
            message("m3", "old@bank.com", covered = true),
        ),
        email = null, createdAt = (waitUntil ?: 1_700_000_245) - 45, service = "gmail", account = "me@gmail.com", waitUntil = waitUntil, grant = null, count = 3u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "", resources = emptyList(), preview = emptyList(), noStanding = false,
        opTitle = "", action = "search", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    /** What the core adds to a routine request: its headline and one-tap answers ("allow" for [allowSecs]). */
    fun quick(
        view: ApprovalView,
        headline: String = "Claude gets the 3 emails found for \"from:bank\".",
        repeats: UInt = 0u,
        allowSecs: ULong = if (repeats >= 2u) 28_800uL else 3_600uL,
        allowWhat: String = "searching and reading me@gmail.com",
    ) = view.copy(
        headline = headline,
        quick = dev.reins.core.QuickApproval(
            fromNotification = true,
            allow = StandingGrant(
                allowSecs, null,
                GrantScopeChoice(true, false, emptyList(), emptyList(), null, emptyList(), emptyList(), emptyList(), emptyList()),
            ),
            allowWhat = allowWhat,
            repeats = repeats,
        ),
    )

    /** An AI reads a Telegram chat: two ordinary messages and a login code from Telegram itself. */
    fun fetchView() = ApprovalView(
        requestId = "req7", connectionId = "c1", connectionLabel = "Claude", kind = ApprovalKind.FETCH, query = null,
        messages = listOf(
            MessageView("100:2", "Anna", "", 1_700_000_050, "Dinner at eight?", false, false),
            MessageView("100:1", "Bob", "", 1_700_000_000, "I am late", false, false),
            MessageView("100:9", "Telegram", "", 1_700_000_090, "Login code: 48151", false, true),
        ),
        email = null, createdAt = 1_700_000_055, service = "telegram", account = "+15550100", waitUntil = null, grant = null,
        count = 3u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "read", resources = listOf(ResourceView("100", "Family", false)), preview = emptyList(), noStanding = false,
        opTitle = "Read Telegram messages", action = "read", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    /** An AI asks for a password: the one item is sensitive, so nothing can be remembered. */
    fun vaultView() = ApprovalView(
        requestId = "req8", connectionId = "c1", connectionLabel = "Claude", kind = ApprovalKind.FETCH, query = null,
        messages = listOf(MessageView("git:password", "octo", "GitHub", 0, "Password for GitHub", false, true)),
        email = null, createdAt = 1_700_000_055, service = "vault", account = "me@example.com", waitUntil = null, grant = null,
        count = 1u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "get", resources = listOf(ResourceView("git", "GitHub", false)), preview = emptyList(), noStanding = true,
        opTitle = "Get a login from the vault", action = "read", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    /** An AI wants to send a Telegram message. */
    fun writeView() = ApprovalView(
        requestId = "req9", connectionId = "c1", connectionLabel = "Claude", kind = ApprovalKind.WRITE, query = null,
        messages = emptyList(),
        email = null, createdAt = 1_700_000_055, service = "telegram", account = "+15550100", waitUntil = null, grant = null,
        count = 1u, accounts = emptyList(), sharedAccounts = emptyList(),
        op = "send", resources = listOf(ResourceView("100", "Family", false)), preview = listOf("Send to Family", "Dinner at eight works"),
        noStanding = false, opTitle = "Send a Telegram message", action = "send", `class` = "", classes = emptyList(), git = null,
        blob = null, mcp = null, ask = null, secrets = null, ssh = null,
    )

    val repoClasses = listOf(
        ClassOption("issues", "Issues"), ClassOption("pulls", "Pull requests"), ClassOption("code", "Code"),
        ClassOption("releases", "Releases"),
    )

    /** An AI commits a file to a branch: the repository and its owner are offered as wider permissions. */
    fun repoWriteView() = writeView().copy(
        requestId = "req10", service = "github", account = "octo-cat", op = "file_put",
        resources = listOf(
            ResourceView("octo/app@main", "Branch main of octo/app", false),
            ResourceView("octo/app", "Any branch of octo/app", true),
            ResourceView("octo", "Every repository of octo", true),
        ),
        preview = listOf("Commit README.md to main of octo/app", "Fix the typo in the title"),
        opTitle = "Commit a file to GitHub", action = "write", `class` = "code", classes = repoClasses,
    )

    /** An AI deletes a repository: always asked for, never remembered. */
    fun onceOnlyWriteView() = repoWriteView().copy(
        requestId = "req11", op = "repo_delete",
        resources = listOf(ResourceView("octo/app", "octo/app", false), ResourceView("octo", "Every repository of octo", true)),
        preview = listOf("Delete the repository octo/app", "All code, issues and pull requests are lost"),
        noStanding = true, opTitle = "Delete a GitHub repository", `class` = "settings",
        classes = repoClasses + ClassOption("settings", "Settings"),
    )

    /** An AI reads a file of a repository branch. */
    fun repoReadView() = fetchView().copy(
        requestId = "req12", service = "github", account = "octo-cat", op = "contents_get",
        messages = listOf(MessageView("README.md", "octo/app", "README.md", 0, "Hello", false, false)),
        resources = listOf(
            ResourceView("octo/app@main", "Branch main of octo/app", false),
            ResourceView("octo/app", "Any branch of octo/app", true),
        ),
        opTitle = "Read a file from GitHub", action = "read",
    )

    fun gitCommit(n: Int) = GitCommitView("%07x".format(0xa1b2c00 + n), "Commit number $n", "Ada Lovelace <ada@example.com>")

    fun gitFile(
        path: String,
        status: String = "modified",
        additions: UInt? = 3u,
        deletions: UInt? = 1u,
        binary: Boolean = false,
    ) = GitFileView(path, status, additions, deletions, binary)

    /** One ref of a push; by default `main` gains 9 commits (7 listed) touching 12 files (10 listed). */
    fun gitRef(
        shortName: String = "main",
        kind: String = "branch",
        change: String = "update",
        force: Boolean = false,
        forceUnknown: Boolean = false,
        commitCount: UInt = 9u,
        commits: List<GitCommitView> = (1..7).map(::gitCommit),
        filesChanged: UInt = 12u,
        files: List<GitFileView> = (1..10).map { gitFile("src/module$it/File$it.kt") },
        additions: ULong? = 120u,
        deletions: ULong? = 14u,
    ) = GitRefView(
        if (kind == "tag") "refs/tags/$shortName" else "refs/heads/$shortName", kind, shortName, change, force, forceUnknown,
        commitCount, commits, filesChanged, files, additions, deletions,
    )

    fun gitPush(vararg refs: GitRefView, notes: List<String> = emptyList(), packBytes: ULong = 12_700u) =
        GitPushView("octo/app", packBytes, notes, refs.toList())

    /** The desktop app pushes with git: the core's preview lines are there too, but the git section replaces them. */
    fun gitPushView(git: GitPushView = gitPush(gitRef()), noStanding: Boolean = false, tags: Boolean = false) = repoWriteView().copy(
        requestId = "req20", connectionLabel = "Reins desktop app on laptop", op = if (tags) "git_tag_push" else "git_push",
        preview = listOf("Push 9 commits to main", "Commit number 1", "+120 −14 in 12 files"),
        noStanding = noStanding, opTitle = if (tags) "Push tags with git" else "Push with git",
        `class` = if (tags) "releases" else "code", git = git,
    )

    /** The desktop app asks to clone and fetch: an ordinary item list, no git section. */
    fun gitFetchView() = fetchView().copy(
        requestId = "req21", connectionLabel = "Reins desktop app on laptop", service = "github", account = "octo-cat",
        op = "git_fetch", count = 1u,
        messages = listOf(MessageView("octo/app", "octo/app (private)", "Clone and fetch octo/app", 0, "Git on your computer can read this repository for 1 hour.", false, false)),
        resources = listOf(ResourceView("octo/app", "octo/app (private)", false), ResourceView("octo", "Every repository of octo", true)),
        opTitle = "Clone and fetch with git", action = "read",
    )

    /** An AI wants to see the accounts of one integration. */
    fun accountsView(
        addresses: List<String> = listOf("me@gmail.com", "work@corp.example"),
        shared: List<String> = emptyList(),
    ) = ApprovalView(
        "req5", "c1", "Claude", ApprovalKind.ACCOUNTS, null, emptyList(), null, 1_700_000_200, "gmail", null, null, null,
        (addresses.size - shared.size).toUInt(), addresses, shared, "", emptyList(), emptyList(), false,
        "", "accounts", "", emptyList(), null, null, null, null, null, null,
    )

    fun sendView() = ApprovalView(
        "req2", "c1", "Claude", ApprovalKind.SEND, null, emptyList(),
        EmailView(listOf("Ann <ann@corp.com>"), listOf("bob@corp.com"), "Hi there", "Body text"),
        1_700_000_200, "gmail", "me@gmail.com", null, null, 2u, emptyList(), emptyList(), "", emptyList(), emptyList(), false,
        "", "send", "", emptyList(), null, null, null, null, null, null,
    )

    fun grantView(duration: ULong = 3600u, breadth: String = "narrow") = ApprovalView(
        "req3", "c1", "Claude", ApprovalKind.GRANT, null, emptyList(), null, 1_700_000_200, "gmail", "me@gmail.com", null,
        GrantRequestView(
            action = "read", summary = "read emails from alerts@bank.com for 1 hour", reason = "Summarise this week's statements",
            durationSecs = duration, maxUses = null, breadth = breadth, lines = listOf("From alerts@bank.com"),
        ),
        1u,
        emptyList(),
        emptyList(),
        "",
        emptyList(),
        emptyList(),
        false,
        "",
        "grant",
        "",
        emptyList(),
        null,
        null,
        null,
        null,
        null,
        null,
    )

    fun defaultScope(action: String, allMail: Boolean = false) = dev.reins.core.GrantScopeChoice(
        allMail = allMail,
        selectedMessagesOnly = false,
        senderAddresses = emptyList(),
        senderDomains = if (action == "send" || allMail) emptyList() else listOf("bank.com"),
        subjectPattern = null,
        recipientAddresses = emptyList(),
        recipientDomains = if (action == "send") listOf("corp.example") else emptyList(),
        resources = emptyList(),
        classes = emptyList(),
    )

    fun connection(id: String = "c1", label: String = "Claude", icon: String? = null, keyFingerprint: String? = null) =
        ConnectionView(id, label, "claude.ai", 1_700_000_000, 1_700_000_100, icon, keyFingerprint)

    /** A computer: the Reins desktop app, paired with its key. */
    fun computer(id: String = "d1", label: String = "Laptop", fingerprint: String = "4821 9930") =
        ConnectionView(id, label, "203.0.113.7", 1_700_000_000, 1_700_000_100, null, fingerprint)

    /**
     * A grant. Active ones were made [ageSeconds] ago and end in [leftSeconds]; ended ones expired long ago
     * unless [state] says they were used up or deleted.
     */
    fun grant(
        id: String = "g1",
        active: Boolean = true,
        uses: UInt = 3u,
        maxUses: UInt? = null,
        action: String = "read",
        state: String = if (active) "active" else "expired",
        leftSeconds: Long = 3_000,
        ageSeconds: Long = 600,
        allMail: Boolean = false,
        editable: dev.reins.core.GrantScopeChoice? = defaultScope(action, allMail),
    ): GrantView {
        val now = (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis()) / 1000
        return GrantView(
            id, "c1", "Claude", action, if (action == "send") "Send emails to @corp.example" else "Read emails from @bank.com",
            if (active) now + leftSeconds else now - 86_400 * 2, maxUses, uses, if (active) now - ageSeconds else now - 86_400 * 3, now - 500,
            "approval", "gmail", "me@gmail.com", listOf("From @bank.com"), active, state, allMail, editable,
        )
    }

    fun entry(
        id: Long,
        action: String = "search",
        outcome: String = "released",
        count: UInt = 2u,
        at: Long = 1_700_000_000 + id,
        grantId: String? = null,
        info: ActivityInfo = info(null, emptyList(), null, null, null),
        opTitle: String = "",
        decidedBy: String = "",
        autopilot: AutopilotNote? = null,
    ) = ActivityEntry(id, at, "c1", "Claude", action, outcome, "detail of $id", grantId, "gmail", "me@gmail.com", count, info, "", opTitle, decidedBy, autopilot)

    fun info(
        query: String?,
        messages: List<ActivityMessage>,
        email: EmailView?,
        note: String?,
        grantSummary: String?,
        accounts: List<String> = emptyList(),
    ) = ActivityInfo(query, messages, email, note, grantSummary, accounts)

    /** What was shared from another integration is kept as text with the entry. */
    fun sharedTexts(vararg texts: String) =
        texts.mapIndexed { i, t -> ActivityMessage("100:$i", t, "Anna", "", 1_700_000_000L + i) }

    fun messages(vararg subjects: String) =
        subjects.mapIndexed { i, s -> ActivityMessage("m$i", "", "Sender $i <s$i@x.com>", s, 1_700_000_000L + i) }

    // ---- MCP servers ----------------------------------------------------------------------------------------------

    /** A read-only search, a write, a destructive delete and a heavy export. */
    fun mcpTools() = listOf(
        McpToolView("search_issues", "Search issues", "Finds issues by text.", readOnly = true, destructive = false, heavy = false),
        McpToolView("create_issue", "Create issue", "Creates an issue in a team.", readOnly = false, destructive = false, heavy = false),
        McpToolView("delete_issue", "Delete issue", "Deletes an issue for good.", readOnly = false, destructive = true, heavy = false),
        McpToolView("export_project", "Export project", "Exports a whole project as a file.", readOnly = true, destructive = false, heavy = true),
    )

    fun mcpServer(
        id: String = "linear",
        name: String = "Linear",
        url: String = "https://mcp.linear.app/mcp",
        status: String = "ok",
        error: String? = null,
        tools: List<McpToolView> = mcpTools(),
    ) = McpServerView(id, name, url, status, error, tools)

    /** An AI calls a tool of an MCP server the user added. */
    fun mcpCallView(readOnly: Boolean = false, destructive: Boolean = false) = writeView().copy(
        requestId = "req30", service = "mcp:linear", account = null, op = "create_issue",
        resources = listOf(ResourceView("create_issue", "Create issue", false)),
        preview = listOf("Create issue on Linear"), noStanding = destructive, opTitle = "",
        action = if (readOnly) "read" else "write", `class` = "", classes = emptyList(),
        mcp = McpCallView(
            serverName = "Linear", serverUrl = "https://mcp.linear.app/mcp", tool = "create_issue", title = "Create issue",
            description = "Creates an issue in a team.",
            argumentsJson = "{\n  \"team\": \"ENG\",\n  \"title\": \"Login fails on Safari\",\n  \"priority\": 2\n}",
            readOnly = readOnly, destructive = destructive,
        ),
    )

    // ---- files --------------------------------------------------------------------------------------------------

    fun blob(
        id: String = "blob_0123456789abcdef",
        name: String = "report.csv",
        size: ULong = 18_432u,
        contentType: String = "text/csv",
        purpose: String = "The quarterly numbers for the summary",
        previewText: String? = "quarter,revenue,costs\nQ1,120,80\nQ2,135,90",
        previewImage: ByteArray? = null,
        label: String = "Claude",
    ) = BlobView(
        id, label, name, size, contentType, "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", purpose,
        previewText, previewImage, 1_700_000_050, 1_700_003_650,
    )

    /** The pending item of an upload, as the core lists it. */
    fun blobItem(view: BlobView = blob()) = PendingItem(
        PendingKind.BLOB, view.id, "${view.connectionLabel} wants to share a file: ${view.name}", "18.0 KB · ${view.purpose}",
        view.createdAt, "c1", view.connectionLabel, "upload", 1u, "files", null, view.expiresAt, "", "", null,
    )

    /** A commit that uses a file the AI uploaded. */
    fun fileWriteView(blob: BlobView = blob(purpose = "For github_file_put")) = repoWriteView().copy(
        requestId = "req31", preview = listOf("Commit report.csv to main of octo/app", "Add the quarterly numbers"), blob = blob,
    )

    /** A tiny PNG (a 2×2 red square), as the server would hand over a preview. */
    val tinyPng: ByteArray by lazy {
        val bitmap = android.graphics.Bitmap.createBitmap(2, 2, android.graphics.Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(android.graphics.Color.RED)
        java.io.ByteArrayOutputStream().also { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
    }

    // ---- the desktop app --------------------------------------------------------------------------------------

    fun askView(detail: String? = "git push --force origin main", topic: String? = "command:git push --force") = writeView().copy(
        requestId = "req40", connectionLabel = "Reins desktop app on laptop", service = "desktop", account = null, op = "ask",
        resources = listOf(ResourceView(topic ?: "ask", topic ?: "Questions without a topic", false)),
        preview = listOf("Force-push main?"), opTitle = "Ask you on your phone", action = "write", `class` = "", classes = emptyList(),
        ask = AskView("Force-push main?", detail, topic),
    )

    fun secretsView(lease: ULong = 1_800u) = writeView().copy(
        requestId = "req41", connectionLabel = "Reins desktop app on laptop", service = "vault", account = "me@example.com",
        op = "secret_release", resources = listOf(ResourceView("secrets", "These secrets", false)),
        preview = listOf("npm run deploy"), noStanding = false, opTitle = "Use secrets on the computer", action = "write",
        `class` = "secrets", classes = emptyList(),
        secrets = SecretReleaseView("npm run deploy", "Deploy the site", listOf("Netlify · password", "GitHub · totp"), lease),
    )

    fun sshView(host: String? = "build.example.com", hostKey: String? = "SHA256:Ql3mV1hd2bS9WgS0aVq2Jv0Rr2q8Zb0yRrj2uMm3n0E") = writeView().copy(
        requestId = "req42", connectionLabel = "Reins desktop app on laptop", service = "vault", account = "me@example.com",
        op = "ssh_sign", resources = listOf(ResourceView("SHA256:abc@build.example.com", "Deploy key on build.example.com", false)),
        preview = listOf("Sign in to build.example.com"), opTitle = "Sign in to a server with SSH", action = "write",
        `class` = "ssh", classes = emptyList(),
        ssh = SshSignView("Deploy key", "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8", host, hostKey),
    )

    // ---- Autopilot ------------------------------------------------------------------------------------------------

    fun modelStatus(
        state: ModelState = ModelState.NOT_INSTALLED,
        downloaded: ULong = 0u,
        size: ULong = 0u,
        error: String? = null,
        runtimeReady: Boolean = true,
    ) = ModelStatus(state, "laya-approvals-base-v1", "Laya approvals (base)", "1", size, downloaded, error, runtimeReady)

    fun classView(
        key: String = "github/write/push",
        label: String = "GitHub · write · push",
        decisions: UInt = 12u,
        approved: UInt = 11u,
        denied: UInt = 1u,
        accuracy: Float? = null,
        autoApprove: Boolean = false,
        autoDeny: Boolean = false,
        manual: Boolean? = null,
        toUnlock: UInt = 8u,
    ) = ClassView(key, label, decisions, approved, denied, accuracy, autoApprove, autoDeny, manual, toUnlock)

    fun profile(
        id: String = "personal",
        name: String = "Personal",
        icon: String? = "🙂",
        preset: Preset = Preset.BALANCED,
        isDefault: Boolean = true,
        memory: UInt = 64u,
        connections: List<String> = emptyList(),
        classes: List<ClassView> = defaultClasses(),
        trainedAt: Long? = 1_700_000_000,
    ) = ProfileView(id, name, icon, preset, isDefault, memory, connections, classes, trainedAt)

    fun defaultClasses() = listOf(
        classView("github/write/push", "GitHub · write · push", 26u, 25u, 1u, 0.98f, autoApprove = true, toUnlock = 0u),
        classView("gmail/read", "Gmail · read", 14u, 12u, 2u, 0.93f, autoDeny = true, toUnlock = 6u),
        classView("desktop/ask/command", "Desktop · ask · command", 5u, 5u, 0u, null, toUnlock = 15u),
        classView("telegram/send", "Telegram · send", 22u, 20u, 2u, 0.97f, autoApprove = false, manual = false, toUnlock = 0u),
    )

    fun profiles() = listOf(
        profile(),
        profile("work", "Work", "💼", Preset.CAUTIOUS, isDefault = false, memory = 9u, connections = listOf("c2"), classes = listOf(classView(decisions = 9u, approved = 9u, denied = 0u, toUnlock = 11u)), trainedAt = null),
    )

    fun neighbours() = listOf(
        NeighbourView("Push to a branch · dkat/reins", Verdict.APPROVE, 0.97f, 1_700_000_000),
        NeighbourView("Push to a branch · dkat/laya", Verdict.APPROVE, 0.91f, 1_699_990_000),
        NeighbourView("Force push · dkat/reins", Verdict.DENY, 0.74f, 1_699_900_000),
    )

    fun suggestion(
        id: String = "req1",
        verdict: Verdict = Verdict.APPROVE,
        pApprove: Float = 0.97f,
        pDeny: Float = 0.02f,
        confidence: Float = 0.91f,
        floor: Boolean = false,
        novel: Boolean = false,
        judged: Boolean = true,
        mode: AutopilotMode = AutopilotMode.ASSISTED,
        reason: String = "Like 4 times you approved: Push to a branch · dkat/reins",
        neighbours: List<NeighbourView> = neighbours(),
    ) = SuggestionView(id, verdict, mode, pApprove, pDeny, confidence, reason, neighbours, "personal", "Personal", "github/write/push", novel, floor, judged)

    fun note(
        mode: AutopilotMode = AutopilotMode.AUTO,
        suggested: Verdict = Verdict.APPROVE,
        pApprove: Float = 0.98f,
        correctable: Boolean = true,
    ) = AutopilotNote(
        mode, suggested, pApprove, 1f - pApprove, 0.93f, "personal", "Personal",
        listOf("approved: Push to a branch · dkat/reins", "approved: Push to a branch · dkat/laya"),
        "Like 4 times you approved: Push to a branch · dkat/reins", correctable,
    )

    fun autoDecision(
        id: String = "req1",
        verdict: Verdict = Verdict.APPROVE,
        decidedBy: String = "autopilot",
        activityId: Long? = 42,
    ) = AutoDecisionView(id, PendingKind.REQUEST, "c1", "Claude Code", "Push to a branch · dkat/reins", verdict, decidedBy, 0.97f, 0.91f, activityId)

    // ---- the vault ----

    fun vaultItems() = listOf(
        VaultItemDetail(
            "openai", "OpenAI", VaultItemKind.LOGIN,
            listOf(
                VaultField("password", "Password", null, true, false),
                VaultField("uris", "Website", "https://platform.openai.com", false, false),
            ),
            listOf(VaultUse("vault:OpenAI/password", "reins run --env, and secret = in an [[api]] block")),
            null, true,
        ),
        VaultItemDetail(
            "github", "GitHub", VaultItemKind.LOGIN,
            listOf(
                VaultField("username", "Username", "octo", false, false),
                VaultField("password", "Password", null, true, false),
            ),
            listOf(
                VaultUse("vault:GitHub/password", "reins run --env, and secret = in an [[api]] block"),
                VaultUse("vault:GitHub/username", "reins run --env, and secret = in an [[api]] block"),
            ),
            null, false,
        ),
        VaultItemDetail(
            "deploy", "Deploy key", VaultItemKind.SSH_KEY,
            listOf(
                VaultField("private_key", "Private key", null, true, true),
                VaultField("public_key", "Public key", "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOk8f2xQ Deploy key", false, true),
                VaultField("fingerprint", "Fingerprint", "SHA256:59UqiUdGtW4OqO6NNXeMEo3goCDvszgSgq1R0tab5n0", false, false),
            ),
            listOf(VaultUse("SHA256:59UqiUdGtW4OqO6NNXeMEo3goCDvszgSgq1R0tab5n0", "Offered by the SSH agent of the desktop app (reins ssh setup); the private key stays on the phone")),
            null, false,
        ),
        VaultItemDetail(
            "visa", "Visa", VaultItemKind.CARD,
            listOf(
                VaultField("holder", "Holder", "Anna Smith", false, false),
                VaultField("number", "Number", null, true, false),
                VaultField("exp_month", "Expiry month", "4", false, false),
                VaultField("exp_year", "Expiry year", "2030", false, false),
            ),
            emptyList(), null, false,
        ),
    )

    fun vaultSecrets() = mapOf(
        "openai/password" to "sk-test-0000",
        "github/password" to "hunter2",
        "visa/number" to "4111111111111111",
    )
}
