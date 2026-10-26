import SwiftUI

// The kit's buttons (the System board, Buttons): inverse for the action, outline for Deny, soft for quiet ones, a danger
// wash for End now, violet for Allow, and on a field white (Review) or a white wash (Deny). Pressed content dims to 55 %,
// disabled to 40 %; focus is a 2 pt violet ring at a 3 pt offset (white on fields).

enum SignalButtonKind: CaseIterable {
    /// The action: ink turned into a plate.
    case inverse
    /// Deny: transparent, a 1.5 pt `rule-2` edge.
    case outline
    /// Quiet: `paper-sunken` ("This was wrong").
    case soft
    /// Destructive on paper: red on `danger-wash` ("End now").
    case danger
    /// Violet fill, white text ("Allow" on a grant).
    case violet
    /// On a field: white with ink text ("Review").
    case onField
    /// On a field: a white wash with white text ("Deny").
    case onFieldWash

    var foreground: Color {
        switch self {
        case .inverse: Palette.onInverse
        case .outline, .soft: Palette.ink
        case .danger: Palette.danger
        case .violet, .onFieldWash: .white
        case .onField: Palette.onFieldInk
        }
    }

    var background: Color {
        switch self {
        case .inverse: Palette.inverse
        case .outline: .clear
        case .soft: Palette.paperSunken
        case .danger: Palette.dangerWash
        case .violet: Palette.violetFill
        case .onField: .white
        case .onFieldWash: Palette.onFieldWash
        }
    }

    /// Sits on a field: its focus ring is white.
    var onField: Bool { self == .onField || self == .onFieldWash }
}

enum SignalButtonSize {
    /// 54 pt, 16.5 semibold: the decision bar.
    case regular
    /// 42 pt, 15 semibold: in cards and rows (Change, Stop, Allow on a waiting card).
    case small

    var height: CGFloat { self == .regular ? 54 : 42 }
    var role: TextRole { self == .regular ? .button : .buttonSmall }
    var padding: CGFloat { self == .regular ? 22 : 16 }
}

/// `.buttonStyle(SignalButtonStyle(.inverse))`. Fills the width it is given (`fill: false` hugs its label). Plays the
/// default tap.
struct SignalButtonStyle: PrimitiveButtonStyle {
    var kind: SignalButtonKind
    var size: SignalButtonSize = .regular
    var fill = true
    /// A height other than the size's own (screens that keep their layout).
    var height: CGFloat?

    init(_ kind: SignalButtonKind, size: SignalButtonSize = .regular, fill: Bool = true, height: CGFloat? = nil) {
        self.kind = kind
        self.size = size
        self.fill = fill
        self.height = height
    }

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: SignalButtonLook(kind: kind, size: size, fill: fill, height: height)).makeBody(configuration: configuration)
    }
}

/// The look itself, for buttons that play their own feedback.
struct SignalButtonLook: ButtonStyle {
    var kind: SignalButtonKind
    var size: SignalButtonSize = .regular
    var fill = true
    var height: CGFloat?
    /// Font size other than the size's own (screens that keep their layout).
    var fontSize: CGFloat?

    func makeBody(configuration: Configuration) -> some View {
        SignalButtonBody(configuration: configuration, kind: kind, size: size, fill: fill, height: height, fontSize: fontSize)
    }
}

private struct SignalButtonBody: View {
    var configuration: ButtonStyleConfiguration
    var kind: SignalButtonKind
    var size: SignalButtonSize
    var fill: Bool
    var height: CGFloat?
    var fontSize: CGFloat?
    @Environment(\.isEnabled) private var enabled
    @Environment(\.isFocused) private var focused

    var body: some View {
        let role = size.role
        configuration.label
            .font(fontSize.map { RFont.sans($0, .semibold) } ?? role.font())
            .labelStyle(SignalLabelStyle())
            .foregroundStyle(kind.foreground)
            .lineLimit(1)
            .padding(.horizontal, size.padding)
            .frame(maxWidth: fill ? .infinity : nil, minHeight: height ?? size.height)
            .background(kind.background, in: Capsule())
            .overlay {
                if kind == .outline { Capsule().strokeBorder(Palette.rule2, lineWidth: 1.5) }
            }
            .contentShape(Capsule())
            .pressDims(pressed: configuration.isPressed, enabled: enabled)
            .signalFocusRing(focused, onField: kind.onField)
    }
}

/// Icon and title 8 pt apart, as in every Signal button.
struct SignalLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 8) {
            configuration.icon
            configuration.title
        }
    }
}

extension View {
    /// Signal's press states: pressed content dims to 55 %, disabled to 40 %.
    func pressDims(pressed: Bool, enabled: Bool = true) -> some View {
        opacity(enabled ? (pressed ? 0.55 : 1) : 0.4)
            .animation(.easeOut(duration: Motion.fade), value: pressed)
    }

    /// A 2 pt ring 3 pt outside a capsule: violet on paper, white on a field.
    func signalFocusRing(_ shown: Bool, onField: Bool = false) -> some View {
        overlay {
            if shown {
                Capsule()
                    .strokeBorder(onField ? Palette.onField : Palette.violet, lineWidth: 2)
                    .padding(-5)
                    .allowsHitTesting(false)
            }
        }
    }
}

/// A round header button (add, filter, back, share): 40 pt, `paper-raised` with a hairline ring, no glass. On a field it
/// is the white wash with a white glyph.
struct RoundHeaderButton: View {
    var glyph: Glyph
    var label: String
    var onField = false
    var size: CGFloat = 40
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            GlyphIcon(glyph, size: onField ? 18 : 20)
        }
        .buttonStyle(RoundHeaderLook(onField: onField, size: size))
        .accessibilityLabel(label)
    }
}

private struct RoundHeaderLook: PrimitiveButtonStyle {
    var onField: Bool
    var size: CGFloat

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapStyle(look: Look(onField: onField, size: size)).makeBody(configuration: configuration)
    }

    struct Look: ButtonStyle {
        var onField: Bool
        var size: CGFloat
        @Environment(\.isEnabled) private var enabled
        @Environment(\.isFocused) private var focused

        func makeBody(configuration: Configuration) -> some View {
            configuration.label
                .foregroundStyle(onField ? Palette.onField : Palette.ink)
                .frame(width: size, height: size)
                .background(onField ? Palette.onFieldWash : Palette.paperRaised, in: Circle())
                .overlay {
                    if !onField { Circle().strokeBorder(Palette.rule, lineWidth: 1) }
                }
                .contentShape(Circle())
                .pressDims(pressed: configuration.isPressed, enabled: enabled)
                .signalFocusRing(focused, onField: onField)
        }
    }
}
