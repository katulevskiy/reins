import Foundation

/// For how long an ended grant can be started again.
enum ResumePeriod: CaseIterable, Hashable {
    case hour, day, week, month

    var label: String {
        switch self {
        case .hour: "1 hour"
        case .day: "24 hours"
        case .week: "7 days"
        case .month: "30 days"
        }
    }

    var seconds: Int64 {
        switch self {
        case .hour: 3_600
        case .day: 86_400
        case .week: 604_800
        case .month: 2_592_000
        }
    }
}

/// A grant for all mail can be resumed for a week at most, like it can be created.
func resumePeriods(allMail: Bool) -> [ResumePeriod] {
    allMail ? [.hour, .day, .week] : ResumePeriod.allCases
}

/// The units a custom time can be given in.
enum CustomUnit: CaseIterable, Hashable {
    case minutes, hours, days

    var label: String {
        switch self {
        case .minutes: "minutes"
        case .hours: "hours"
        case .days: "days"
        }
    }

    var seconds: Int64 {
        switch self {
        case .minutes: 60
        case .hours: 3_600
        case .days: 86_400
        }
    }
}

private let minResumeSeconds: Int64 = 60
private let maxResumeSeconds: Int64 = 30 * 86_400
private let maxAllMailSeconds: Int64 = 7 * 86_400
let maxResumeUses = 1000

/// Everything the resume sheet lets the user change.
struct ResumeDraft: Equatable {
    /// One of the quick choices; ignored when a custom time is typed.
    var period: ResumePeriod = .day
    var customAmount = ""
    var customUnit: CustomUnit = .hours
    var moreOpen = false
    var limitUses = false
    var uses = "5"
    var allMail = false
    /// Addresses (`a@b.com`) and domains (`@b.com`): senders for a read grant, recipients for a send grant.
    var partiesText = ""
    var subject = ""
}

enum ResumeResult: Equatable {
    /// `standing` is set only when the user changed who or what it covers, or the number of uses.
    case ok(seconds: Int64, standing: StandingGrant?)
    case invalid(String)
}

/// The sheet as it opens: the grant exactly as it was, for a day.
func initialResumeDraft(_ grant: GrantView) -> ResumeDraft {
    let scope = grant.editableScope
    var draft = ResumeDraft()
    draft.limitUses = grant.maxUses != nil
    draft.uses = String(grant.maxUses ?? 5)
    draft.allMail = scope?.allMail == true
    draft.partiesText = scope.map { parties(grant, $0) } ?? ""
    draft.subject = scope?.subjectPattern ?? ""
    return draft
}

private func parties(_ grant: GrantView, _ scope: GrantScopeChoice) -> String {
    let send = grant.action == "send"
    let addresses = send ? scope.recipientAddresses : scope.senderAddresses
    let domains = send ? scope.recipientDomains : scope.senderDomains
    return (addresses + domains.map { "@\($0)" }).joined(separator: ", ")
}

/// How long the draft asks for: the typed time if there is one, else the quick choice.
func resumeSeconds(_ draft: ResumeDraft) -> Int64? {
    let typed = draft.customAmount.trimmingCharacters(in: .whitespacesAndNewlines)
    if typed.isEmpty { return draft.period.seconds }
    return Int64(typed).map { $0 * draft.customUnit.seconds }
}

/// Turns the sheet into what the core takes, or says what is wrong. Never guesses.
func buildResume(_ grant: GrantView, _ draft: ResumeDraft) -> ResumeResult {
    guard let seconds = resumeSeconds(draft) else { return .invalid("Enter the time as a whole number.") }
    let editable = grant.editableScope
    let allMail = editable != nil && draft.allMail
    if seconds < minResumeSeconds { return .invalid("Choose at least a minute.") }
    if seconds > maxResumeSeconds { return .invalid("Choose 30 days at most.") }
    if allMail && seconds > maxAllMailSeconds { return .invalid("Access to all mail can last 7 days at most.") }
    var maxUses: UInt32?
    if draft.limitUses {
        guard let n = Int(draft.uses.trimmingCharacters(in: .whitespacesAndNewlines)), (1...maxResumeUses).contains(n) else {
            return .invalid("Uses must be between 1 and \(maxResumeUses).")
        }
        maxUses = UInt32(n)
    }
    let usesChanged = maxUses != grant.maxUses

    guard let editable else { return .ok(seconds: seconds, standing: nil) }
    let send = grant.action == "send"
    let (addresses, domains) = parseParties(draft.partiesText)
    let trimmed = draft.subject.trimmingCharacters(in: .whitespacesAndNewlines)
    let subject: String? = trimmed.isEmpty ? nil : trimmed
    if !allMail {
        if send && addresses.isEmpty && domains.isEmpty {
            return .invalid("Enter who it may send to: addresses like a@b.com or domains like @b.com.")
        }
        if !send && addresses.isEmpty && domains.isEmpty && subject == nil {
            return .invalid("Enter which senders or which subject text it covers, or choose all mail.")
        }
    }
    let readsFrom = !send && !allMail
    let scope = GrantScopeChoice(
        allMail: allMail,
        selectedMessagesOnly: false,
        senderAddresses: readsFrom ? addresses : [],
        senderDomains: readsFrom ? domains : [],
        subjectPattern: allMail ? nil : subject,
        recipientAddresses: send ? addresses : [],
        recipientDomains: send ? domains : [],
        resources: [],
        classes: []
    )
    let changed = !sameScope(scope, editable) || usesChanged
    return .ok(seconds: seconds, standing: changed ? StandingGrant(durationSecs: UInt64(seconds), maxUses: maxUses, scope: scope) : nil)
}

private func sameScope(_ a: GrantScopeChoice, _ b: GrantScopeChoice) -> Bool {
    func trim(_ s: String?) -> String { (s ?? "").trimmingCharacters(in: .whitespacesAndNewlines) }
    return a.allMail == b.allMail
        && a.senderAddresses.sorted() == b.senderAddresses.sorted()
        && a.senderDomains.sorted() == b.senderDomains.sorted()
        && a.recipientAddresses.sorted() == b.recipientAddresses.sorted()
        && a.recipientDomains.sorted() == b.recipientDomains.sorted()
        && trim(a.subjectPattern) == trim(b.subjectPattern)
}
