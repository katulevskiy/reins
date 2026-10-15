import SwiftUI

/// Hold to confirm, for what cannot be taken back (Bypass, Disconnect, Delete account, an irreversible approval; the
/// Android app's `HoldButton`): the button has to be held for `duration` (1.2 s). While held, `danger-fill-2` sweeps
/// across its `danger-wash`, a ring fills beside "Keep holding", and haptic ticks close from 260 ms to 70 ms apart; at the
/// end a heavy thud and the success sound, then `onConfirm`. Letting go early drains the fill in 180 ms, quietly. The
/// hold starts once the finger has stayed a moment (DelayedPress), so a scroll that starts on the button scrolls.
///
/// The fill is measured in elapsed time, not animated, so Reduce Motion never shortens or skips it. VoiceOver's
/// activation confirms at once: its deliberate double tap is the hold.
struct HoldButton: View {
    var title: String
    var tint: Color = Palette.danger
    var duration: Double = HoldButton.defaultDuration
    var symbol: String? = nil
    var enabled = true
    var busy = false
    var height: CGFloat = 54
    /// Shown once confirmed, solid in the tint with a check ("Deleted"); nil keeps the button as it was.
    var doneTitle: String? = nil
    /// A still of the button part-held (the Signal gallery); nil in the app.
    var still: Double? = nil
    var onConfirm: () -> Void

    /// Signal's hold: 1.2 s, the same everywhere.
    static let defaultDuration: Double = Motion.holdDuration
    /// What to say under a hold button.
    static let hint = "Hold for a second"

    init(
        title: String, tint: Color = Palette.danger, duration: Double = HoldButton.defaultDuration, symbol: String? = nil,
        enabled: Bool = true, busy: Bool = false, height: CGFloat = 54, doneTitle: String? = nil, still: Double? = nil,
        onConfirm: @escaping () -> Void
    ) {
        self.title = title
        self.tint = tint
        self.duration = duration
        self.symbol = symbol
        self.enabled = enabled
        self.busy = busy
        self.height = height
        self.doneTitle = doneTitle
        self.still = still
        self.onConfirm = onConfirm
    }

    @Environment(\.feedback) private var feedback
    @State private var started: Date?
    /// The fill when the finger lifted, and when: it drains from there.
    @State private var released: (at: Date, from: Double)?
    @State private var done = false
    @State private var ticker: Task<Void, Never>?
    /// Where the hold began, while the finger is down.
    @State private var pressedAt: CGPoint?

    private var active: Bool { enabled && !busy && !done }
    private var holding: Bool { started != nil }
    private var danger: Bool { tint == Palette.danger }
    private var wash: Color { danger ? Palette.dangerWash : tint.opacity(0.14) }
    private var fill: Color { danger ? Palette.dangerFill2 : tint.opacity(0.28) }

    var body: some View {
        TimelineView(.animation(paused: !holding && released == nil)) { context in
            let progress = still ?? progress(at: context.date)
            ZStack(alignment: .leading) {
                if done {
                    tint
                } else {
                    wash
                    GeometryReader { geo in
                        Rectangle().fill(fill).frame(width: geo.size.width * progress)
                    }
                }
                label(progress, held: holding || still != nil)
                    .frame(maxWidth: .infinity)
            }
        }
        .frame(height: height)
        .frame(maxWidth: .infinity)
        .clipShape(Capsule())
        .contentShape(Capsule())
        .opacity(enabled ? 1 : 0.4)
        // Hold buttons sit in scrolling sheets and pages. The hold begins once the finger has stayed a moment, as a
        // button's press does in a scroll view (DelayedPress): a finger that moves at once scrolls, with no fill or tick.
        // A long press here would start the fill at the touch and could keep the page from scrolling.
        .gesture(DelayedPress(space: .local, onPress: { point in
            pressedAt = point
            begin()
        }, onSlide: { point in
            // Wandering off lets go, as far as a long press allowed.
            guard let from = pressedAt, hypot(point.x - from.x, point.y - from.y) > 40 else { return }
            pressedAt = nil
            end()
        }, onRelease: { _, held in
            pressedAt = nil
            // A tap shows what a hold would do and drains at once.
            if !held { begin() }
            end()
        }, onCancel: {
            pressedAt = nil
            end()
        }))
        .allowsHitTesting(active)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(holding ? "Keep holding" : "")
        .accessibilityHint("Hold to confirm")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            guard active else { return }
            feedback.play(.holdConfirmed)
            confirmed()
        }
        .onDisappear { ticker?.cancel() }
    }

    @ViewBuilder
    private func label(_ progress: Double, held: Bool) -> some View {
        HStack(spacing: 8) {
            if done {
                GlyphIcon(.check, size: 20)
            } else if busy {
                ProgressView().tint(tint)
            } else if held {
                Circle()
                    .stroke(fill, lineWidth: 2.4)
                    .overlay {
                        Circle()
                            .trim(from: 0, to: progress)
                            .stroke(tint, style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
                            .rotationEffect(.degrees(-90))
                    }
                    .frame(width: 17, height: 17)
            } else if let symbol {
                Image(systemName: symbol).font(.system(size: 16, weight: .semibold))
            }
            Text(done ? (doneTitle ?? title) : held ? "Keep holding" : title).lineLimit(1)
        }
        .textRole(.button)
        .foregroundStyle(done ? Palette.onField : tint)
    }

    /// How full the button is at `date`: the time held over the duration, or, after letting go, draining linearly.
    private func progress(at date: Date) -> Double {
        if let started { return min(max(date.timeIntervalSince(started) / duration, 0), 1) }
        if let released {
            return max(released.from * (1 - date.timeIntervalSince(released.at) / Motion.holdDrain), 0)
        }
        return 0
    }

    private func begin() {
        guard active, !holding else { return }
        started = .now
        released = nil
        ticker = Task { @MainActor in
            var elapsed = 0.0
            var step = 0
            while elapsed < duration, !Task.isCancelled {
                feedback.play(.holdTick, step: step)
                step += 1
                let gap = HoldButton.tickGap(elapsed / duration)
                try? await Task.sleep(for: .seconds(gap))
                elapsed += gap
            }
            // Held to the end with the finger still down: confirmed.
            guard !Task.isCancelled, let started = self.started else { return }
            let left = duration - Date.now.timeIntervalSince(started)
            if left > 0 { try? await Task.sleep(for: .seconds(left)) }
            if !Task.isCancelled { finish() }
        }
    }

    /// The finger lifted (or the gesture ended): before the end, the fill drains with no haptic; at the end, it is the
    /// confirmation.
    private func end() {
        guard let started else { return }
        let held = Date.now.timeIntervalSince(started)
        if held >= duration * 0.97 {
            finish()
            return
        }
        ticker?.cancel()
        self.started = nil
        released = (Date.now, min(held / duration, 1))
        feedback.play(.holdReleased)
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(Motion.holdDrain))
            if self.started == nil { released = nil }
        }
    }

    private func finish() {
        guard holding else { return }
        ticker?.cancel()
        started = nil
        released = nil
        feedback.play(.holdConfirmed)
        confirmed()
    }

    private func confirmed() {
        if doneTitle != nil { done = true }
        onConfirm()
    }

    /// The pause after a tick at `fraction` of the hold: 0.26 s at the start, 0.07 s at the end.
    static func tickGap(_ fraction: Double) -> Double {
        0.26 - 0.19 * min(max(fraction, 0), 1)
    }
}
