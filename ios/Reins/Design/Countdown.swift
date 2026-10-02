import SwiftUI

/// The clock the countdowns read. Tests and screenshots freeze it (`-freezeTime <unix seconds>`), so a waiting
/// request always shows the same time left.
enum Timers {
    /// A fixed "now" in unix seconds; nil = the real clock.
    nonisolated(unsafe) static var frozenNow: TimeInterval? = {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-freezeTime"), i + 1 < args.count else { return nil }
        return TimeInterval(args[i + 1])
    }()

    static var live: Bool { frozenNow == nil }

    /// Now, in unix seconds (fractional).
    static func now(_ date: Date = Date()) -> TimeInterval { frozenNow ?? date.timeIntervalSince1970 }

    /// Now, in whole unix seconds.
    static func nowSeconds() -> Int64 { Int64(now()) }
}

/// With this many seconds left, the countdown turns red.
let urgentSeconds: Int64 = 15

/// How urgent a request is, from when the AI stops waiting.
struct Urgency: Equatable {
    var remainingSeconds: Int64
    /// What is left of the wait, 1 when it arrives, 0 when it is over.
    var fraction: Double
    /// The AI stopped waiting (approving still works; it has to ask again).
    var stale: Bool
    var urgent: Bool

    /// Nil when nothing waits on the answer (an upload, a request without a deadline).
    static func of(createdAt: Int64, waitUntil: Int64?, now: TimeInterval) -> Urgency? {
        guard let waitUntil else { return nil }
        let total = Double(max(waitUntil - createdAt, 1))
        let remaining = Double(waitUntil) - now
        let fraction = min(max(remaining / total, 0), 1)
        let stale = remaining <= 0
        return Urgency(
            remainingSeconds: max(Int64(remaining), 0),
            fraction: fraction,
            stale: stale,
            urgent: !stale && remaining <= Double(urgentSeconds)
        )
    }
}

/// Redraws `content` with the current time while timers are live: four times a second by default, which keeps a
/// seconds countdown smooth without redrawing more than it needs.
struct LiveClock<Content: View>: View {
    var interval: TimeInterval = 0.25
    @ViewBuilder var content: (TimeInterval) -> Content

    var body: some View {
        if Timers.live {
            TimelineView(.periodic(from: .now, by: interval)) { context in
                content(Timers.now(context.date))
            }
        } else {
            content(Timers.now())
        }
    }
}

/// The outline of a rounded rectangle that starts at the top-left corner and runs clockwise, so trimming it to
/// `0...fraction` leaves a bar that empties back towards the corner it started from.
struct TimeBarOutline: Shape {
    var corner: CGFloat
    var inset: CGFloat

    func path(in rect: CGRect) -> Path {
        let r = rect.insetBy(dx: inset, dy: inset)
        let c = min(max(corner - inset, 0), min(r.width, r.height) / 2)
        var p = Path()
        p.move(to: CGPoint(x: r.minX, y: r.minY + c))
        p.addArc(center: CGPoint(x: r.minX + c, y: r.minY + c), radius: c, startAngle: .degrees(180), endAngle: .degrees(270), clockwise: false)
        p.addLine(to: CGPoint(x: r.maxX - c, y: r.minY))
        p.addArc(center: CGPoint(x: r.maxX - c, y: r.minY + c), radius: c, startAngle: .degrees(270), endAngle: .degrees(0), clockwise: false)
        p.addLine(to: CGPoint(x: r.maxX, y: r.maxY - c))
        p.addArc(center: CGPoint(x: r.maxX - c, y: r.maxY - c), radius: c, startAngle: .degrees(0), endAngle: .degrees(90), clockwise: false)
        p.addLine(to: CGPoint(x: r.minX + c, y: r.maxY))
        p.addArc(center: CGPoint(x: r.minX + c, y: r.maxY - c), radius: c, startAngle: .degrees(90), endAngle: .degrees(180), clockwise: false)
        p.closeSubpath()
        return p
    }
}

extension View {
    /// A 2 pt border that is a progress bar round the edge: `fraction` of it in `color`, the rest the hairline track.
    func timeBar(fraction: Double, color: Color, corner: CGFloat = 18) -> some View {
        overlay {
            ZStack {
                RoundedRectangle(cornerRadius: corner, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 2)
                if fraction > 0 {
                    TimeBarOutline(corner: corner, inset: 1)
                        .trim(from: 0, to: min(fraction, 1))
                        .stroke(color, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                }
            }
            .allowsHitTesting(false)
        }
    }
}

/// The frame of a request that waits for the user: full in the accent colour when it arrives, emptying as the AI's
/// patience runs out, red for the last `urgentSeconds`, grey once the AI stopped waiting.
struct CountdownFrame<Content: View>: View {
    var createdAt: Int64
    var waitUntil: Int64?
    var corner: CGFloat = 18
    @ViewBuilder var content: (Urgency?) -> Content

    var body: some View {
        LiveClock { now in
            let u = Urgency.of(createdAt: createdAt, waitUntil: waitUntil, now: now)
            content(u)
                .timeBar(fraction: u.map { $0.stale ? 0 : $0.fraction } ?? 1, color: Self.color(u), corner: corner)
                .animation(.easeInOut(duration: 0.4), value: u?.urgent)
        }
    }

    static func color(_ u: Urgency?) -> Color {
        guard let u else { return Palette.accent }
        if u.stale { return Palette.tertiary }
        return u.urgent ? Palette.danger : Palette.accent
    }
}
