import SwiftUI

// Signal's small controls (Components.png: Mode chip, Controls, Rows): the mode chip in every header, pills that always
// carry a word, the switch, the check box, the radio, the stepper and the filter chip.

/// Autopilot's mode in a header: Manual on raised paper, Auto and Overnight in violet, Lockdown amber, Bypass red with
/// its time left and a dot that breathes (35 % to 100 % every 1.8 s; still under Reduce Motion). A trigger for the mode
/// menu shows a chevron.
struct ModeChip: View {
    enum Mode: Equatable {
        case manual, auto, overnight, lockdown
        /// A bypass running, with what is left ("14:05").
        case bypass(left: String)
    }

    var mode: Mode
    var chevron = false

    var body: some View {
        HStack(spacing: 6) {
            switch mode {
            case .bypass(let left):
                BreathingDot()
                Text(left).font(RFont.mono(13.5)).monospacedDigit()
            default:
                GlyphIcon(glyph, size: 16)
            }
            Text(title)
            if chevron { GlyphIcon(.chevronDown, size: 14) }
        }
        .font(RFont.sans(14, .medium))
        .foregroundStyle(mode == .manual ? Palette.ink : .white)
        .padding(.horizontal, 13)
        .frame(height: 34)
        .background(fill, in: Capsule())
        .overlay {
            if mode == .manual { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
        }
        .accessibilityElement(children: .combine)
    }

    var title: String {
        switch mode {
        case .manual: "Manual"
        case .auto: "Auto"
        case .overnight: "Overnight"
        case .lockdown: "Lockdown"
        case .bypass: "Bypass"
        }
    }

    private var glyph: Glyph {
        switch mode {
        case .manual: .person
        case .auto, .bypass: .autopilot
        case .overnight: .moon
        case .lockdown: .lock
        }
    }

    private var fill: Color {
        switch mode {
        case .manual: Palette.paperRaised
        case .auto, .overnight: Palette.violetFill
        case .lockdown: Palette.fieldWarn
        case .bypass: Palette.fieldDelete
        }
    }
}

/// The bypass dot: white, breathing while any bypass runs (tab bar, mode chip).
struct BreathingDot: View {
    var color: Color = .white
    var size: CGFloat = 8
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        TimelineView(.animation(paused: reduceMotion)) { context in
            let phase = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: Motion.breathe) / Motion.breathe
            // Ease in and out between 35 % and 100 %.
            let wave = (1 - cos(phase * 2 * .pi)) / 2
            Circle()
                .fill(color)
                .frame(width: size, height: size)
                .opacity(reduceMotion ? 1 : 1 - 0.65 * wave)
        }
        .accessibilityHidden(true)
    }
}

/// A word on a tint: Auto, New, Ready, Denied, Code. Colour never speaks alone, so a pill always has its word.
struct Pill: View {
    enum Tone {
        case violet, success, danger, warning, neutral
        /// On a field: white with ink text (Review).
        case onField
        /// A kind's ink on its tile.
        case kind(FieldKind)
    }

    var text: String
    var tone: Tone = .neutral
    var glyph: Glyph? = nil

    init(_ text: String, tone: Tone = .neutral, glyph: Glyph? = nil) {
        self.text = text
        self.tone = tone
        self.glyph = glyph
    }

    var body: some View {
        let (ink, fill) = colors
        HStack(spacing: 4) {
            if let glyph { GlyphIcon(glyph, size: 12) }
            Text(text).lineLimit(1)
        }
        .textRole(.tag)
        .foregroundStyle(ink)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(fill, in: Capsule())
        .fixedSize()
    }

    private var colors: (Color, Color) {
        switch tone {
        case .violet: (Palette.violet, Palette.violet.opacity(0.13))
        case .success: (Palette.success, Palette.success.opacity(0.13))
        case .danger: (Palette.danger, Palette.danger.opacity(0.13))
        case .warning: (Palette.warning, Palette.warning.opacity(0.13))
        case .neutral: (Palette.ink2, Palette.paperSunken)
        case .onField: (Palette.onFieldInk, .white)
        case .kind(let kind): (kind.ink, kind.tile)
        }
    }
}

/// The switch: 51 × 31, `violet-fill` when on, `paper-sunken` when off, a white knob. VoiceOver and UI tests see a
/// system switch.
struct SignalSwitchStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        SignalSwitchBody(configuration: configuration)
    }
}

private struct SignalSwitchBody: View {
    var configuration: ToggleStyleConfiguration
    @Environment(\.isEnabled) private var enabled
    @Environment(\.labelsVisibility) private var labels

    var body: some View {
        HStack(spacing: 12) {
            if labels != .hidden {
                configuration.label
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            SignalSwitchTrack(on: configuration.isOn)
        }
        .contentShape(Rectangle())
        .onTapGesture { configuration.isOn.toggle() }
        .opacity(enabled ? 1 : 0.4)
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

/// The track and knob alone (the gallery, rows that draw their own label).
struct SignalSwitchTrack: View {
    var on: Bool

    var body: some View {
        Capsule()
            .fill(on ? Palette.violetFill : Palette.paperSunken)
            .frame(width: 51, height: 31)
            .overlay(alignment: on ? .trailing : .leading) {
                Circle()
                    .fill(.white)
                    .shadow(color: .black.opacity(0.25), radius: 1.5, y: 1)
                    .frame(width: 27, height: 27)
                    .padding(2)
            }
            .animation(.spring(duration: 0.25, bounce: 0.15), value: on)
            .accessibilityHidden(true)
    }
}

/// A check box: 24 pt, rounded 8, violet with a white check when on, a faint ring when off.
struct SignalCheckbox: View {
    var on: Bool

    var body: some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(on ? Palette.violetFill : .clear)
            .overlay {
                if on {
                    GlyphIcon(.check, size: 16).foregroundStyle(.white)
                } else {
                    RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.inkFaint, lineWidth: 1.5)
                }
            }
            .frame(width: 24, height: 24)
            .accessibilityHidden(true)
    }
}

/// − 5 +: a sunken capsule with two round buttons; each step ticks the detent.
struct SignalStepper: View {
    var value: String
    var canDecrease = true
    var canIncrease = true
    var onDecrease: () -> Void
    var onIncrease: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 10) {
            step(.minus, "Fewer", enabled: canDecrease, action: onDecrease)
            Text(value).font(RFont.mono(15)).foregroundStyle(Palette.ink).monospacedDigit()
            step(.plus, "More", enabled: canIncrease, action: onIncrease)
        }
        .padding(.horizontal, 6)
        .frame(height: 40)
        .background(Palette.paperSunken, in: Capsule())
        .accessibilityElement(children: .contain)
    }

    private func step(_ glyph: Glyph, _ label: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button {
            action()
            feedback.play(.detent)
        } label: {
            GlyphIcon(glyph, size: 16)
                .foregroundStyle(Palette.ink)
                .frame(width: 30, height: 30)
                .background(Palette.paperRaised, in: Circle())
        }
        .buttonStyle(PressDimsStyle())
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

/// A filter chip (All, Autopilot, Denied, Grants): raised with a hairline ring, `inverse` when chosen.
struct FilterChip: View {
    var title: String
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(RFont.sans(14, .medium))
                .foregroundStyle(selected ? Palette.onInverse : Palette.ink)
                .padding(.horizontal, 13)
                .frame(height: 34)
                .background(selected ? Palette.inverse : Palette.paperRaised, in: Capsule())
                .overlay {
                    if !selected { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
                }
                .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}
