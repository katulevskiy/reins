import Foundation
import UIKit

/// The `-demo` core's sample data: builders with sensible defaults (Android's `TestData`) and the seeded world the
/// screens open on (what Android's `ScreenshotsTest` sets up), with every time relative to launch so countdowns run.
enum DemoData {
    static let server = "https://reins.example.com"
    static let email = "me@example.com"
    static let desktop = "Reins desktop app on laptop"
    /// The demo account's recovery code (`AccountSecret`'s format: thirteen groups of four, base32).
    static let recoveryCode = "TKRQ-7HXM-2PLA-W4ZD-QE6N-B3VY-JF5C-K8SU-RM2G-XT7H-NAPQ-D6WL-ZE4B"

    /// Another phone, "Pixel 9", asking this one for the account's keys (`-demoJoin`).
    static func join(_ now: Int64) -> JoinView {
        JoinView(id: "join-pixel", deviceName: "Pixel 9", code: "482 193", createdAt: now - 4)
    }

    static func joinItem(_ j: JoinView) -> PendingItem {
        PendingItem(
            kind: .join, id: j.id, title: "Add \(j.deviceName) to your account?", subtitle: "Another phone asks for this account's keys",
            createdAt: j.createdAt, connectionId: "", connectionLabel: j.deviceName, action: "join", count: 1, service: "",
            account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil
        )
    }

    // ---- connections ------------------------------------------------------------------------------------------

    static func connections(_ now: Int64) -> [ConnectionView] {
        [
            ConnectionView(id: "c1", label: "Claude", clientHost: "claude.ai", createdAt: now - 86_400 * 40, lastUsedAt: now - 60, icon: "claude", keyFingerprint: nil),
            ConnectionView(id: "c2", label: "My ChatGPT", clientHost: "chatgpt.com", createdAt: now - 86_400 * 21, lastUsedAt: now - 120, icon: "openai", keyFingerprint: nil),
            ConnectionView(id: "c3", label: "Hermes agent", clientHost: "hermes.local", createdAt: now - 86_400 * 9, lastUsedAt: now - 7_200, icon: nil, keyFingerprint: nil),
            ConnectionView(id: "c4", label: "notes-bot", clientHost: "notes.example.com", createdAt: now - 86_400 * 5, lastUsedAt: now - 3_600, icon: nil, keyFingerprint: nil),
            ConnectionView(id: "c5", label: "Cursor", clientHost: "cursor.com", createdAt: now - 86_400 * 3, lastUsedAt: now - 1_800, icon: "cursor", keyFingerprint: nil),
            ConnectionView(id: "c6", label: desktop, clientHost: "laptop", createdAt: now - 86_400 * 2, lastUsedAt: now - 300, icon: nil, keyFingerprint: "4821 9930"),
        ]
    }

    // ---- pending items ----------------------------------------------------------------------------------------

    static func item(
        _ view: ApprovalView, title: String, subtitle: String, suggestion: String? = nil
    ) -> PendingItem {
        PendingItem(
            kind: .request, id: view.requestId, title: title, subtitle: subtitle, createdAt: view.createdAt,
            connectionId: view.connectionId, connectionLabel: view.connectionLabel, action: view.action, count: view.count,
            service: view.service, account: view.account, waitUntil: view.waitUntil, op: view.op, opTitle: view.opTitle,
            suggestion: suggestion, headline: view.headline, quick: view.quick?.fromNotification ?? false
        )
    }

    /// The headline and the one-tap answers, as the core's `quick::decorate` words them for these samples: nothing for
    /// what is asked every time, Approve from the notification unless something looks like a code, and "allow for a
    /// while" for the same target (`repeats` earlier identical approvals offer 8 hours instead of 1).
    static func decorated(_ v: ApprovalView, repeats: UInt32 = 0) -> ApprovalView {
        var v = v
        let shared = v.messages.filter { !$0.sensitive }.count
        let held = v.messages.count - shared == 0 ? "" : " One that looks like a code or a password stays private unless you tick it."
        let narrow = v.resources.filter { !$0.wider }.map(\.label)
        let emails = shared == 1 ? "1 email" : "\(shared) emails"
        v.headline = switch v.kind {
        case .search: "\(v.connectionLabel) gets the \(emails) found for \"\(v.query ?? "")\".\(held)"
        case .read: "\(v.connectionLabel) gets the full text of \(emails).\(held)"
        case .send: "An email to \(v.email?.to.first ?? "") goes out from \(v.account ?? "your Gmail")."
        case .grant: "\(v.connectionLabel) may \(v.grant?.summary ?? "") without asking you."
        case .accounts: "\(v.connectionLabel) sees the Gmail addresses you tick, nothing in them."
        case .fetch: "\(v.connectionLabel) gets \(shared == 1 ? "1 item" : "\(shared) items") from \(narrow.joined(separator: " and ")).\(held)"
        case .write:
            if let m = v.mcp { "\(v.connectionLabel) runs \(m.title) on \(m.serverName)." }
            else if let a = v.ask { "The desktop app gets a yes to: \(a.question)" }
            else { v.preview.first.map { $0.hasSuffix(".") ? $0 : $0 + "." } ?? "" }
        }
        let floor = v.kind == .grant || v.kind == .accounts || v.secrets != nil || v.ssh != nil || v.noStanding
            || v.mcp?.destructive == true || v.git?.refs.contains { $0.force || $0.forceUnknown || $0.change == "delete" } == true
        guard !floor else { return v }
        let allow: (GrantScopeChoice, String)? = switch v.kind {
        case .search, .read: (scope(allMail: true), "searching and reading \(v.account ?? "your Gmail")")
        case .send: v.email.map { e in (scope(recipients: e.to + e.cc), "emails to \((e.to + e.cc).joined(separator: " and "))") }
        case .fetch, .write:
            narrow.isEmpty ? nil : (scope(resources: v.resources.filter { !$0.wider }.map(\.id)), v.kind == .fetch
                ? "reading \(narrow.joined(separator: " and "))"
                : v.mcp.map { "\($0.title) on \($0.serverName)" } ?? "\(v.opTitle.prefix(1).lowercased() + v.opTitle.dropFirst()): \(narrow.joined(separator: " and "))")
        case .grant, .accounts: nil
        }
        v.quick = QuickApproval(
            fromNotification: !v.messages.contains(where: \.sensitive),
            allow: allow.map { StandingGrant(durationSecs: repeats >= 2 ? 28_800 : 3_600, maxUses: nil, scope: $0.0) },
            allowWhat: allow?.1 ?? "",
            repeats: repeats
        )
        return v
    }

    private static func scope(allMail: Bool = false, recipients: [String] = [], resources: [String] = []) -> GrantScopeChoice {
        GrantScopeChoice(
            allMail: allMail, selectedMessagesOnly: false, senderAddresses: [], senderDomains: [], subjectPattern: nil,
            recipientAddresses: recipients, recipientDomains: [], resources: resources, classes: []
        )
    }

    static func pairingItem(_ p: PairingView) -> PendingItem {
        PendingItem(
            kind: .pairing, id: p.id, title: "Connect \(p.clientName) to Reins?", subtitle: p.clientHost, createdAt: p.createdAt,
            connectionId: "", connectionLabel: p.clientName, action: "pair", count: 1, service: "", account: nil, waitUntil: nil,
            op: "", opTitle: "", suggestion: nil
        )
    }

    static func blobItem(_ b: BlobView, connectionId: String) -> PendingItem {
        PendingItem(
            kind: .blob, id: b.id, title: "\(b.connectionLabel) wants to share a file: \(b.name)",
            subtitle: "\(ByteCountFormatter.string(fromByteCount: Int64(b.size), countStyle: .file)) · \(b.purpose)",
            createdAt: b.createdAt, connectionId: connectionId, connectionLabel: b.connectionLabel, action: "upload", count: 1,
            service: "files", account: nil, waitUntil: b.expiresAt, op: "", opTitle: "", suggestion: nil
        )
    }

    /// An approval view with everything empty; each sample fills in what it is about.
    static func view(
        _ id: String, conn: String, label: String, kind: ApprovalKind, action: String, service: String, account: String?,
        createdAt: Int64, waitUntil: Int64?, count: UInt32 = 1, query: String? = nil, messages: [MessageView] = [],
        email: EmailView? = nil, grant: GrantRequestView? = nil, accounts: [String] = [], sharedAccounts: [String] = [],
        op: String = "", resources: [ResourceView] = [], preview: [String] = [], noStanding: Bool = false,
        opTitle: String = "", klass: String = "", classes: [ClassOption] = [], git: GitPushView? = nil, blob: BlobView? = nil,
        mcp: McpCallView? = nil, ask: AskView? = nil, secrets: SecretReleaseView? = nil, ssh: SshSignView? = nil
    ) -> ApprovalView {
        ApprovalView(
            requestId: id, connectionId: conn, connectionLabel: label, kind: kind, query: query, messages: messages, email: email,
            createdAt: createdAt, service: service, account: account, waitUntil: waitUntil, grant: grant, count: count,
            accounts: accounts, sharedAccounts: sharedAccounts, op: op, resources: resources, preview: preview,
            noStanding: noStanding, opTitle: opTitle, action: action, class: klass, classes: classes, git: git, blob: blob,
            mcp: mcp, ask: ask, secrets: secrets, ssh: ssh
        )
    }

    static func message(_ id: String, _ from: String, _ subject: String, _ snippet: String, _ date: Int64, covered: Bool = false, sensitive: Bool = false) -> MessageView {
        MessageView(id: id, from: from, subject: subject, date: date, snippet: snippet, coveredByGrant: covered, sensitive: sensitive)
    }

    static let repoClasses = [
        ClassOption(id: "issues", label: "Issues"), ClassOption(id: "pulls", label: "Pull requests"),
        ClassOption(id: "code", label: "Code"), ClassOption(id: "releases", label: "Releases"),
    ]

    static func realisticCommits() -> [GitCommitView] {
        [
            GitCommitView(shortSha: "9f3c2a1", subject: "Log in with a passkey when the browser offers one", author: "Ada Lovelace <ada@example.com>"),
            GitCommitView(shortSha: "4be81d0", subject: "Remove the old login form", author: "Ada Lovelace <ada@example.com>"),
            GitCommitView(shortSha: "c07a9e3", subject: "Keep the session for 30 days", author: "Grace Hopper <grace@example.com>"),
            GitCommitView(shortSha: "17d0f5b", subject: "Document the login flow", author: "Ada Lovelace <ada@example.com>"),
            GitCommitView(shortSha: "e2a4c66", subject: "Bump dependencies", author: "dependabot[bot] <support@github.com>"),
            GitCommitView(shortSha: "88b13f2", subject: "Fix a typo", author: "Grace Hopper <grace@example.com>"),
        ]
    }

    static func realisticFiles() -> [GitFileView] {
        [
            GitFileView(path: "src/auth/session.rs", status: "modified", additions: 42, deletions: 7, binary: false),
            GitFileView(path: "src/auth/login.rs", status: "added", additions: 88, deletions: 0, binary: false),
            GitFileView(path: "crates/reins-core/src/connector/github/very/deeply/nested/module/path/git.rs", status: "modified", additions: 12, deletions: 3, binary: false),
            GitFileView(path: "assets/logo.png", status: "added", additions: nil, deletions: nil, binary: true),
            GitFileView(path: "src/old_login.rs", status: "deleted", additions: 0, deletions: 64, binary: false),
            GitFileView(path: "scripts/run.sh", status: "type_changed", additions: 0, deletions: 0, binary: false),
            GitFileView(path: "README.md", status: "modified", additions: 6, deletions: 2, binary: false),
            GitFileView(path: "Cargo.toml", status: "modified", additions: 1, deletions: 0, binary: false),
            GitFileView(path: "Cargo.lock", status: "modified", additions: 30, deletions: 12, binary: false),
            GitFileView(path: "docs/login.md", status: "added", additions: 40, deletions: 0, binary: false),
        ]
    }

    static func gitRef(
        _ shortName: String, kind: String = "branch", change: String = "update", force: Bool = false, forceUnknown: Bool = false,
        commitCount: UInt32, commits: [GitCommitView], filesChanged: UInt32, files: [GitFileView], additions: UInt64?, deletions: UInt64?
    ) -> GitRefView {
        GitRefView(
            name: kind == "tag" ? "refs/tags/\(shortName)" : "refs/heads/\(shortName)", kind: kind, shortName: shortName,
            change: change, force: force, forceUnknown: forceUnknown, commitCount: commitCount, commits: commits,
            filesChanged: filesChanged, files: files, additions: additions, deletions: deletions
        )
    }

    static let blobSha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"

    /// A small drawn picture, so the upload sheet has an image preview to show.
    static func previewImage() -> Data? {
        let size = CGSize(width: 240, height: 160)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        let image = UIGraphicsImageRenderer(size: size, format: format).image { ctx in
            let colors = [UIColor.systemIndigo.cgColor, UIColor.systemTeal.cgColor] as CFArray
            if let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1]) {
                ctx.cgContext.drawLinearGradient(gradient, start: .zero, end: CGPoint(x: size.width, y: size.height), options: [])
            }
            UIColor.white.withAlphaComponent(0.85).setFill()
            for (i, h) in [60.0, 95.0, 75.0, 120.0].enumerated() {
                UIBezierPath(roundedRect: CGRect(x: 30 + Double(i) * 48, y: 140 - h, width: 32, height: h), cornerRadius: 4).fill()
            }
        }
        return image.pngData()
    }

    /// Every pending item with what opening it shows. Ids follow Android's test data where there is one.
    static func pending(_ now: Int64) -> (items: [PendingItem], views: [String: ApprovalView], pairings: [String: PairingView], blobs: [String: BlobView]) {
        var items: [PendingItem] = []
        var views: [String: ApprovalView] = [:]
        func add(_ v: ApprovalView, _ title: String, _ subtitle: String, suggestion: String? = nil, repeats: UInt32 = 0) {
            let v = decorated(v, repeats: repeats)
            views[v.requestId] = v
            items.append(item(v, title: title, subtitle: subtitle, suggestion: suggestion))
        }

        // Gmail search, the first thing an AI usually asks; one message is already covered by a permission.
        add(view(
            "req1", conn: "c1", label: "Claude", kind: .search, action: "search", service: "gmail", account: "me@gmail.com",
            createdAt: now - 5, waitUntil: now + 45, count: 3, query: "from:bank newer_than:30d",
            messages: [
                message("m1", "Bank <alerts@bank.com>", "Your March statement is ready", "Your statement for the account ending 4821 is now available.", now - 86_400 * 2),
                message("m2", "Friend <pal@gmail.com>", "Re: the bank thing", "Did you ever hear back from them about the fee?", now - 86_400 * 4),
                message("m3", "old@bank.com", "Wire transfer receipt", "We received your transfer of $1,200.00.", now - 86_400 * 9, covered: true),
            ]
        ), "Claude wants to search your emails", "from:bank newer_than:30d", repeats: 2)

        // An email to two people.
        add(view(
            "req2", conn: "c2", label: "My ChatGPT", kind: .send, action: "send", service: "gmail", account: "me@gmail.com",
            createdAt: now - 30, waitUntil: now + 90, count: 2,
            email: EmailView(
                to: ["Ann Lee <ann@corp.example>"], cc: ["bob@corp.example"], subject: "Q3 report",
                body: "Hi Ann,\n\nAttached is the Q3 report as discussed. Revenue is up 12% on Q2, mostly from the EMEA launch.\n\nLet me know if you want to go through it on Thursday.\n\nBest,\nAlex"
            )
        ), "My ChatGPT wants to send an email to Ann Lee <ann@corp.example>", "Q3 report")

        // A permission request.
        add(view(
            "req3", conn: "c3", label: "Hermes agent", kind: .grant, action: "grant", service: "gmail", account: "me@gmail.com",
            createdAt: now - 45, waitUntil: now + 120,
            grant: GrantRequestView(
                action: "read", summary: "read emails from alerts@bank.com for 1 hour", reason: "Summarise this week's statements",
                durationSecs: 3_600, maxUses: nil, breadth: "narrow", lines: ["From alerts@bank.com"]
            )
        ), "Hermes agent asks for permission: read emails from alerts@bank.com for 1 hour", "Summarise this week's statements")

        // Reading emails, already too late: the AI stopped waiting, so the sheet shows the late banner.
        add(view(
            "req4", conn: "c4", label: "notes-bot", kind: .read, action: "read", service: "gmail", account: "work@corp.example",
            createdAt: now - 80, waitUntil: now - 20, count: 2,
            messages: [
                message("m4", "Dana Wu <dana@corp.example>", "Offsite agenda", "Day one starts at 9 with the roadmap review.", now - 86_400),
                message("m5", "IT <it@corp.example>", "Laptop refresh", "Your new laptop is ready for pick-up at the front desk.", now - 86_400 * 3),
            ]
        ), "notes-bot wants to read 2 emails", "2 messages found")

        // Which accounts there are.
        add(view(
            "req5", conn: "c1", label: "Claude", kind: .accounts, action: "accounts", service: "gmail", account: nil,
            createdAt: now - 50, waitUntil: now + 150, count: 2, accounts: ["me@gmail.com", "work@corp.example"]
        ), "Claude wants to see your Gmail accounts", "2 accounts")

        // Telegram: a chat read with a login code among the messages, and a message to send.
        add(view(
            "req7", conn: "c1", label: "Claude", kind: .fetch, action: "read", service: "telegram", account: "+15550100",
            createdAt: now - 60, waitUntil: now + 200, count: 3,
            messages: [
                message("100:2", "Anna", "", "Dinner at eight?", now - 3_000),
                message("100:1", "Bob", "", "I am late", now - 3_600),
                message("100:9", "Telegram", "", "Login code: 48151", now - 900, sensitive: true),
            ],
            op: "read", resources: [ResourceView(id: "100", label: "Family", wider: false)], opTitle: "Read Telegram messages"
        ), "Claude wants to read telegram messages", "Family")

        add(view(
            "req9", conn: "c2", label: "My ChatGPT", kind: .write, action: "send", service: "telegram", account: "+15550100",
            createdAt: now - 70, waitUntil: now + 240,
            op: "send", resources: [ResourceView(id: "100", label: "Family", wider: false)],
            preview: ["Send to Family", "Dinner at eight works"], opTitle: "Send a Telegram message"
        ), "My ChatGPT wants to send a telegram message", "Family")

        // A password from the vault: sensitive, so nothing can be remembered; Autopilot never decides these.
        add(view(
            "req8", conn: "c1", label: "Claude", kind: .fetch, action: "read", service: "vault", account: "me@example.com",
            createdAt: now - 90, waitUntil: now + 260,
            messages: [message("git:password", "octo", "GitHub", "Password for GitHub", 0, sensitive: true)],
            op: "get", resources: [ResourceView(id: "git", label: "GitHub", wider: false)], noStanding: true,
            opTitle: "Get a login from the vault"
        ), "Claude wants to get a login from the vault", "GitHub", suggestion: "Autopilot always asks you about passwords")

        // A GitHub commit, with the repository and its owner offered as wider permissions.
        add(view(
            "req10", conn: "c5", label: "Cursor", kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 100, waitUntil: now + 280,
            op: "file_put",
            resources: [
                ResourceView(id: "octo/app@main", label: "Branch main of octo/app", wider: false),
                ResourceView(id: "octo/app", label: "Any branch of octo/app", wider: true),
                ResourceView(id: "octo", label: "Every repository of octo", wider: true),
            ],
            preview: ["Commit README.md to main of octo/app", "Fix the typo in the title"], opTitle: "Commit a file to GitHub",
            klass: "code", classes: repoClasses
        ), "Cursor wants to commit a file to github", "Branch main of octo/app", suggestion: "Autopilot is not sure · 61%")

        // Deleting a repository: asked every time.
        add(view(
            "req11", conn: "c5", label: "Cursor", kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 110, waitUntil: now + 300,
            op: "repo_delete",
            resources: [ResourceView(id: "octo/app", label: "octo/app", wider: false), ResourceView(id: "octo", label: "Every repository of octo", wider: true)],
            preview: ["Delete the repository octo/app", "All code, issues and pull requests are lost"], noStanding: true,
            opTitle: "Delete a GitHub repository", klass: "settings", classes: repoClasses + [ClassOption(id: "settings", label: "Settings")]
        ), "Cursor wants to delete a github repository", "octo/app")

        // The desktop app pushes a feature branch (Autopilot would approve), then force-pushes main.
        let pushResources = [
            ResourceView(id: "octo/app@feature/passkeys", label: "Branch feature/passkeys of octo/app", wider: false),
            ResourceView(id: "octo/app", label: "Any branch of octo/app", wider: true),
            ResourceView(id: "octo", label: "Every repository of octo", wider: true),
        ]
        add(view(
            "req20", conn: "c6", label: desktop, kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 20, waitUntil: now + 600,
            op: "git_push", resources: pushResources,
            preview: ["Push 8 commits to feature/passkeys", "Log in with a passkey when the browser offers one", "+219 −88 in 11 files"],
            opTitle: "Push with git", klass: "code", classes: repoClasses,
            git: GitPushView(repo: "octo/app", packBytes: 48_213, notes: ["1 file was too large to count lines."], refs: [
                gitRef("feature/passkeys", commitCount: 8, commits: realisticCommits(), filesChanged: 11, files: realisticFiles(), additions: 219, deletions: 88),
            ])
        ), "\(desktop) wants to push with git", "octo/app · feature/passkeys", suggestion: "Autopilot would approve · 97%")

        add(view(
            "req21", conn: "c6", label: desktop, kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 25, waitUntil: now + 600,
            op: "git_push", resources: [ResourceView(id: "octo/app@main", label: "Branch main of octo/app", wider: false)],
            preview: ["Force-push 2 commits to main", "Log in with a passkey when the browser offers one", "+142 −10 in 3 files"],
            noStanding: true, opTitle: "Push with git", klass: "code", classes: repoClasses,
            git: GitPushView(repo: "octo/app", packBytes: 3_145_728, notes: [], refs: [
                gitRef("main", force: true, commitCount: 2, commits: Array(realisticCommits().prefix(2)), filesChanged: 3, files: Array(realisticFiles().prefix(3)), additions: 142, deletions: 10),
            ])
        ), "\(desktop) wants to push with git", "octo/app · main")

        // Tags, a branch whose history is unknown, and a deleted branch in one push.
        add(view(
            "req22", conn: "c6", label: desktop, kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 400, waitUntil: nil,
            op: "git_tag_push", resources: [ResourceView(id: "octo/app", label: "octo/app", wider: false)],
            preview: ["Push tag v1.2.0", "Update release", "Delete old-experiment"], noStanding: true, opTitle: "Push tags with git",
            klass: "releases", classes: repoClasses,
            git: GitPushView(repo: "octo/app", packBytes: 12_700, notes: [], refs: [
                gitRef("v1.2.0", kind: "tag", change: "create", commitCount: 0, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
                gitRef("release", force: true, forceUnknown: true, commitCount: 1, commits: Array(realisticCommits().prefix(1)), filesChanged: 2, files: Array(realisticFiles()[3..<5]), additions: nil, deletions: nil),
                gitRef("old-experiment", change: "delete", commitCount: 0, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
            ])
        ), "\(desktop) wants to push tags with git", "octo/app · v1.2.0")

        // A tool of an MCP server.
        add(view(
            "req30", conn: "c5", label: "Cursor", kind: .write, action: "write", service: "mcp:linear", account: nil,
            createdAt: now - 120, waitUntil: now + 320,
            op: "create_issue", resources: [ResourceView(id: "create_issue", label: "Create issue", wider: false)],
            preview: ["Create issue on Linear"],
            mcp: McpCallView(
                serverName: "Linear", serverUrl: "https://mcp.linear.app/mcp", tool: "create_issue", title: "Create issue",
                description: "Creates an issue in a team.",
                argumentsJson: "{\n  \"team\": \"ENG\",\n  \"title\": \"Login fails on Safari\",\n  \"priority\": 2\n}",
                readOnly: false, destructive: false
            )
        ), "Cursor wants to use create_issue (Linear)", "Linear")

        add(view(
            "req32", conn: "c5", label: "Cursor", kind: .write, action: "write", service: "mcp:linear", account: nil,
            createdAt: now - 500, waitUntil: nil,
            op: "delete_issue", resources: [ResourceView(id: "delete_issue", label: "Delete issue", wider: false)],
            preview: ["Delete issue on Linear"], noStanding: true,
            mcp: McpCallView(
                serverName: "Linear", serverUrl: "https://mcp.linear.app/mcp", tool: "delete_issue", title: "Delete issue",
                description: "Deletes an issue for good.", argumentsJson: "{\n  \"id\": \"ENG-1042\"\n}", readOnly: false, destructive: true
            )
        ), "Cursor wants to use delete_issue (Linear)", "Linear")

        // A commit that carries a file the AI uploaded.
        let attached = BlobView(
            id: "blob_attached01", connectionLabel: "Cursor", name: "report.csv", size: 18_432, contentType: "text/csv", sha256: blobSha,
            purpose: "For github_file_put", previewText: "quarter,revenue,costs\nQ1,120,80\nQ2,135,90", previewImage: nil,
            createdAt: now - 140, expiresAt: now + 3_460
        )
        add(view(
            "req31", conn: "c5", label: "Cursor", kind: .write, action: "write", service: "github", account: "octo-cat",
            createdAt: now - 130, waitUntil: now + 340,
            op: "file_put", resources: [ResourceView(id: "octo/app@main", label: "Branch main of octo/app", wider: false)],
            preview: ["Commit report.csv to main of octo/app", "Add the quarterly numbers"], opTitle: "Commit a file to GitHub",
            klass: "code", classes: repoClasses, blob: attached
        ), "Cursor wants to commit a file to github", "Branch main of octo/app")

        // The desktop app: a question, secrets for a command, an SSH sign-in.
        add(view(
            "req40", conn: "c6", label: desktop, kind: .write, action: "write", service: "desktop", account: nil,
            createdAt: now - 35, waitUntil: now + 180,
            op: "ask", resources: [ResourceView(id: "command:git push --force", label: "command:git push --force", wider: false)],
            preview: ["Force-push main?"], opTitle: "Ask you on your phone",
            ask: AskView(question: "Force-push main?", detail: "git push --force origin main", topic: "command:git push --force")
        ), "\(desktop) wants to ask you on your phone", "Force-push main?")

        add(view(
            "req41", conn: "c6", label: desktop, kind: .write, action: "write", service: "vault", account: "me@example.com",
            createdAt: now - 40, waitUntil: now + 200,
            op: "secret_release", resources: [ResourceView(id: "secrets", label: "These secrets", wider: false)],
            preview: ["npm run deploy"], opTitle: "Use secrets on the computer", klass: "secrets",
            secrets: SecretReleaseView(command: "npm run deploy", purpose: "Deploy the site", items: ["Netlify · password", "GitHub · totp"], leaseSecs: 1_800)
        ), "\(desktop) wants to use secrets on the computer", "npm run deploy")

        add(view(
            "req42", conn: "c6", label: desktop, kind: .write, action: "write", service: "vault", account: "me@example.com",
            createdAt: now - 42, waitUntil: now + 200,
            op: "ssh_sign", resources: [ResourceView(id: "SHA256:abc@build.example.com", label: "Deploy key on build.example.com", wider: false)],
            preview: ["Sign in to build.example.com"], opTitle: "Sign in to a server with SSH", klass: "ssh",
            ssh: SshSignView(keyName: "Deploy key", keyFingerprint: "SHA256:nThbg6kXUpJWGl7E1IGOCspRomTxdCARLviKw6E5SY8", host: "build.example.com", hostKey: "SHA256:Ql3mV1hd2bS9WgS0aVq2Jv0Rr2q8Zb0yRrj2uMm3n0E")
        ), "\(desktop) wants to sign in to a server with ssh", "Deploy key on build.example.com")

        // Pairings: an AI on the web (three codes to pick from), and the desktop app with its key's digits.
        let pair1 = PairingView(id: "pair1", clientName: "Gemini", clientHost: "gemini.google.com", choices: Data([7, 42, 88]), createdAt: now - 12, keyFingerprint: nil)
        let pair2 = PairingView(id: "pair2", clientName: "Reins desktop app on studio", clientHost: "studio", choices: Data([19, 63, 5]), createdAt: now - 600, keyFingerprint: "4821 9930")
        items.append(pairingItem(pair1))
        items.append(pairingItem(pair2))

        // Uploads: a CSV with a text preview and a picture.
        let csv = BlobView(
            id: "blob_q3numbers", connectionLabel: "Claude", name: "q3-numbers.csv", size: 18_432, contentType: "text/csv", sha256: blobSha,
            purpose: "The quarterly numbers for the summary",
            previewText: "quarter,region,revenue,costs\nQ3,EMEA,1204000,880000\nQ3,Americas,2310500,1502000\nQ3,APAC,980250,701000",
            previewImage: nil, createdAt: now - 55, expiresAt: now + 3_545
        )
        let png = BlobView(
            id: "blob_chart", connectionLabel: "My ChatGPT", name: "revenue-chart.png", size: 48_120, contentType: "image/png",
            sha256: "2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae", purpose: "A chart for the Q3 email",
            previewText: nil, previewImage: previewImage(), createdAt: now - 65, expiresAt: now + 3_535
        )
        items.append(blobItem(csv, connectionId: "c1"))
        items.append(blobItem(png, connectionId: "c2"))

        items.sort { $0.createdAt > $1.createdAt }
        return (items, views, ["pair1": pair1, "pair2": pair2], [csv.id: csv, png.id: png, attached.id: attached])
    }

    /// The request `-demoArrive` delivers a few seconds after launch.
    static func arrival(_ now: Int64) -> (PendingItem, ApprovalView) {
        let v = decorated(view(
            "req50", conn: "c1", label: "Claude", kind: .search, action: "search", service: "gmail", account: "me@gmail.com",
            createdAt: now, waitUntil: now + 60, count: 2, query: "invoice from:acme",
            messages: [
                message("m50", "Acme Billing <billing@acme.example>", "Invoice #2291", "Your invoice for September is attached.", now - 86_400 * 3),
                message("m51", "Acme Billing <billing@acme.example>", "Invoice #2207", "Your invoice for August is attached.", now - 86_400 * 33),
            ]
        ))
        return (item(v, title: "Claude wants to search your emails", subtitle: "invoice from:acme"), v)
    }

    // ---- emails -------------------------------------------------------------------------------------------------

    /// What opening an email returns, by message id. Ids not here read as "no longer in Gmail".
    static func emails(_ now: Int64) -> [String: EmailContent] {
        func mail(_ id: String, _ from: String, _ subject: String, _ date: Int64, _ body: String) -> (String, EmailContent) {
            (id, EmailContent(id: id, from: from, to: ["me@gmail.com"], cc: [], subject: subject, date: date, body: body))
        }
        return Dictionary(uniqueKeysWithValues: [
            mail("m1", "Bank <alerts@bank.com>", "Your March statement is ready", now - 86_400 * 2,
                 "Hello,\n\nYour statement for the account ending 4821 is now available in online banking.\n\nClosing balance: $3,412.08\n\nThis is an automated message."),
            mail("m2", "Friend <pal@gmail.com>", "Re: the bank thing", now - 86_400 * 4,
                 "Did you ever hear back from them about the fee? Mine got refunded after one call.\n\n> On Monday you wrote:\n> They charged me twice for the transfer."),
            mail("m3", "old@bank.com", "Wire transfer receipt", now - 86_400 * 9, "We received your transfer of $1,200.00. Reference: WX-88213."),
            mail("m4", "Dana Wu <dana@corp.example>", "Offsite agenda", now - 86_400,
                 "Day one starts at 9 with the roadmap review, then team breakouts.\nDay two is the hackathon.\n\nDana"),
            mail("m5", "IT <it@corp.example>", "Laptop refresh", now - 86_400 * 3, "Your new laptop is ready for pick-up at the front desk. Bring your old one."),
            mail("a1", "Bank <alerts@bank.com>", "Your March statement is ready", now - 86_400 * 2,
                 "Hello,\n\nYour statement for the account ending 4821 is now available in online banking."),
            mail("a2", "Bank <alerts@bank.com>", "Wire transfer receipt", now - 86_400 * 6, "We received your transfer of $1,200.00. Reference: WX-88213."),
            mail("a3", "Bank <alerts@bank.com>", "Security alert", now - 86_400 * 8,
                 "A new device signed in to your account. If this was you, there is nothing to do."),
            mail("g1m", "Bank <alerts@bank.com>", "Card payment", now - 86_400 * 2, "A payment of $42.10 to Corner Café was made with your card ending 4821."),
            mail("m50", "Acme Billing <billing@acme.example>", "Invoice #2291", now - 86_400 * 3, "Your invoice for September is attached. Amount due: $1,180.00."),
            mail("m51", "Acme Billing <billing@acme.example>", "Invoice #2207", now - 86_400 * 33, "Your invoice for August is attached. Amount due: $1,180.00."),
        ])
    }

    // ---- activity -----------------------------------------------------------------------------------------------

    static func info(query: String? = nil, messages: [ActivityMessage] = [], email: EmailView? = nil, note: String? = nil, grantSummary: String? = nil, accounts: [String] = []) -> ActivityInfo {
        ActivityInfo(query: query, messages: messages, email: email, note: note, grantSummary: grantSummary, accounts: accounts)
    }

    static func entry(
        _ id: Int64, at: Int64, conn: String = "c1", label: String = "Claude", action: String, outcome: String, detail: String,
        grantId: String? = nil, service: String = "gmail", account: String? = "me@gmail.com", count: UInt32 = 1,
        info: ActivityInfo = info(), op: String = "", opTitle: String = "", decidedBy: String = "", autopilot: AutopilotNote? = nil
    ) -> ActivityEntry {
        ActivityEntry(
            id: id, at: at, connectionId: conn, connectionLabel: label, action: action, outcome: outcome, detail: detail,
            grantId: grantId, service: service, account: account, count: count, info: info, op: op, opTitle: opTitle,
            decidedBy: decidedBy, autopilot: autopilot
        )
    }

    static func note(_ suggested: Verdict = .approve, pApprove: Float = 0.98, correctable: Bool = true, mode: AutopilotMode = .auto) -> AutopilotNote {
        AutopilotNote(
            mode: mode, suggested: suggested, pApprove: pApprove, pDeny: 1 - pApprove, confidence: 0.93, profileId: "personal",
            profileName: "Personal",
            neighbours: suggested == .approve
                ? ["approved: Push to a branch · dkat/reins", "approved: Push to a branch · dkat/laya"]
                : ["denied: Send an email · unknown recipient", "denied: Forward emails · outside address"],
            reason: suggested == .approve
                ? "Like 4 times you approved: Push to a branch · dkat/reins"
                : "Like 2 times you denied: Send an email · unknown recipient",
            correctable: correctable
        )
    }

    /// The history, newest first: every outcome, every way of deciding.
    static func activity(_ now: Int64) -> [ActivityEntry] {
        let statements = [
            ActivityMessage(id: "a1", text: "", from: "Bank <alerts@bank.com>", subject: "Your March statement is ready", date: now - 86_400 * 2),
            ActivityMessage(id: "a2", text: "", from: "Bank <alerts@bank.com>", subject: "Wire transfer receipt", date: now - 86_400 * 6),
            ActivityMessage(id: "a3", text: "", from: "Bank <alerts@bank.com>", subject: "Security alert", date: now - 86_400 * 8),
        ]
        let telegram = [
            ActivityMessage(id: "100:0", text: "Dinner at eight?", from: "Anna", subject: "", date: now - 7_000),
            ActivityMessage(id: "100:1", text: "Sure, I'll book", from: "Bob", subject: "", date: now - 6_900),
        ]
        return [
            entry(14, at: now - 60, conn: "c6", label: desktop, action: "write", outcome: "sent",
                  detail: "feature/laya → dkat/reins · 3 commits", service: "github", account: "dkat", op: "git_push",
                  opTitle: "Push to a branch", decidedBy: "autopilot", autopilot: note()),
            entry(13, at: now - 300, conn: "c4", label: "notes-bot", action: "send", outcome: "denied",
                  detail: "To backup-svc@protonmail.example", count: 1,
                  info: info(email: EmailView(to: ["backup-svc@protonmail.example"], cc: [], subject: "Export", body: "All notes attached."), note: "Autopilot denied this"),
                  decidedBy: "autopilot", autopilot: note(.deny, pApprove: 0.04)),
            entry(12, at: now - 600, conn: "c2", label: "My ChatGPT", action: "read", outcome: "released", detail: "2 emails",
                  count: 2, info: info(messages: Array(statements.prefix(2))), decidedBy: "bypass"),
            entry(11, at: now - 700, conn: "c5", label: "Cursor", action: "write", outcome: "denied", detail: "Create issue on Linear",
                  service: "mcp:linear", account: nil, op: "create_issue", decidedBy: "lockdown"),
            entry(10, at: now - 800, conn: "c2", label: "My ChatGPT", action: "read", outcome: "released", detail: "3 emails",
                  count: 3, info: info(messages: statements)),
            entry(9, at: now - 900, action: "send", outcome: "sent", detail: "To ann@corp.example", count: 2,
                  info: info(email: EmailView(to: ["ann@corp.example"], cc: ["bob@corp.example"], subject: "Q3 report", body: "Hi Ann,\n\nAttached is the Q3 report as discussed.\n\nBest"))),
            entry(8, at: now - 1_800, conn: "c1", label: "Claude", action: "read", outcome: "released", detail: "Family", service: "telegram",
                  account: "+15550100", count: 2, info: info(messages: telegram), op: "read", opTitle: "Read Telegram messages"),
            entry(7, at: now - 3_600, conn: "c4", label: "notes-bot", action: "search", outcome: "denied", detail: "in:inbox newer_than:7d",
                  count: 5, info: info(query: "in:inbox newer_than:7d")),
            entry(6, at: now - 5_400, conn: "c1", label: "Claude", action: "accounts", outcome: "released", detail: "2 accounts",
                  service: "gmail", account: nil, count: 2, info: info(accounts: ["me@gmail.com", "work@corp.example"])),
            entry(5, at: now - 7_200, conn: "c3", label: "Hermes agent", action: "grant", outcome: "granted",
                  detail: "read emails from @bank.com for 1 hour", grantId: "g1",
                  info: info(note: "Summarise this week's statements", grantSummary: "Read emails from @bank.com for 1 hour")),
            entry(4, at: now - 86_400 * 2, action: "read", outcome: "released", detail: "Card payment", grantId: "g1",
                  info: info(messages: [ActivityMessage(id: "g1m", text: "", from: "Bank <alerts@bank.com>", subject: "Card payment", date: now - 86_400 * 2)])),
            entry(3, at: now - 86_400 * 2 - 600, conn: "c1", label: "Claude", action: "upload", outcome: "released",
                  detail: "q2-numbers.csv · 12 KB", service: "files", account: nil),
            entry(2, at: now - 86_400 * 3, action: "search", outcome: "error", detail: "Gmail did not answer in time", count: 0,
                  info: info(query: "from:landlord", note: "Gmail did not answer in time. Nothing was shared.")),
            entry(1, at: now - 86_400 * 4, conn: "c5", label: "Cursor", action: "write", outcome: "sent",
                  detail: "Commit README.md to main of octo/app", service: "github", account: "octo-cat", op: "file_put",
                  opTitle: "Commit a file to GitHub"),
        ]
    }

    // ---- grants -------------------------------------------------------------------------------------------------

    static func scope(senderDomains: [String] = [], recipientDomains: [String] = [], resources: [String] = [], classes: [String] = [], allMail: Bool = false) -> GrantScopeChoice {
        GrantScopeChoice(
            allMail: allMail, selectedMessagesOnly: false, senderAddresses: [], senderDomains: senderDomains, subjectPattern: nil,
            recipientAddresses: [], recipientDomains: recipientDomains, resources: resources, classes: classes
        )
    }

    static func grants(_ now: Int64) -> [GrantView] {
        func g(
            _ id: String, conn: String = "c1", label: String = "Claude", action: String = "read", summary: String = "Read emails from @bank.com",
            expiresAt: Int64?, maxUses: UInt32? = nil, uses: UInt32, createdAt: Int64, service: String = "gmail", account: String? = "me@gmail.com",
            lines: [String] = ["From @bank.com"], active: Bool = true, state: String = "active", allMail: Bool = false,
            origin: String = "approval", editable: GrantScopeChoice? = scope(senderDomains: ["bank.com"])
        ) -> GrantView {
            GrantView(
                id: id, connectionId: conn, connectionLabel: label, action: action, summary: summary, expiresAt: expiresAt, maxUses: maxUses,
                uses: uses, createdAt: createdAt, lastUsedAt: uses > 0 ? now - 500 : nil, origin: origin, service: service, account: account,
                lines: lines, active: active, state: state, allMail: allMail, editableScope: editable
            )
        }
        return [
            g("g1", conn: "c3", label: "Hermes agent", expiresAt: now + 3_000, uses: 12, createdAt: now - 600),
            g("g2", conn: "c2", label: "My ChatGPT", action: "send", summary: "Send emails to @corp.example", expiresAt: now + 5 * 86_400 + 600,
              maxUses: 1, uses: 0, createdAt: now - 86_400, lines: ["To @corp.example"], editable: scope(recipientDomains: ["corp.example"])),
            g("g4", expiresAt: now + 240, maxUses: 5, uses: 3, createdAt: now - 3_360),
            g("g5", conn: "c5", label: "Cursor", action: "write", summary: "Change issues and code of octo/app", expiresAt: now + 10 * 7 * 86_400,
              maxUses: 40, uses: 9, createdAt: now - 3_600, service: "github", account: "octo-cat",
              lines: ["Any branch of octo/app", "Issues, Code"], origin: "manual", editable: scope(resources: ["octo/app"], classes: ["issues", "code"])),
            g("g8", conn: "c1", label: "Claude", action: "read", summary: "Read all emails", expiresAt: nil, uses: 31, createdAt: now - 86_400 * 12,
              account: "work@corp.example", lines: ["All mail"], allMail: true, editable: scope(allMail: true)),
            g("g3", expiresAt: now - 86_400 * 2, uses: 3, createdAt: now - 86_400 * 3, active: false, state: "expired"),
            g("g6", conn: "c4", label: "notes-bot", expiresAt: now + 86_400, maxUses: 3, uses: 3, createdAt: now - 86_400 * 3, active: false, state: "used_up"),
            g("g7", conn: "c2", label: "My ChatGPT", action: "read", summary: "Read Telegram messages in Family", expiresAt: now - 86_400 * 2, uses: 2,
              createdAt: now - 86_400 * 4, service: "telegram", account: "+15550100", lines: ["Family"], active: false, state: "revoked",
              editable: scope(resources: ["100"])),
        ]
    }

    // ---- integrations -----------------------------------------------------------------------------------------

    static func accounts(_ now: Int64) -> [AccountView] {
        [
            AccountView(service: "gmail", account: "me@gmail.com", addedAt: now - 86_400 * 30),
            AccountView(service: "gmail", account: "work@corp.example", addedAt: now - 86_400 * 4),
            AccountView(service: "gcalendar", account: "me@gmail.com", addedAt: now - 86_400 * 30),
            AccountView(service: "telegram", account: "+15550100", addedAt: now - 86_400),
            AccountView(service: "github", account: "octo-cat", addedAt: now - 86_400 * 7),
            AccountView(service: "gitlab", account: "ada.l", addedAt: now - 86_400 * 2),
            AccountView(service: "device_calendar", account: "this phone", addedAt: now - 3_600),
            AccountView(service: "device_contacts", account: "this phone", addedAt: now - 3_600),
            AccountView(service: "vault", account: "me@example.com", addedAt: now - 86_400 * 10),
        ]
    }

    /// The catalogue, as the core lists it (accounts are filled in per call).
    static let catalogue: [(service: String, name: String, kind: String, available: Bool, note: String?)] = [
        ("gmail", "Gmail", "google", true, nil),
        ("gcalendar", "Google Calendar", "google", true, nil),
        ("gcontacts", "Google Contacts", "google", true, nil),
        ("telegram", "Telegram", "telegram", true, nil),
        ("github", "GitHub", "token", true, nil),
        ("gitlab", "GitLab", "token", true, nil),
        ("codeberg", "Codeberg", "token", true, nil),
        ("bitbucket", "Bitbucket", "token", true, nil),
        ("device_calendar", "Phone calendar", "device", true, nil),
        ("device_contacts", "Phone contacts", "device", true, nil),
        ("sms", "Text messages", "device", false, "Not available in this build."),
        ("vault", "Password vault", "vault", true, nil),
    ]

    static func mcpTools() -> [McpToolView] {
        [
            McpToolView(name: "search_issues", title: "Search issues", description: "Finds issues by text.", readOnly: true, destructive: false, heavy: false),
            McpToolView(name: "create_issue", title: "Create issue", description: "Creates an issue in a team.", readOnly: false, destructive: false, heavy: false),
            McpToolView(name: "delete_issue", title: "Delete issue", description: "Deletes an issue for good.", readOnly: false, destructive: true, heavy: false),
            McpToolView(name: "export_project", title: "Export project", description: "Exports a whole project as a file.", readOnly: true, destructive: false, heavy: true),
        ]
    }

    static func mcpServers() -> [McpServerView] {
        [
            McpServerView(id: "linear", name: "Linear", url: "https://mcp.linear.app/mcp", status: "ok", error: nil, tools: mcpTools()),
            McpServerView(id: "notion", name: "Notion", url: "https://mcp.notion.com/mcp", status: "needs_sign_in", error: nil, tools: []),
            McpServerView(id: "sentry", name: "Sentry", url: "https://mcp.sentry.dev/mcp", status: "error",
                          error: "The server did not answer in time. Try again later.", tools: Array(mcpTools().prefix(2))),
        ]
    }

    // ---- Autopilot ------------------------------------------------------------------------------------------------

    static func model(_ state: ModelState, downloaded: UInt64 = 0, error: String? = nil) -> ModelStatus {
        ModelStatus(
            state: state, id: "laya-approvals-base-v1", label: "Laya approvals (base)", version: "1",
            sizeBytes: state == .notInstalled ? 0 : modelSize, downloadedBytes: downloaded, error: error, runtimeReady: true
        )
    }

    static let modelSize: UInt64 = 412_000_000

    static func classView(
        _ key: String, _ label: String, decisions: UInt32, approved: UInt32, denied: UInt32, accuracy: Float?, autoApprove: Bool = false,
        autoDeny: Bool = false, manual: Bool? = nil, toUnlock: UInt32
    ) -> ClassView {
        ClassView(
            classKey: key, label: label, decisions: decisions, approved: approved, denied: denied, shadowAccuracy: accuracy,
            autoApprove: autoApprove, autoDeny: autoDeny, manual: manual, decisionsToUnlock: toUnlock
        )
    }

    /// Personal (default): one class decided on its own, one denied on its own, one still learning, one unlocked by
    /// hand, one kept manual. Work: young, nothing unlocked yet.
    static func profiles(_ now: Int64) -> [ProfileView] {
        [
            ProfileView(
                id: "personal", name: "Personal", icon: "🙂", preset: .balanced, isDefault: true, memoryCount: 64, connections: [],
                classes: [
                    classView("github/write/push", "GitHub · write · push", decisions: 26, approved: 25, denied: 1, accuracy: 0.98, autoApprove: true, toUnlock: 0),
                    classView("gmail/read", "Gmail · read", decisions: 14, approved: 12, denied: 2, accuracy: 0.93, autoDeny: true, toUnlock: 6),
                    classView("desktop/ask/command", "Desktop · ask · command", decisions: 5, approved: 5, denied: 0, accuracy: nil, toUnlock: 15),
                    classView("telegram/send", "Telegram · send", decisions: 22, approved: 20, denied: 2, accuracy: 0.97, manual: false, toUnlock: 0),
                    classView("gmail/send", "Gmail · send", decisions: 31, approved: 29, denied: 2, accuracy: 0.96, manual: true, toUnlock: 0),
                ],
                trainedAt: now - 86_400
            ),
            ProfileView(
                id: "work", name: "Work", icon: "💼", preset: .cautious, isDefault: false, memoryCount: 9, connections: [],
                classes: [classView("github/write/push", "GitHub · write · push", decisions: 9, approved: 9, denied: 0, accuracy: nil, toUnlock: 11)],
                trainedAt: nil
            ),
        ]
    }

    static func neighbours(_ now: Int64) -> [NeighbourView] {
        [
            NeighbourView(label: "Push to a branch · dkat/reins", verdict: .approve, similarity: 0.97, at: now - 3_600),
            NeighbourView(label: "Push to a branch · dkat/laya", verdict: .approve, similarity: 0.91, at: now - 86_400),
            NeighbourView(label: "Force push · dkat/reins", verdict: .deny, similarity: 0.74, at: now - 86_400 * 3),
        ]
    }

    static func suggestion(
        _ id: String, _ verdict: Verdict = .approve, pApprove: Float = 0.97, pDeny: Float = 0.02, confidence: Float = 0.91,
        floor: Bool = false, novel: Bool = false, judged: Bool = true, reason: String = "Like 4 times you approved: Push to a branch · dkat/reins",
        neighbours: [NeighbourView], classKey: String = "github/write/push"
    ) -> SuggestionView {
        SuggestionView(
            requestId: id, verdict: verdict, mode: .assisted, pApprove: pApprove, pDeny: pDeny, confidence: confidence, reason: reason,
            neighbours: neighbours, profileId: "personal", profileName: "Personal", classKey: classKey, novel: novel, floor: floor,
            judged: judged
        )
    }

    static func suggestions(_ now: Int64) -> [String: SuggestionView] {
        [
            "req20": suggestion("req20", neighbours: neighbours(now)),
            "req8": suggestion("req8", floor: true, judged: false, reason: "", neighbours: [], classKey: "vault/get"),
            "req10": suggestion("req10", .ask, pApprove: 0.61, pDeny: 0.3, confidence: 0.42, novel: true,
                                reason: "Nothing like this before: Commit a file · octo/app", neighbours: Array(neighbours(now).suffix(1)),
                                classKey: "github/write/code"),
        ]
    }
}
