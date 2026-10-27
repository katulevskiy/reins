import SwiftUI

// The kit's small controls (the System board): the mode chip in every header, tags and pills that always carry a word,
// the switch, the check box, the radio, the stepper, the filter chip and the selectable chip.

/// The mode in a header (the System board, Modes): the same colour, glyph and name as the mode's card. Manual sits on
/// raised paper; every other mode is solid in its colour, a timed one with its time left in mono. A trigger for the
/// mode sheet shows a chevron.
struct ModeChip: View {
    enum Mode: Equatable {
        case manual, auto, lockdown
        /// An Overnight running, with what is left ("5:59").
        case overnight(left: String)
        /// A bypass running, with what is left ("47:12").
        case bypass(left: String)

        var kind: ModeKind {
            switch self {
            case .manual: .manual
            case .auto: .auto
            case .overnight: .overnight
            case .lockdown: .lockdown
            case .bypass: .bypass
            }
        }

        var left: String? {
            switch self {
            case .overnight(let left), .bypass(let left): left.isEmpty ? nil : left
            default: nil
            }
        }
    }

    var mode: Mode
    var chevron = false

    var body: some View {
        let kind = mode.kind
        HStack(spacing: 6) {
            ModeGlyph(kind, size: 16)
            // A timed mode running shows its glyph and what is left only, so the header's title keeps its room
            // ("Bypass 54:27" squeezed "Activity" to "Act…"); its name stays in the spoken label.
            if let left = mode.left {
                Text(left).font(RFont.mono(13.5, .medium)).monospacedDigit()
            } else {
                Text(kind.name)
            }
            if chevron { GlyphIcon(.chevronDown, size: 13) }
        }
        .font(RFont.sans(14, .semibold))
        .lineLimit(1)
        .foregroundStyle(kind == .manual ? Palette.ink : .white)
        .padding(.horizontal, 13)
        .frame(height: 36)
        .background(kind == .manual ? Palette.paperRaised : kind.fill, in: Capsule())
        .overlay {
            if kind == .manual { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(spoken)
    }

    var title: String { mode.kind.name }

    /// "Bypass, 54:27 left"; "Lockdown".
    var spoken: String { mode.left.map { "\(mode.kind.name), \($0) left" } ?? mode.kind.name }
}

/// A word on a tint: Auto, New, Ready, Denied, Code. Colour never speaks alone, so a pill always has its word. `.tag`
/// is the outcome tag of an Activity row (20 pt, 11.5 semibold); `.pill` the larger one of facts and flags (24 pt,
/// 12.5 semibold: Changes CI, Approves, New AI).
struct Pill: View {
    enum Size { case tag, pill }

    enum Tone {
        case violet, success, danger, warning, neutral
        /// On a field: white with ink text (Review).
        case onField
        /// On a field: a white wash with white text (Autopilot's pick, Still asks).
        case onFieldWash
        /// Overnight's navy (Night).
        case night
        /// A kind's ink on its tile.
        case kind(FieldKind)
    }

    var text: String
    var tone: Tone = .neutral
    var glyph: Glyph? = nil
    var mode: ModeKind? = nil
    var size: Size = .tag
    /// An SF Symbol in place of a glyph (one the Signal set doesn't draw, like a family's).
    var symbol: String? = nil

    init(_ text: String, tone: Tone = .neutral, glyph: Glyph? = nil, mode: ModeKind? = nil, size: Size = .tag, symbol: String? = nil) {
        self.text = text
        self.tone = tone
        self.glyph = glyph
        self.mode = mode
        self.size = size
        self.symbol = symbol
    }

    var body: some View {
        let (ink, fill) = colors
        let tag = size == .tag
        HStack(spacing: tag ? 3 : 4) {
            if let mode {
                ModeGlyph(mode, size: tag ? 11 : 12)
            } else if let symbol {
                Image(systemName: symbol).font(.system(size: tag ? 10 : 11, weight: .semibold)).accessibilityHidden(true)
            } else if let glyph {
                GlyphIcon(glyph, size: tag ? 11 : 12)
            }
            Text(text).lineLimit(1)
        }
        .font(RFont.sans(tag ? 11.5 : 12.5, .semibold))
        .foregroundStyle(ink)
        .padding(.horizontal, tag ? 7 : 9)
        .frame(minHeight: tag ? 20 : 24)
        .background(fill, in: Capsule())
        .fixedSize()
    }

    private var colors: (Color, Color) {
        switch tone {
        case .violet: (Palette.violet, Palette.violet.opacity(0.12))
        case .success: (Palette.success, Palette.success.opacity(0.13))
        case .danger: (Palette.danger, Palette.danger.opacity(0.12))
        case .warning: (Palette.warning, Palette.warning.opacity(0.13))
        case .neutral: (Palette.ink2, Palette.paperSunken)
        case .onField: (Palette.onFieldInk, .white)
        case .onFieldWash: (.white, .white.opacity(0.2))
        case .night: (Palette.nightText, Palette.nightText.opacity(0.12))
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

/// A check box: 24 pt, rounded 8, violet with a white check when on, a 2 pt faint ring when off.
struct SignalCheckbox: View {
    var on: Bool

    var body: some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(on ? Palette.violet : .clear)
            .overlay {
                if on {
                    GlyphIcon(.check, size: 14).foregroundStyle(.white)
                } else {
                    RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(Palette.inkFaint, lineWidth: 2)
                }
            }
            .frame(width: 24, height: 24)
            .accessibilityHidden(true)
    }
}

/// − 5 +: a sunken capsule, 34 pt, with two bare buttons; each step ticks the detent.
struct SignalStepper: View {
    var value: String
    var canDecrease = true
    var canIncrease = true
    var onDecrease: () -> Void
    var onIncrease: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 4) {
            step(.minus, "Fewer", enabled: canDecrease, action: onDecrease)
            Text(value).font(RFont.mono(14)).foregroundStyle(Palette.ink).monospacedDigit().frame(minWidth: 14)
            step(.plus, "More", enabled: canIncrease, action: onIncrease)
        }
        .padding(.horizontal, 4)
        .frame(height: 34)
        .background(Palette.paperSunken, in: Capsule())
        .accessibilityElement(children: .contain)
    }

    private func step(_ glyph: Glyph, _ label: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button {
            action()
            feedback.play(.detent)
        } label: {
            GlyphIcon(glyph, size: 14)
                .foregroundStyle(Palette.ink)
                .frame(width: 26, height: 26)
                .frame(width: 34, height: 34)
                .contentShape(Circle())
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
                .font(RFont.sans(14, .semibold))
                .foregroundStyle(selected ? Palette.onInverse : Palette.ink)
                .padding(.horizontal, 13)
                .frame(height: 36)
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
