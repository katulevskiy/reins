import Foundation

// The approval screen's state turned into what the core expects (the Android app's ApprovalLogic.kt, line for line).
// Pure: no views, no core calls, so every rule is tested in ApprovalLogicTests.

/// Lifetime choices of the approval screen: Once · 1 h · 24 h · 7 days · 30 days · until revoked · N uses.
enum LifetimeKind: String, CaseIterable, Identifiable {
    case once, hour, day, week, month, untilRevoked, uses

    var id: String { rawValue }

    var label: String {
        switch self {
        case .once: "Once"
        case .hour: "1 hour"
        case .day: "24 hours"
        case .week: "7 days"
        case .month: "30 days"
        case .untilRevoked: "Until revoked"
        case .uses: "N uses"
        }
    }

    var seconds: UInt64? {
        switch self {
        case .hour: 3_600
        case .day: 86_400
        case .week: 604_800
        case .month: 2_592_000
        case .once, .untilRevoked, .uses: nil
        }
    }

    /// What "Remember this for" offers (a month is only for showing accounts).
    static let remember: [LifetimeKind] = allCases.filter { $0 != .month }
}

enum ApprovalRules {
    static let maxUses = 1000
    /// Time boxes offered for "allow all mail": the policy never allows an open-ended grant for everything.
    static let allMailLifetimes: [LifetimeKind] = [.hour, .day, .week]
    /// Time boxes offered for "everything in this integration": never open-ended, and never for changing anything.
    static let everythingLifetimes = allMailLifetimes
    /// The periods offered for showing an integration's accounts to an AI; a month is what the user gets by default.
    static let accountsLifetimes: [LifetimeKind] = [.once, .hour, .day, .week, .month]
    /// The steps a permission request can be shortened to (the request's own length is added as the last).
    static let shortenSteps: [Int64] = [60, 600, 3_600, 6 * 3_600, 86_400, 7 * 86_400, 30 * 86_400]
}

/// Everything the user can change on the approval screen.
struct ApprovalDraft: Equatable {
    var selected: Set<String> = []
    var lifetime: LifetimeKind = .once
    var uses: Int = 5
    /// Read: false = "only these messages", true = "also allow similar".
    var similar = false
    var senderAddresses: Set<String> = []
    var senderDomains: Set<String> = []
    var subject = ""
    /// Read: allow every message (any search or read) for this long; overrides lifetime and scope when set.
    var allMail: LifetimeKind?
    /// Send: recipient addresses that are allowed by whole domain instead of exactly.
    var domainRecipients: Set<String> = []
    /// Permission requests: allow it for less time than asked (seconds); nil = as asked.
    var grantSeconds: Int64?
    /// Another integration: the chats, calendars or repositories a standing permission covers.
    var resources: Set<String> = []
    /// Another integration, a permission to change things: the kinds of change it allows (ids from `ApprovalView.classes`).
    var classes: Set<String> = []

    /// What the sheet starts with: everything found ticked (approving is one tap, unticking is how you hold something
    /// back), except a code or a password, and never for a send, a change or a permission.
    static func initial(for view: ApprovalView) -> ApprovalDraft {
        let selected: Set<String> = switch view.kind {
        case .grant, .send, .write: []
        case .accounts: Set(shareableAccounts(view))
        // A code or a password is never ticked for the user.
        case .fetch, .search, .read: Set(view.messages.filter { !$0.sensitive }.map(\.id))
        }
        // Showing the accounts is remembered for a month unless the user says otherwise.
        return ApprovalDraft(
            selected: selected,
            lifetime: view.kind == .accounts ? .month : .once,
            resources: defaultResources(view),
            classes: defaultClasses(view)
        )
    }
}

enum BuildResult: Equatable {
    case ok(ApprovalChoice)
    case invalid(String)

    var choice: ApprovalChoice? {
        if case let .ok(c) = self { return c }
        return nil
    }
}

/// A scope with nothing set; each rule below fills in what it allows.
private func emptyScope(allMail: Bool = false, selectedOnly: Bool = false) -> GrantScopeChoice {
    GrantScopeChoice(
        allMail: allMail, selectedMessagesOnly: selectedOnly, senderAddresses: [], senderDomains: [], subjectPattern: nil,
        recipientAddresses: [], recipientDomains: [], resources: [], classes: []
    )
}

/// `Name <a@b.com>` or `a@b.com` → `a@b.com` (lower-cased); nil when there is no usable address.
func addressOf(_ line: String) -> String? {
    var candidate = line.trimmingCharacters(in: .whitespacesAndNewlines)
    if let match = line.firstMatch(of: #/<([^<>\s]+)>\s*$/#) { candidate = String(match.1) }
    candidate = candidate.lowercased()
    guard let at = candidate.firstIndex(of: "@"), at == candidate.lastIndex(of: "@"), at > candidate.startIndex,
          candidate.index(after: at) < candidate.endIndex, !candidate.contains(where: \.isWhitespace) else { return nil }
    return candidate
}

func domainOf(_ address: String) -> String {
    (address.components(separatedBy: "@").last ?? address).lowercased()
}

/// Distinct sender addresses of the messages in `view` that the user ticked, in order of appearance.
func senderAddresses(_ view: ApprovalView, selected: Set<String>) -> [String] {
    view.messages.filter { selected.contains($0.id) }.compactMap { addressOf($0.from) }.uniqued()
}

func recipientAddresses(_ view: ApprovalView) -> [String] {
    guard let email = view.email else { return [] }
    return (email.to + email.cc).compactMap(addressOf).uniqued()
}

/// Turns the screen state into what the core expects, or says what is missing. Never guesses.
func buildChoice(_ view: ApprovalView, _ draft: ApprovalDraft) -> BuildResult {
    switch view.kind {
    case .grant: return buildPermissionAnswer(view, draft)
    case .accounts: return buildAccountsAnswer(view, draft)
    case .fetch, .write: return buildConnectorAnswer(view, draft)
    case .search, .read, .send: break
    }
    let send = view.kind == .send
    let covered = Set(view.messages.filter(\.coveredByGrant).map(\.id))
    // "All mail" releases everything shown but what looks like a code, so nothing has to be ticked.
    let ids: [String] = if send {
        []
    } else if draft.allMail != nil {
        view.messages.filter { !$0.sensitive || draft.selected.contains($0.id) }.map(\.id)
    } else {
        view.messages.map(\.id).filter { draft.selected.contains($0) || covered.contains($0) }
    }
    if draft.allMail != nil { return buildAllMail(view, draft, ids) }
    if !send && ids.isEmpty { return .invalid("Select at least one message.") }
    var standing: StandingGrant?
    if draft.lifetime != .once {
        if draft.lifetime == .uses && !(1...ApprovalRules.maxUses).contains(draft.uses) {
            return .invalid("Uses must be between 1 and \(ApprovalRules.maxUses).")
        }
        let scope: GrantScopeChoice
        switch send ? sendScope(view, draft) : readScope(draft) {
        case let .ok(s): scope = s
        case let .missing(message): return .invalid(message)
        }
        standing = StandingGrant(
            durationSecs: draft.lifetime.seconds,
            maxUses: draft.lifetime == .uses ? UInt32(draft.uses) : nil,
            scope: scope
        )
    }
    return .ok(ApprovalChoice(selectedMessageIds: ids, standing: standing))
}

/// Lists, reads and searches in another integration (the ticked items are released) and changes to it (the preview is
/// what will be done). A permission made alongside covers the things ticked under "What should it cover?".
private func buildConnectorAnswer(_ view: ApprovalView, _ draft: ApprovalDraft) -> BuildResult {
    let write = view.kind == .write
    let covered = Set(view.messages.filter(\.coveredByGrant).map(\.id))
    let ids = write ? [] : view.messages.map(\.id).filter { draft.selected.contains($0) || covered.contains($0) }
    if !write && ids.isEmpty && !view.messages.isEmpty { return .invalid("Tick at least one item.") }
    if view.noStanding || (draft.lifetime == .once && draft.allMail == nil) {
        return .ok(ApprovalChoice(selectedMessageIds: ids, standing: nil))
    }
    if draft.lifetime == .uses && !(1...ApprovalRules.maxUses).contains(draft.uses) {
        return .invalid("Uses must be between 1 and \(ApprovalRules.maxUses).")
    }
    if let everything = draft.allMail {
        if write { return .invalid("Changing things cannot be allowed everywhere.") }
        if !ApprovalRules.everythingLifetimes.contains(everything) {
            return .invalid("Allowing everything needs a time limit: 1 hour, 24 hours or 7 days.")
        }
        return .ok(ApprovalChoice(selectedMessageIds: ids, standing: StandingGrant(durationSecs: everything.seconds, maxUses: nil, scope: emptyScope(allMail: true))))
    }
    let known = Set(view.resources.map(\.id))
    let picked = draft.resources.filter { known.contains($0) }
    // A wider permission already covers what is inside it: `owner/repo` and `owner/repo@main` is just `owner/repo`.
    let chosen = picked.filter { id in !picked.contains { other in other != id && resourceCovers(other, id) } }.sorted()
    if chosen.isEmpty { return .invalid("Choose what the permission should cover.") }
    // Nothing ticked (or nothing offered) = the kind of this request, which is what the core assumes.
    let classes = write ? view.classes.map(\.id).filter { draft.classes.contains($0) } : []
    var scope = emptyScope()
    scope.resources = chosen
    scope.classes = classes
    return .ok(ApprovalChoice(
        selectedMessageIds: ids,
        standing: StandingGrant(
            durationSecs: draft.lifetime.seconds,
            maxUses: draft.lifetime == .uses ? UInt32(draft.uses) : nil,
            scope: scope
        )
    ))
}

/// The permission for `granted` also covers `thing`: the same thing, or something inside it (`A` covers `A`, `A@x`, `A/x`).
func resourceCovers(_ granted: String, _ thing: String) -> Bool {
    if thing == granted { return true }
    guard thing.hasPrefix(granted), thing.count > granted.count else { return false }
    let next = thing[thing.index(thing.startIndex, offsetBy: granted.count)]
    return next == "@" || next == "/"
}

/// What starts ticked under "What should it cover?": the things the request touches, not the wider ones they belong to.
func defaultResources(_ view: ApprovalView) -> Set<String> { Set(view.resources.filter { !$0.wider }.map(\.id)) }

/// What starts ticked under "Allow these kinds of change": only the kind of this request.
func defaultClasses(_ view: ApprovalView) -> Set<String> { Set(view.classes.map(\.id).filter { $0 == view.class }) }

/// Ticks or unticks `id`. Ticking a wider thing unticks the ones inside it (its permission covers them anyway), and
/// ticking a narrower one unticks the wider ones around it, so the ticks never say the same thing twice.
func toggleResource(_ selected: Set<String>, _ id: String, on: Bool) -> Set<String> {
    if !on { return selected.subtracting([id]) }
    return selected.filter { other in !resourceCovers(id, other) && !resourceCovers(other, id) }.union([id])
}

/// Ticks or unticks a kind of change; the last one stays ticked.
func toggleClass(_ selected: Set<String>, _ id: String, on: Bool) -> Set<String> {
    if on { return selected.union([id]) }
    return selected == [id] ? selected : selected.subtracting([id])
}

/// The accounts that can still be ticked: those the AI was not already allowed to see.
func shareableAccounts(_ view: ApprovalView) -> [String] { view.accounts.filter { !view.sharedAccounts.contains($0) } }

/// Showing the ticked accounts once, or for a while (which leaves a grant naming exactly those accounts for this AI).
/// The addresses travel in `selectedMessageIds`.
private func buildAccountsAnswer(_ view: ApprovalView, _ draft: ApprovalDraft) -> BuildResult {
    let picked = shareableAccounts(view).filter { draft.selected.contains($0) }
    if picked.isEmpty { return .invalid("Tick at least one account, or deny the request.") }
    guard ApprovalRules.accountsLifetimes.contains(draft.lifetime) else { return .invalid("Choose once, or one of the periods.") }
    let standing = draft.lifetime.seconds.map { StandingGrant(durationSecs: $0, maxUses: nil, scope: emptyScope()) }
    return .ok(ApprovalChoice(selectedMessageIds: picked, standing: standing))
}

/// Allowing a permission an AI asked for: as asked, or (never longer) for less time.
private func buildPermissionAnswer(_ view: ApprovalView, _ draft: ApprovalDraft) -> BuildResult {
    guard let asked = view.grant?.durationSecs else { return .invalid("This request has no permission attached.") }
    let shorter = draft.grantSeconds.flatMap { $0 >= 0 ? UInt64($0) : nil }.flatMap { $0 < asked ? $0 : nil }
    let standing = shorter.map { StandingGrant(durationSecs: $0, maxUses: nil, scope: emptyScope()) }
    return .ok(ApprovalChoice(selectedMessageIds: [], standing: standing))
}

private func buildAllMail(_ view: ApprovalView, _ draft: ApprovalDraft, _ ids: [String]) -> BuildResult {
    if view.kind == .send { return .invalid("Sending cannot be allowed for everyone.") }
    guard let lifetime = draft.allMail, ApprovalRules.allMailLifetimes.contains(lifetime) else {
        return .invalid("Allowing all mail needs a time limit: 1 hour, 24 hours or 7 days.")
    }
    return .ok(ApprovalChoice(selectedMessageIds: ids, standing: StandingGrant(durationSecs: lifetime.seconds, maxUses: nil, scope: emptyScope(allMail: true))))
}

private enum ScopeResult {
    case ok(GrantScopeChoice)
    case missing(String)
}

private func readScope(_ draft: ApprovalDraft) -> ScopeResult {
    if !draft.similar { return .ok(emptyScope(selectedOnly: true)) }
    let subject = draft.subject.trimmingCharacters(in: .whitespacesAndNewlines)
    if draft.senderAddresses.isEmpty && draft.senderDomains.isEmpty && subject.isEmpty {
        return .missing("Choose at least one sender, domain or subject text for the similar mail.")
    }
    var scope = emptyScope()
    scope.senderAddresses = draft.senderAddresses.sorted()
    scope.senderDomains = draft.senderDomains.sorted()
    scope.subjectPattern = subject.isEmpty ? nil : subject
    return .ok(scope)
}

private func sendScope(_ view: ApprovalView, _ draft: ApprovalDraft) -> ScopeResult {
    let recipients = recipientAddresses(view)
    let domains = recipients.filter { draft.domainRecipients.contains($0) }.map(domainOf).uniqued().sorted()
    let exact = recipients.filter { !draft.domainRecipients.contains($0) }
    if exact.isEmpty && domains.isEmpty { return .missing("This email has no recipients to allow.") }
    let subject = draft.subject.trimmingCharacters(in: .whitespacesAndNewlines)
    var scope = emptyScope()
    scope.subjectPattern = subject.isEmpty ? nil : subject
    scope.recipientAddresses = exact
    scope.recipientDomains = domains
    return .ok(scope)
}

/// Free mail providers: allowing one of these domains allows millions of unrelated people.
enum PublicMailDomains {
    private static let domains: Set<String> = [
        "gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "live.com", "msn.com", "yahoo.com", "ymail.com",
        "rocketmail.com", "icloud.com", "me.com", "mac.com", "aol.com", "proton.me", "protonmail.com", "pm.me",
        "gmx.com", "gmx.de", "gmx.net", "web.de", "mail.com", "mail.ru", "yandex.com", "yandex.ru", "zoho.com",
        "fastmail.com", "hey.com", "tutanota.com", "tuta.io", "qq.com", "163.com", "126.com", "comcast.net",
        "att.net", "verizon.net", "sbcglobal.net", "orange.fr", "free.fr", "t-online.de", "libero.it",
    ]

    static func isPublic(_ domain: String) -> Bool {
        var d = domain.lowercased()
        while d.hasPrefix("@") { d.removeFirst() }
        return domains.contains(d)
    }
}

/// Public mail domains a standing grant built from `choice` would allow as a whole.
func publicDomainWarnings(_ choice: ApprovalChoice) -> [String] {
    guard let scope = choice.standing?.scope else { return [] }
    return (scope.senderDomains + scope.recipientDomains).filter(PublicMailDomains.isPublic).uniqued()
}

/// The steps a permission of `asked` seconds can be shortened to, ending with `asked` itself.
func shortenSteps(asked: Int64) -> [Int64] {
    ApprovalRules.shortenSteps.filter { $0 < asked } + [asked]
}

fileprivate extension Array where Element: Hashable {
    /// The elements in order, each only the first time it appears.
    func uniqued() -> [Element] {
        var seen = Set<Element>()
        return filter { seen.insert($0).inserted }
    }
}
