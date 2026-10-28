import Foundation

/// Personal guardianship (decisions v2, 04 §4) in words and rules, pure and unit-tested like `CircleLogic` (the Android
/// app's guardian helpers in `CircleLogic.kt` say the same): the notice the ward picks up front, how leaving stands,
/// what a row says while it ends, the ward-only weekly summary, the guardian's notices and their one notification.
///
/// Personal guardianship is cooperative (08 R7): the ward's phone holds to it and the guardian is told when it ends
/// early. Nothing here claims more.
enum GuardianLogic {
    // MARK: Who is who

    /// This account is the ward: the other person is its guardian (the ward owns the arrangement).
    static func isWard(_ p: CirclePerson) -> Bool { p.role == .guardian && p.mine }

    /// This account is the other person's guardian.
    static func isGuardian(_ p: CirclePerson) -> Bool { p.role == .guardian && !p.mine }

    // MARK: The notice (chosen by the ward, shown before the code)

    /// The notices a ward picks from: 0, 3 or 7 days, never more, never "permanent".
    static let noticeChoices: [UInt8] = [0, 3, 7]

    /// The chooser's segment: "None", "3 days", "7 days".
    static func noticeLabel(_ days: UInt8) -> String { days == 0 ? "None" : days == 1 ? "1 day" : "\(days) days" }

    /// "Ends after 3 days' notice"; "Ends right away" with none.
    static func noticeLine(_ days: UInt8) -> String {
        switch days {
        case 0: "Ends right away"
        case 1: "Ends after 1 day's notice"
        default: "Ends after \(days) days' notice"
        }
    }

    /// What the (?) beside the chooser says.
    static let noticeHelp = "If you leave, it ends after this notice and your guardian is told. 7 days at most: you can always leave."

    /// What the Add screen sends. A guardianship carries the ward's notice; the other roles never do.
    static func draft(role: CircleRole, preset: CircleScopePreset, deferred: Bool, noticeDays: UInt8?) -> CircleDraft {
        CircleLogic.draft(role: role, preset: preset, deferred: deferred, noticeDays: noticeDays)
    }

    /// The link can be made once the ward picked a notice (Guardian), at once for the other roles. No age signal is
    /// ever needed for this.
    static func canMakeLink(role: CircleRole, noticeDays: UInt8?) -> Bool {
        role != .guardian || noticeDays.map(noticeChoices.contains) == true
    }

    // MARK: Leaving (the ward's side)

    enum Leave: Equatable {
        /// Running: Leave starts the notice.
        case available(noticeDays: UInt8)
        /// Asked: it ends then; the obligations stay until.
        case ending(at: Int64)
        /// A minor's account: the guardian's consent ends it.
        case waitingForConsent
    }

    static func leave(_ w: WardView) -> Leave {
        if w.needsConsent { return .waitingForConsent }
        if let at = w.ending { return .ending(at: at) }
        return .available(noticeDays: w.noticeDays)
    }

    /// What Leave's confirmation says under its title.
    static func leaveMessage(_ w: WardView) -> String {
        if w.minor { return "\(CircleLogic.name(w.guardian)) is asked to agree." }
        return "\(noticeLine(w.noticeDays)). \(CircleLogic.name(w.guardian)) is told."
    }

    /// "Ends in 2 d 4 h", "Ends in 5 h", "Ends in 12 min"; "Ending" once due.
    static func endingLine(_ ending: Int64, now: Int64) -> String {
        let left = ending - now
        switch left {
        case ...0: return "Ending"
        case ..<3_600: return "Ends in \(max((left + 59) / 60, 1)) min"
        case ..<86_400: return "Ends in \(left / 3_600) h"
        default:
            let hours = (left % 86_400) / 3_600
            return "Ends in \(left / 86_400) d" + (hours > 0 ? " \(hours) h" : "")
        }
    }

    /// "Waiting for Mom": a minor's request to leave, until the guardian agrees.
    static func waitingFor(_ guardian: String) -> String { "Waiting for \(CircleLogic.name(guardian))" }

    /// What a guardianship's row says while it ends: the countdown, or who it waits for. Nil otherwise.
    static func badge(_ p: CirclePerson, now: Int64) -> String? {
        guard p.role == .guardian, p.state != .ended else { return nil }
        if p.needsConsent { return p.mine ? waitingFor(p.peer) : "Asked to leave" }
        return p.ending.map { endingLine($0, now: now) }
    }

    // MARK: The minor mark

    /// The guardian may ask to mark the ward's account a minor's while it runs (the ward accepts or not).
    static func canProposeMinor(_ p: CirclePerson) -> Bool { isGuardian(p) && p.state == .active }

    /// The ward's card: the guardian asked, and it isn't marked yet.
    static func showsMinorCard(_ w: WardView) -> Bool { w.minorProposed && !w.minor }

    // MARK: The weekly summary (opt-in by the ward)

    /// Only the ward has the switch: the guardian never turns it on.
    static func showsDigestSwitch(_ p: CirclePerson) -> Bool { isWard(p) && p.state != .ended }

    /// "Weekly summary for Mom".
    static func digestLabel(_ guardian: String) -> String { "Weekly summary for \(CircleLogic.name(guardian))" }

    static let digestHelp = "Once a week: how many automatic decisions in scope, one line each. No messages, no names."

    // MARK: The guardian's notices

    /// Under a notice this phone could not check.
    static let reportedByServer = "Reported by the server"

    /// Nil when this phone checked it; else "Reported by the server".
    static func source(_ n: GuardianNoticeView) -> String? { n.verified ? nil : reportedByServer }

    /// "Asked to leave", as the core words it (it came from another account, so untrusted).
    static func noticeText(_ n: GuardianNoticeView) -> String {
        let line = untrusted(n.line)
        return line.isEmpty ? "Guardianship changed" : line
    }

    /// "Sam asked to leave. Allow?": the consent card.
    static func consentLine(_ ward: String) -> String { "\(CircleLogic.name(ward)) asked to leave. Allow?" }

    /// The wards whose request to leave waits for this account, by name.
    static func consents(_ people: [CirclePerson]) -> [CirclePerson] {
        people.filter { isGuardian($0) && $0.needsConsent && $0.state != .ended }
            .sorted { CircleLogic.name($0.peer).lowercased() < CircleLogic.name($1.peer).lowercased() }
    }

    /// The notices, newest first, at most `limit`.
    static func notices(_ list: [GuardianNoticeView], limit: Int = 20) -> [GuardianNoticeView] {
        Array(list.sorted { a, b in a.at != b.at ? a.at > b.at : a.id < b.id }.prefix(limit))
    }

    // MARK: The notification

    /// "Sam: guardianship changed": the one guardian notification, titled by the ward's short name.
    static func notificationTitle(_ ward: String) -> String { "\(CircleLogic.name(ward)): guardianship changed" }

    /// Under it: what happened, and whether only the server says so.
    static func notificationBody(_ n: GuardianNoticeView) -> String {
        if n.needsConsent { return consentLine(n.ward) }
        return [noticeText(n), source(n) ?? ""].filter { !$0.isEmpty }.joined(separator: "\n")
    }
}
