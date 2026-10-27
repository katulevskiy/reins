import Foundation

/// Circles (decisions v2, 04) in words and order, pure and unit-tested: who someone is, what a role and a scope are
/// called, the owner's progress line on a card, "For others" grouped by person with the soonest deadline first, and the
/// one Circle notification's title. The app and the notification extension say the same.
///
/// Everything a Circle shows came from another account (a person's email, a scope line, a request's title), so all of
/// it goes through `untrusted`.
enum CircleLogic {
    // MARK: People

    /// "Sam" for "sam.lee@example.com": the first word of the address's name, capitalised. The email itself is only a
    /// label (the safety code proves who it is), so the short name is never more than that.
    static func name(_ email: String) -> String {
        let clean = untrustedName(email, limit: 254)
        let local = clean.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false).first.map(String.init) ?? clean
        let word = local.split(whereSeparator: { ".+_-".contains($0) }).first.map(String.init) ?? local
        guard let first = word.first else { return clean }
        let named = first.uppercased() + word.dropFirst()
        return String(named.prefix(32))
    }

    /// "Alex", "Alex and Sam", "Alex, Sam and Dana".
    static func names(_ emails: [String]) -> String {
        let all = emails.map(name).filter { !$0.isEmpty }
        switch all.count {
        case 0: return ""
        case 1: return all[0]
        default: return all.dropLast().joined(separator: ", ") + " and " + all[all.count - 1]
        }
    }

    // MARK: Roles

    static func roleName(_ role: CircleRole) -> String {
        switch role {
        case .delegate: "Delegate"
        case .cosign: "Co-sign"
        case .guardian: "Guardian"
        case .watcher: "Watcher"
        }
    }

    /// The SF Symbol of a role.
    static func roleSymbol(_ role: CircleRole) -> String {
        switch role {
        case .delegate: "person.badge.shield.checkmark"
        case .cosign: "person.2"
        case .guardian: "hand.raised"
        case .watcher: "eye"
        }
    }

    /// What the role means, in a few words: from the owner's side (`mine`), or the helper's.
    static func roleHint(_ role: CircleRole, mine: Bool) -> String {
        switch (role, mine) {
        case (.delegate, true): "Decides instead of you"
        case (.cosign, true): "Must approve too"
        case (.guardian, true): "Must approve"
        case (.watcher, true): "Is told, no buttons"
        case (.delegate, false): "You decide for them"
        case (.cosign, false): "You approve too"
        case (.guardian, false): "You must approve"
        case (.watcher, false): "You're told, no buttons"
        }
    }

    /// The roles the Add screen offers, in its order.
    static let roles: [CircleRole] = [.delegate, .cosign, .guardian, .watcher]

    // MARK: Scopes

    /// The presets the Add screen offers before "One AI" (which names a connection).
    static let presets: [CircleScopePreset] = [.everything, .money, .messagesOutside, .risky, .routine]

    static func presetTitle(_ preset: CircleScopePreset) -> String {
        switch preset {
        case .everything: "Everything"
        case .money: "Money"
        case .messagesOutside: "Messages to Work or Outside"
        case .risky: "What can't be undone"
        case .routine: "Routine"
        case let .agent(_, label): untrustedName(label, limit: 40).isEmpty ? "One AI" : untrustedName(label, limit: 40)
        }
    }

    static func presetSymbol(_ preset: CircleScopePreset) -> String {
        switch preset {
        case .everything: "square.stack.3d.up"
        case .money: "creditcard"
        case .messagesOutside: "envelope"
        case .risky: "exclamationmark.triangle"
        case .routine: "arrow.triangle.2.circlepath"
        case .agent: "sparkles"
        }
    }

    /// Whether the helper sees the card as the owner does (else a summary): full for a delegate and a co-signer,
    /// a summary for a guardian, never anything for a watcher (04 §5).
    static func fullCardsByDefault(_ role: CircleRole) -> Bool { role == .delegate || role == .cosign }

    /// What the Add screen sends: the class's default window, no fallback, the role's default view of the card. A
    /// guardianship's notice is the ward's pick (`GuardianLogic.draft`).
    static func draft(role: CircleRole, preset: CircleScopePreset, deferred: Bool, noticeDays: UInt8? = nil) -> CircleDraft {
        CircleDraft(
            role: role, preset: preset, deferred: deferred, windowSecs: nil, fallBackToMe: false, fullCards: fullCardsByDefault(role),
            noticeDays: role == .guardian ? noticeDays.flatMap { GuardianLogic.noticeChoices.contains($0) ? $0 : nil } : nil
        )
    }

    /// "2 min", "4 h", "2 d".
    static func window(_ secs: UInt64) -> String {
        switch secs {
        case ..<3_600: "\(max(secs / 60, 1)) min"
        case ..<172_800: "\(secs / 3_600) h"
        default: "\(secs / 86_400) d"
        }
    }

    /// "Deferred · 4 h", "Instant · 2 min".
    static func classLine(deferred: Bool, windowSecs: UInt64) -> String {
        let kind = deferred ? "Deferred" : "Instant"
        return windowSecs > 0 ? "\(kind) · \(window(windowSecs))" : kind
    }

    // MARK: The list (Settings › Circles)

    /// What a state says beside a person; nil while it runs.
    static func badge(_ state: CircleLinkState) -> String? {
        switch state {
        case .active: nil
        case .paused: "Paused"
        case .ended: "Ended"
        case .suspended: "Needs a new invite"
        }
    }

    /// The two lists: who helps this account (`mine`), and whom this account helps. Running ones first, then by name;
    /// what ended last.
    static func split(_ people: [CirclePerson]) -> (helpingYou: [CirclePerson], youHelp: [CirclePerson]) {
        let sorted = people.sorted { a, b in
            let ra = rank(a.state), rb = rank(b.state)
            if ra != rb { return ra < rb }
            let na = name(a.peer).lowercased(), nb = name(b.peer).lowercased()
            if na != nb { return na < nb }
            return a.linkId < b.linkId
        }
        return (sorted.filter(\.mine), sorted.filter { !$0.mine })
    }

    private static func rank(_ state: CircleLinkState) -> Int {
        switch state {
        case .active: 0
        case .paused: 1
        case .suspended: 2
        case .ended: 3
        }
    }

    /// The owner pauses and resumes what they set up; nothing ended can be. Never a ward its own guardianship (04 §4:
    /// it can't be removed silently; the core refuses too).
    static func canPause(_ p: CirclePerson) -> Bool {
        p.mine && !GuardianLogic.isWard(p) && (p.state == .active || p.state == .paused)
    }

    /// Ending is the owner's word; a helper leaves, and so does a ward.
    static func endTitle(_ p: CirclePerson) -> String { p.mine && !GuardianLogic.isWard(p) ? "End" : "Leave" }

    /// Anything not ended; a ward's guardianship once only (it is ending, or waits for the guardian, after).
    static func canEnd(_ p: CirclePerson) -> Bool {
        p.state != .ended && !(GuardianLogic.isWard(p) && (p.ending != nil || p.needsConsent))
    }

    // MARK: The owner's card

    /// The progress line on the owner's card: the core's own line ("Needs Alex too", "Alex approved, waiting for you"),
    /// else one made from who still answers and who approved.
    static func progressLine(_ p: CircleProgress) -> String {
        let line = untrusted(p.line)
        if !line.isEmpty { return line }
        if p.youApproved {
            return p.needs.isEmpty ? "You approved" : "You approved · waiting for \(names(p.needs))"
        }
        if !p.approved.isEmpty && p.waitingForYou { return "\(names(p.approved)) approved, waiting for you" }
        if !p.needs.isEmpty { return "Needs \(names(p.needs)) too" }
        return p.waitingForYou ? "Waiting for you" : ""
    }

    // MARK: For others

    /// One person's reviews waiting for this phone, the soonest due first.
    struct ReviewGroup: Identifiable, Equatable {
        var person: String
        var reviews: [CircleReviewView]
        /// The soonest deadline among them.
        var deadline: Int64

        var id: String { person }
    }

    /// The reviews that wait, by person (the email, so two people of one short name never mix), the group with the
    /// soonest deadline first; in each, the soonest due first, then the oldest, then by id, so the order never jumps.
    static func groups(_ reviews: [CircleReviewView]) -> [ReviewGroup] {
        let waiting = reviews.filter { $0.state == .waiting }.sorted(by: soonest)
        var order: [String] = []
        var by: [String: [CircleReviewView]] = [:]
        for r in waiting {
            if by[r.person] == nil { order.append(r.person) }
            by[r.person, default: []].append(r)
        }
        return order.compactMap { person in
            guard let items = by[person], let first = items.first else { return nil }
            return ReviewGroup(person: person, reviews: items, deadline: first.deadline)
        }
    }

    private static func soonest(_ a: CircleReviewView, _ b: CircleReviewView) -> Bool {
        if a.deadline != b.deadline { return a.deadline < b.deadline }
        if a.createdAt != b.createdAt { return a.createdAt < b.createdAt }
        return a.id < b.id
    }

    /// What was settled lately (shown dimmed under what waits), newest first, at most `limit`.
    static func settled(_ reviews: [CircleReviewView], limit: Int = 10) -> [CircleReviewView] {
        Array(reviews.filter { $0.state != .waiting }.sorted { a, b in
            a.createdAt != b.createdAt ? a.createdAt > b.createdAt : a.id < b.id
        }.prefix(limit))
    }

    /// A settled review's outcome in a word or two.
    static func outcome(_ state: CircleReviewState) -> String {
        switch state {
        case .waiting: "Waiting"
        case .approved: "You approved"
        case .denied: "You denied"
        case .expired: "Expired"
        case .withdrawn: "Withdrawn"
        }
    }

    /// "Sam's ChatGPT": whose agent asks, on top of the card.
    static func whose(_ r: CircleReviewView) -> String {
        let agent = untrustedName(r.agent, limit: 40)
        let person = name(r.person)
        return agent.isEmpty ? person : "\(person)'s \(agent)"
    }

    /// "Due in 3 h", "Overdue".
    static func due(_ deadline: Int64, now: Int64) -> String { TicketText.due(deadline: deadline, now: now) }

    /// What a watcher was told, newest first.
    static func notices(_ list: [CircleNoticeView], limit: Int = 20) -> [CircleNoticeView] {
        Array(list.sorted { a, b in a.at != b.at ? a.at > b.at : a.id < b.id }.prefix(limit))
    }

    /// A note as typed: trimmed, at most 500 characters; nil when empty.
    static func note(_ text: String?) -> String? {
        let t = (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        return t.isEmpty ? nil : String(t.prefix(500))
    }

    // MARK: The notification

    /// "Sam: purchase $42 waiting" (04 §6.2): the one Circle notification, titled by person.
    static func notificationTitle(_ r: CircleReviewView) -> String {
        var what = untrusted(r.title)
        if what.isEmpty { what = "a request" }
        return "\(name(r.person)): \(lowerFirst(what)) waiting"
    }

    /// The first letter in lower case, unless the word is an acronym ("SSH key" stays).
    static func lowerFirst(_ s: String) -> String {
        let chars = Array(s)
        guard let first = chars.first, first.isUppercase else { return s }
        if chars.count > 1, chars[1].isUppercase { return s }
        return first.lowercased() + String(chars.dropFirst())
    }
}
