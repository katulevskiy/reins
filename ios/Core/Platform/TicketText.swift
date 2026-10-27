import Foundation

/// A ticket (a request the agent no longer waits for, under "Waiting for you") in words (pure, unit-tested). The app
/// and the notification extension say the same.
enum TicketText {
    private static let minute: Int64 = 60
    private static let hour: Int64 = 3_600
    private static let day: Int64 = 86_400

    /// Under this much time left, a ticket is urgent: its arrival rings (time-sensitive) instead of arriving quietly.
    static let urgentWithin: Int64 = hour

    /// "Due in 45 min", "Due in 3 h", "Due in 2 d", "Overdue". Rounded down, so it never says more time is left than is.
    static func due(deadline: Int64, now: Int64) -> String {
        let left = deadline - now
        switch left {
        case ...0: return "Overdue"
        case ..<hour: return "Due in \(max(left / minute, 1)) min"
        case ..<(2 * day): return "Due in \(left / hour) h"
        default: return "Due in \(left / day) d"
        }
    }

    /// Less than an hour left (or none).
    static func urgent(deadline: Int64, now: Int64) -> Bool { deadline - now < urgentWithin }

    /// The ticket's state in a word, when it says more than "it waits": the reviewer asked Why?, the agent answered,
    /// the world moved since it was approved.
    static func state(_ t: TicketView) -> String? {
        if t.stale != nil || t.state == "stale" { return "Stale" }
        if t.agentReply != nil { return "Answered" }
        if t.question != nil || t.state == "needs_info" { return "Why?" }
        return nil
    }

    /// The agent's answer to a Why?, cleaned and labelled as the agent's: never a fact Reins states.
    static func agentSays(_ t: TicketView) -> String? {
        guard let reply = t.agentReply else { return nil }
        let clean = untrusted(reply).trimmingCharacters(in: .whitespacesAndNewlines)
        return clean.isEmpty ? nil : clean
    }

    /// What changed since it was approved ("main moved from 1a2b3c4 to 5d6e7f8"), cleaned.
    static func stale(_ t: TicketView) -> String? {
        guard let stale = t.stale else { return nil }
        let clean = untrusted(stale).trimmingCharacters(in: .whitespacesAndNewlines)
        return clean.isEmpty ? nil : clean
    }

    /// The label above the agent's answer.
    static let agentSaysLabel = "Agent says"
}
