import SwiftUI

// What a grant's clock says (pure, unit-tested), and the pie and meter that show it (the Android app's
// design/GrantTime.kt).

private let minute: Int64 = 60
private let hour: Int64 = 3_600
private let day: Int64 = 86_400
private let week: Int64 = 7 * day
private let year: Int64 = 365 * day

/// "47m", "3h", "5d", "10w", "2y": the time left in its largest unit (seconds only for the last minute).
func compactDuration(_ seconds: Int64) -> String {
    let s = max(seconds, 0)
    switch s {
    case ..<minute: return "\(s)s"
    case ..<hour: return "\(s / minute)m"
    case ..<day: return "\(s / hour)h"
    case ..<week: return "\(s / day)d"
    case ..<year: return "\(s / week)w"
    default: return "\(s / year)y"
    }
}

/// How long before it ends a grant counts as ending soon (and the user is reminded): a tenth of its life, 5 min to 1 h.
func expiryLeadSeconds(createdAt: Int64, expiresAt: Int64) -> Int64 {
    min(max((expiresAt - createdAt) / 10, 5 * minute), hour)
}

/// The moment the reminder is due, or nil for a grant that does not run out on its own.
func reminderAt(_ grant: GrantView) -> Int64? {
    grant.expiresAt.map { $0 - expiryLeadSeconds(createdAt: grant.createdAt, expiresAt: $0) }
}

/// What is left of a grant's time at a given moment.
struct GrantClock: Equatable {
    /// Seconds left; nil when it never expires on its own.
    var remaining: Int64?
    /// 1 when it has just begun, 0 when it ends; 1 for a grant without an end.
    var fraction: Double
    var soon: Bool

    var label: String { remaining.map(compactDuration) ?? "\u{221E}" }
}

func grantClock(_ grant: GrantView, now: Int64) -> GrantClock {
    guard let end = grant.expiresAt else { return GrantClock(remaining: nil, fraction: 1, soon: false) }
    let remaining = max(end - now, 0)
    let total = max(end - grant.createdAt, 1)
    return GrantClock(
        remaining: remaining,
        fraction: min(max(Double(remaining) / Double(total), 0), 1),
        soon: grant.active && remaining <= expiryLeadSeconds(createdAt: grant.createdAt, expiresAt: end)
    )
}

/// The colour of a running grant's clock: calm, or the warning colour once it is about to end.
func clockColor(_ clock: GrantClock) -> Color { clock.soon ? Palette.danger : Palette.violet }

/// Unix seconds now.
func nowSeconds(_ date: Date = Date()) -> Int64 { Int64(date.timeIntervalSince1970) }

// MARK: Words

/// How grants and their times read (the Android app's `Format.kt` and the grant rows' words).
enum GrantText {
    /// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", else the date.
    static func relative(_ epoch: Int64, now: Int64 = nowSeconds()) -> String {
        let d = now - epoch
        switch d {
        case ..<45: return "just now"
        case ..<3_600: return "\(max(1, d / 60)) min ago"
        case ..<86_400: return "\(d / 3_600) h ago"
        case ..<(2 * 86_400): return "yesterday"
        case ..<(7 * 86_400): return "\(d / 86_400) days ago"
        default: return Date(timeIntervalSince1970: TimeInterval(epoch)).formatted(date: .abbreviated, time: .omitted)
        }
    }

    /// Date and time, spelled out.
    static func full(_ epoch: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(epoch)).formatted(date: .long, time: .shortened)
    }

    /// "expires in 2 h" style remaining time.
    static func expiry(_ expiresAt: Int64?, now: Int64 = nowSeconds()) -> String {
        guard let expiresAt else { return "no time limit" }
        let left = expiresAt - now
        switch left {
        case ...0: return "expired"
        case ..<3_600: return "expires in \(max(1, left / 60)) min"
        case ..<86_400: return "expires in \(left / 3_600) h"
        default: return "expires in \(left / 86_400) days"
        }
    }

    /// How a finished grant ended: "Expired 2 days ago", "All 5 uses spent", "Deleted".
    static func endedLine(_ g: GrantView, now: Int64 = nowSeconds()) -> String {
        switch g.state {
        case "revoked": return "Deleted"
        case "used_up": return "All \(g.maxUses ?? g.uses) uses spent"
        default: return g.expiresAt.map { "Expired \(relative($0, now: now))" } ?? "Expired"
        }
    }

    static func inactiveWord(_ g: GrantView) -> String {
        switch g.state {
        case "revoked": "Deleted"
        case "used_up": "Used up"
        default: "Expired"
        }
    }

    /// The verb on a running grant's row ("Reads").
    static func verb(_ action: String) -> String {
        switch action {
        case "send": "Sends"
        case "write": "Writes"
        case "list": "Lists"
        case "accounts": "Shows accounts"
        case "headlines": "Headlines"
        default: "Reads"
        }
    }

    /// The same, on the grant's own page ("Reading").
    static func gerund(_ action: String) -> String {
        switch action {
        case "send": "Sending"
        case "write": "Writing"
        case "list": "Listing"
        case "accounts": "Showing accounts"
        case "headlines": "Showing headlines"
        default: "Reading"
        }
    }

    /// Where the grant came from.
    static func origin(_ origin: String, label: String) -> String {
        switch origin {
        case "ai_request": "\(untrusted(label)) asked and you allowed it"
        case "user": "Created by you in advance"
        case "retry": "A one-time pass after you approved late"
        case "starter": "Your starting rule, when you connected \(untrusted(label))"
        case let o where o == "bypass" || o.hasPrefix("bypass:"): "Approved by Bypass; ends with it"
        case let o where o == "overnight" || o.hasPrefix("overnight:"): "Approved by \(AutopilotText.overnightName); ends with it"
        default: "Chosen while approving a request"
        }
    }

    /// "Not used yet", "Used once", "Used 4 times".
    static func usedTimes(_ uses: Int) -> String {
        switch uses {
        case 0: "Not used yet"
        case 1: "Used once"
        default: "Used \(uses) times"
        }
    }

    /// A running grant's line under its summary: "3 of 10 uses", "used 12 times", "not used yet".
    static func usesLine(_ g: GrantView) -> String {
        if let cap = g.maxUses { return "\(g.uses) of \(cap) uses" }
        switch g.uses {
        case 0: return "not used yet"
        case 1: return "used once"
        default: return "used \(g.uses) times"
        }
    }

    /// The Uses row on a grant's page: "3 of 10", "Used 12 times".
    static func usesValue(_ g: GrantView) -> String {
        g.maxUses.map { "\(g.uses) of \($0)" } ?? usedTimes(Int(g.uses))
    }

    /// The big ring's number: "5 h", "47 m", "6 d", "∞".
    static func ringLabel(_ clock: GrantClock) -> String {
        guard let r = clock.remaining else { return "\u{221E}" }
        let compact = compactDuration(r)
        guard let unit = compact.last, unit.isLetter else { return compact }
        return "\(compact.dropLast()) \(unit)"
    }

    /// What a grant never covers, where that is known: a send grant never reads, a read grant never sends or changes
    /// anything, and codes and passwords always ask.
    static func never(_ g: GrantView) -> [String] {
        let mail = g.service.isEmpty || g.service == "gmail"
        switch g.action {
        case "send": return [mail ? "Read your mail" : "Read anything"]
        case "read", "search", "list": return ["Send or change anything", "Codes and passwords"]
        case "headlines": return ["Open anything", "Send or change anything"]
        default: return []
        }
    }

    /// "2 left", "All used", "9 of 40": what a meter with a cap says.
    static func usesLeft(_ uses: Int, of maxUses: Int) -> String {
        maxUses <= 8 ? (uses >= maxUses ? "All used" : "\(maxUses - uses) left") : "\(uses) of \(maxUses)"
    }
}

// MARK: Visuals

/// The part of a circle that remains: from `-90 + 360 * (1 - fraction)` degrees clockwise to 12 o'clock.
private struct PieWedge: Shape {
    var fraction: Double

    var animatableData: Double {
        get { fraction }
        set { fraction = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let radius = min(rect.width, rect.height) / 2
        var p = Path()
        if fraction >= 1 {
            p.addEllipse(in: rect)
            return p
        }
        p.move(to: center)
        // SwiftUI's y axis points down, so `clockwise: false` runs clockwise on screen.
        p.addArc(center: center, radius: radius, startAngle: .degrees(-90 + 360 * (1 - fraction)), endAngle: .degrees(270), clockwise: false)
        p.closeSubpath()
        return p
    }
}
