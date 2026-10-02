import Foundation

// The integration screens' pure logic, kept apart from the views so it can be tested (the Android app's
// `ServiceScreen.kt` helpers, `GitHosts.kt` and `McpLogic.kt`).

// MARK: GitHub

/// GitHub's own token pages, filled in with what the tools need, and how a copied token is recognised.
enum GitHubToken {
    /// The fine-grained token page. GitHub refuses a second token with the same name, so `fineGrainedURL` gives each
    /// one its own number. Write access where the tools change something, read for the alert lists, metadata is
    /// always read.
    static let fineGrainedPage =
        "https://github.com/settings/personal-access-tokens/new?name=Rewarden&description=Lets+my+AI+work+with+my+repositories+through+the+Rewarden+app&expires_in=180"
            + "&metadata=read&contents=write&issues=write&pull_requests=write&actions=write&workflows=write&administration=write"
            + "&repository_hooks=write&secrets=write&variables=write&environments=write&checks=write&statuses=write"
            + "&security_events=read&vulnerability_alerts=read&secret_scanning_alerts=read"

    /// The classic token page: one token for everything, including the account-level things fine-grained tokens
    /// cannot do.
    static let classicPage =
        "https://github.com/settings/tokens/new?description=Rewarden&scopes=repo,workflow,gist,notifications,read:org,admin:repo_hook,delete_repo"

    static func fineGrainedURL(suffix: Int = Int.random(in: 100_000...999_999)) -> String {
        fineGrainedPage.replacingOccurrences(of: "name=Rewarden&", with: "name=Rewarden-\(suffix)&")
    }

    static func classicURL(suffix: Int = Int.random(in: 100_000...999_999)) -> String {
        classicPage.replacingOccurrences(of: "description=Rewarden&", with: "description=Rewarden-\(suffix)&")
    }

    /// A token is recognised by its prefix, so nothing else that happens to be on the clipboard is ever used.
    static func looksLikeToken(_ text: String?) -> Bool {
        guard let t = text?.trimmingCharacters(in: .whitespacesAndNewlines) else { return false }
        return (30...255).contains(t.count) && t.wholeMatch(of: #/(github_pat_|ghp_|gho_|ghu_|ghs_)[A-Za-z0-9_]+/#) != nil
    }
}

// MARK: Other git hosts

/// A git host besides GitHub that is connected with a pasted token: where its token page is (filled in where the
/// host allows it), what to do there, and what a token looks like so that one copied there can be recognised.
struct GitHost {
    var service: String
    var name: String
    var page: (Int) -> String
    /// A copied token is recognised by its shape, so nothing else on the clipboard is ever used; nil: never taken
    /// from the clipboard by itself.
    var shape: Regex<Substring>?
    /// What to do on the token page.
    var steps: String
    /// The paste field's placeholder and the line under it.
    var placeholder: String
    var hint: String
    /// The paste field is there from the start (the token alone is not enough to connect).
    var pasteFirst = false

    func tokenURL(suffix: Int = Int.random(in: 100_000...999_999)) -> String { page(suffix) }

    func looksLikeToken(_ text: String?) -> Bool {
        guard let shape, let t = text?.trimmingCharacters(in: .whitespacesAndNewlines) else { return false }
        return t.wholeMatch(of: shape) != nil
    }
}

enum GitHosts {
    private static let all: [String: GitHost] = Dictionary(uniqueKeysWithValues: [
        GitHost(
            service: "gitlab",
            name: "GitLab",
            // GitLab refuses nothing by name, but a fresh number tells the tokens apart in its list.
            page: { "https://gitlab.com/-/user_settings/personal_access_tokens?name=Rewarden-\($0)&scopes=read_api,read_repository,write_repository" },
            shape: #/glpat-[A-Za-z0-9_.\-]{20,250}/#,
            steps: "GitLab opens with a new token already set up: read_api, read_repository and write_repository. Pick an expiry date, tap Create token, then copy it. Come back here and paste it.",
            placeholder: "Personal access token (glpat-…)",
            hint: "A GitLab personal access token with read_api, read_repository and write_repository. It is kept encrypted on this phone."
        ),
        GitHost(
            service: "codeberg",
            name: "Codeberg",
            page: { _ in "https://codeberg.org/user/settings/applications" },
            shape: #/[0-9a-f]{40}/#,
            steps: "Under Generate new token, name it Rewarden, choose Select permissions, and set repository to Read and write and user to Read (read:user, read:repository, write:repository). Tap Generate token and copy it. Come back here and paste it.",
            placeholder: "Access token",
            hint: "A Codeberg access token with read:user and read and write access to repositories. It is kept encrypted on this phone."
        ),
        GitHost(
            service: "bitbucket",
            name: "Bitbucket",
            page: { _ in "https://id.atlassian.com/manage-profile/security/api-tokens" },
            shape: nil,
            steps: "Tap Create API token with scopes, name it Rewarden, choose Bitbucket, and tick read:user:bitbucket, read:repository:bitbucket and write:repository:bitbucket. Copy the token. Bitbucket checks it together with your Atlassian account email, so paste both below as email:token, for example you@example.com:ATATT3xF… (app passwords no longer work).",
            placeholder: "email:token",
            hint: "Your Atlassian account email, a colon, then the API token. It is kept encrypted on this phone.",
            pasteFirst: true
        ),
    ].map { ($0.service, $0) })

    /// The host behind `service`; nil for GitHub (which has its own steps) and everything else.
    static func of(_ service: String) -> GitHost? { all[service] }
}

// MARK: MCP servers

enum McpLogic {
    /// Where an MCP server's sign-in page sends the browser back to (the core registers it with the server).
    static let redirectScheme = "dev.rewarden.android"
    static let redirectHost = "mcp-oauth"
    /// Longest redirect address passed on to the core; a real one is a code and a state.
    static let maxRedirectChars = 8_192

    /// The host (and port) of a server's address, as the list shows it; never its path or query.
    static func host(_ url: String) -> String {
        guard let c = URLComponents(string: url.trimmingCharacters(in: .whitespaces)), let host = c.host, !host.isEmpty else { return url }
        return c.port.map { "\(host):\($0)" } ?? host
    }

    static func statusLabel(_ status: String) -> String {
        switch status {
        case "ok": "Connected"
        case "needs_sign_in": "Needs sign-in"
        default: "Error"
        }
    }

    static func toolCount(_ n: Int) -> String {
        switch n {
        case 0: "No tools"
        case 1: "1 tool"
        default: "\(n) tools"
        }
    }

    /// The sign-in page sent the browser to the app's own redirect address (anything else is never handed to the
    /// core).
    static func isRedirect(_ uri: String?) -> Bool {
        guard let uri, uri.count <= maxRedirectChars, let c = URLComponents(string: uri) else { return false }
        return c.scheme == redirectScheme && c.host == redirectHost && c.user == nil && c.password == nil && c.port == nil
    }

    /// A sign-in page may only be a web page (never a `file:`, `javascript:` or another app's address).
    static func isWebPage(_ url: String) -> Bool {
        guard let c = URLComponents(string: url), let scheme = c.scheme?.lowercased() else { return false }
        return (scheme == "https" || scheme == "http") && !(c.host ?? "").isEmpty
    }
}

/// What a tool's badges say about it.
enum ToolBadge: String, CaseIterable {
    case readOnly, changes, asksEveryTime, heavy

    var label: String {
        switch self {
        case .readOnly: "Read only"
        case .changes: "Changes things"
        case .asksEveryTime: "Asks every time"
        case .heavy: "Large results"
        }
    }

    static func of(_ tool: McpToolView) -> [ToolBadge] {
        var badges: [ToolBadge] = [tool.readOnly ? .readOnly : .changes]
        if tool.destructive { badges.append(.asksEveryTime) }
        if tool.heavy { badges.append(.heavy) }
        return badges
    }
}

// MARK: Accounts

/// Where a phone-number sign-in stands (Telegram).
enum TelegramStep: Equatable {
    /// Not started: the phone number is asked for.
    case phone
    /// The code Telegram sent is asked for.
    case code(phone: String)
    /// The account has two-step verification.
    case password(hint: String?)

    /// Enough digits to be a phone number with its country code.
    static func phoneReady(_ phone: String) -> Bool { phone.filter(\.isASCII).filter(\.isNumber).count >= 7 }

    /// What the code field keeps of what was typed or pasted: digits, at most eight.
    static func cleanCode(_ text: String) -> String { String(text.filter { $0.isASCII && $0.isNumber }.prefix(8)) }

    static func codeReady(_ code: String) -> Bool { code.count >= 4 }
}

/// The words of the integration screens that depend on the service (Android's `intro`, `accountsFooter`,
/// `fineprint`, `needsAgain`).
enum ServiceCopy {
    /// iOS gives apps no access to text messages; the core still lists the service, so it is shown as unavailable.
    static let sms = ServiceView(
        service: "sms", name: "Text messages", kind: "device", available: false,
        note: "iOS does not let apps read or send text messages, so your AIs cannot use them through an iPhone.", accounts: []
    )

    /// The heading of an empty accounts list.
    static func emptyTitle(_ service: ServiceView) -> String {
        if !service.available { return "Not available" }
        return service.service == "gmail" ? "No account yet" : "Not connected"
    }

    /// The line under a service in the list.
    static func summary(_ service: ServiceView) -> String {
        let n = service.accounts.count
        if !service.available { return "Not available" }
        return switch n {
        case 0: "Not connected"
        case 1: "1 account"
        default: "\(n) accounts"
        }
    }

    static func intro(_ service: ServiceView) -> String {
        switch service.service {
        case "gmail": "Add a Google account so your AIs can search and send mail through this phone."
        case "gcalendar": "Add a Google account so your AIs can see your events and add new ones."
        case "gcontacts": "Add a Google account so your AIs can look up your contacts."
        case "telegram": "Sign in with your own Telegram account, the way you do in the Telegram app. It is not a bot."
        case "github": "Connect a GitHub access token so your AIs can work with your repositories: read code and issues, and, when you approve, change them."
        case "gitlab", "codeberg", "bitbucket":
            "Connect a \(service.name) access token so git on your computer can clone, fetch and, when you approve each push, push through the Rewarden desktop app."
        case "device_calendar": "Let your AIs see and add events in the calendars on this phone."
        case "device_contacts": "Let your AIs look up the contacts on this phone."
        case "sms": "On Android, Rewarden lets your AIs read and send text messages. iPhone keeps them to the Messages app."
        case "vault": "Let your AIs ask for a login from your password vault, one field at a time."
        default: "Connect an account so your AIs can use it."
        }
    }

    static func accountsFooter(_ service: ServiceView) -> String {
        switch service.kind {
        case "device": "This works with what is on this phone. You approve what your AIs see or do."
        case "vault": "Passwords, one-time codes and usernames are asked for every time and never remembered."
        default: "Your AIs see this list and pick the account a request is about. Grants belong to one account."
        }
    }

    /// The small print at the bottom of a service's page; empty for none.
    static func fineprint(_ service: ServiceView) -> String {
        switch service.service {
        case "gmail": "Reins asks Google for access on this phone only. Nothing about your mail is stored on the server."
        case "telegram":
            "Reins signs in as you on this phone only; the session is kept encrypted here and never sent to the server. Telegram may limit accounts that are used by automation, so Reins only acts when you approve."
        case "github": "The token is kept encrypted on this phone. Every change is shown to you first, and dangerous ones are asked for every time."
        case "gitlab", "codeberg", "bitbucket":
            "The token is kept encrypted on this phone and never leaves it. Each push is shown to you branch by branch before it goes to \(service.name)."
        case "sms": ""
        case "vault": "Your master password is used once to unlock the vault key, which is then kept encrypted on this phone. The password itself is not kept."
        case "device_calendar", "device_contacts": "iOS asks you to allow this. You can take the permission back in Settings at any time."
        default: "Reins asks Google for access on this phone only. Nothing is stored on the server."
        }
    }

    /// The status line of an account that needs the user again.
    static func needsAgain(_ service: ServiceView) -> String {
        switch service.kind {
        case "device": "Needs the permission again"
        case "telegram": "Signed out: remove it and sign in again"
        case "token": "The token no longer works: remove it and add a new one"
        case "vault": "Remove it and enter the master password again"
        default: "Needs your permission again"
        }
    }

    /// "Allow again" can fix it: a Google consent or this phone's permission.
    static func canAllowAgain(_ service: ServiceView) -> Bool { service.kind == "google" || service.kind == "device" }

    /// How an account is named in its row and the removal question: this phone's services by the service.
    static func accountTitle(_ service: ServiceView, _ account: String) -> String {
        service.kind == "device" ? service.name : account
    }

    static func removeTitle(_ service: ServiceView, _ account: String) -> String {
        "Remove \(accountTitle(service, account))?"
    }

    static func removeMessage(gmail: Bool) -> String {
        gmail
            ? "Your AIs lose access to this account, and the grants made for it are deleted."
            : "Your AIs lose access to this, and the grants made for it are deleted."
    }

    static let permissionRefused = "iOS did not allow it. You can allow it in Settings for Reins."
}
