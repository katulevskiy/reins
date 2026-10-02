import Foundation

/// How long a grant made in advance lasts.
enum NewGrantLifetime: CaseIterable, Hashable {
    case oneTime, hour, day, week, month

    var label: String {
        switch self {
        case .oneTime: "One time"
        case .hour: "1 hour"
        case .day: "24 hours"
        case .week: "7 days"
        case .month: "30 days"
        }
    }

    /// nil for one time: it stays until it is used once.
    var seconds: Int64? {
        switch self {
        case .oneTime: nil
        case .hour: 3_600
        case .day: 86_400
        case .week: 604_800
        case .month: 2_592_000
        }
    }

    /// All mail needs a time limit of a week at most.
    var allowedForAllMail: Bool { self != .oneTime && self != .month }
}

/// What the "New grant" form holds.
struct NewGrantDraft: Equatable {
    var connectionId: String?
    /// Which connected Gmail account the permission is for.
    var account: String?
    var send = false
    /// Read only: every email (needs a time limit of at most 7 days).
    var anyMail = false
    /// Addresses (`a@b.com`) and domains (`@b.com` or `b.com`), separated by commas, spaces or lines.
    var partiesText = ""
    var subject = ""
    var lifetime: NewGrantLifetime = .day
}

enum NewGrantResult: Equatable {
    case ok(connectionId: String, account: String, kind: ApprovalKind, standing: StandingGrant)
    case invalid(String)
}

/// Splits the text into addresses and domains, dropping blanks and duplicates (first one wins, order kept).
func parseParties(_ text: String) -> (addresses: [String], domains: [String]) {
    let separators: Set<Character> = [",", ";", " ", "\n", "\t"]
    var seen = Set<String>()
    let tokens = text.split(whereSeparator: { separators.contains($0) })
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
        .filter { !$0.isEmpty && seen.insert($0).inserted }
    let domains = tokens.filter { $0.hasPrefix("@") || !$0.contains("@") }.map { $0.hasPrefix("@") ? String($0.dropFirst()) : $0 }
    let addresses = tokens.filter { !$0.hasPrefix("@") && $0.contains("@") }
    return (addresses, domains)
}

/// Turns the form into what the core takes, or says what is missing. Never guesses.
func buildNewGrant(_ draft: NewGrantDraft) -> NewGrantResult {
    guard let connection = draft.connectionId else { return .invalid("Choose which AI this is for.") }
    guard let account = draft.account else { return .invalid("Choose which Gmail account this is for.") }
    let (addresses, domains) = parseParties(draft.partiesText)
    let trimmed = draft.subject.trimmingCharacters(in: .whitespacesAndNewlines)
    let subject: String? = trimmed.isEmpty ? nil : trimmed
    if draft.anyMail && draft.send { return .invalid("Sending cannot be allowed for everyone.") }
    if draft.anyMail {
        guard let s = draft.lifetime.seconds, s <= NewGrantLifetime.week.seconds ?? 0 else {
            return .invalid("Allowing all mail needs a time limit of 1 hour, 24 hours or 7 days.")
        }
    } else if draft.send && addresses.isEmpty && domains.isEmpty {
        return .invalid("Enter who it may send to: addresses like a@b.com or domains like @b.com.")
    } else if !draft.send && addresses.isEmpty && domains.isEmpty && subject == nil {
        return .invalid("Enter which senders or which subject text it covers, or choose all mail.")
    }
    let readsFrom = !draft.send && !draft.anyMail
    let scope = GrantScopeChoice(
        allMail: draft.anyMail,
        selectedMessagesOnly: false,
        senderAddresses: readsFrom ? addresses : [],
        senderDomains: readsFrom ? domains : [],
        subjectPattern: draft.anyMail ? nil : subject,
        recipientAddresses: draft.send ? addresses : [],
        recipientDomains: draft.send ? domains : [],
        resources: [],
        classes: []
    )
    let standing = StandingGrant(
        durationSecs: draft.lifetime.seconds.map { UInt64($0) },
        maxUses: draft.lifetime == .oneTime ? 1 : nil,
        scope: scope
    )
    return .ok(connectionId: connection, account: account, kind: draft.send ? .send : .read, standing: standing)
}
