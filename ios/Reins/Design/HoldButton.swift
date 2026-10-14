import SwiftUI

/// Hold to confirm, for what cannot be taken back (Bypass, Disconnect, Delete account; the Android app's
/// `HoldButton`): the button has to be held for `duration` seconds. While held, a fill sweeps across it, a small ring
/// fills beside "Keep holding", haptic ticks speed up and climb in pitch; on completion a heavy haptic and the success
/// sound, then `onConfirm`. Letting go early cancels, with a soft release cue.
///
/// VoiceOver's activation confirms at once: its deliberate double tap is the hold.
struct HoldButton: View {
    var title: String
    var tint: Color = Palette.danger
    var duration: Double = HoldButton.defaultDuration
    var symbol: String? = nil
    var enabled = true
    var busy = false
    var height: CGFloat = 54
    var onConfirm: () -> Void

    /// The usual hold, about two seconds.
    static let defaultDuration: Double = 2

    init(
        title: String, tint: Color = Palette.danger, duration: Double = HoldButton.defaultDuration, symbol: String? = nil,
        enabled: Bool = true, busy: Bool = false, height: CGFloat = 54, onConfirm: @escaping () -> Void
    ) {
        self.title = title
        self.tint = tint
        self.duration = duration
        self.symbol = symbol
        self.enabled = enabled
        self.busy = busy
        self.height = height
        self.onConfirm = onConfirm
    }

    @Environment(\.feedback) private var feedback
    @State private var progress: Double = 0
    @State private var holding = false
    @State private var started: Date?
    @State private var ticker: Task<Void, Never>?

    private var active: Bool { enabled && !busy }
    /// Dark text on the bright reds of the dark theme; white on the deeper ones of the light theme.
    private var foreground: Color { Palette.dynamic(0xFFFFFF, 0x2A0606) }

    var body: some View {
        ZStack(alignment: .leading) {
            tint
            // The fill that sweeps across while held.
            GeometryReader { geo in
                Rectangle()
                    .fill(Color.black.opacity(0.22))
                    .frame(width: geo.size.width * progress)
            }
            HStack(spacing: 8) {
                if busy {
                    ProgressView().tint(foreground)
                } else if holding {
                    Circle()
                        .stroke(foreground.opacity(0.25), lineWidth: 2.4)
                        .overlay {
                            Circle()
                                .trim(from: 0, to: progress)
                                .stroke(foreground, style: StrokeStyle(lineWidth: 2.4, lineCap: .round))
                                .rotationEffect(.degrees(-90))
                        }
                        .frame(width: 17, height: 17)
                } else if let symbol {
                    Image(systemName: symbol).font(.system(size: 16, weight: .semibold))
                }
                Text(holding ? "Keep holding" : title).lineLimit(1)
            }
            .font(RFont.sans(17, .semibold))
            .foregroundStyle(foreground)
            .frame(maxWidth: .infinity)
        }
        .frame(height: height)
        .frame(maxWidth: .infinity)
        .clipShape(Capsule())
        .contentShape(Capsule())
        .opacity(enabled ? 1 : 0.45)
        .onLongPressGesture(minimumDuration: duration, maximumDistance: 40) {
            finish()
        } onPressingChanged: { pressing in
            if pressing { begin() } else { end() }
        }
        .allowsHitTesting(active)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue(holding ? "Keep holding" : "")
        .accessibilityHint("Hold to confirm")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            guard active else { return }
            feedback.play(.holdConfirmed)
            onConfirm()
        }
        .onDisappear { ticker?.cancel() }
    }

    private func begin() {
        guard active, !holding else { return }
        holding = true
        started = .now
        withAnimation(.linear(duration: duration)) { progress = 1 }
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
        }
    }

    /// The finger lifted (or the gesture ended): before the end, nothing happens; at the end, it is the confirmation.
    private func end() {
        guard holding else { return }
        let held = started.map { Date.now.timeIntervalSince($0) } ?? 0
        if held >= duration * 0.97 {
            finish()
            return
        }
        ticker?.cancel()
        holding = false
        started = nil
        feedback.play(.holdReleased)
        withAnimation(.easeOut(duration: 0.22)) { progress = 0 }
    }

    private func finish() {
        guard holding else { return }
        ticker?.cancel()
        holding = false
        started = nil
        feedback.play(.holdConfirmed)
        onConfirm()
        withAnimation(.easeOut(duration: 0.3)) { progress = 0 }
    }

    /// The pause after a tick at `fraction` of the hold: 0.26 s at the start, 0.07 s at the end.
    static func tickGap(_ fraction: Double) -> Double {
        0.26 - 0.19 * min(max(fraction, 0), 1)
    }
}
