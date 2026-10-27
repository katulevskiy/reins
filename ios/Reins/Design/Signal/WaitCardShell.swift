import SwiftUI

/// A waiting ask as a card in its field colour (the System board's hero template): the request's icon at the left, who
/// asks and the timer at the right; the ask in `cardAsk`; then the buttons, Autopilot's pick first. Radius 26, 16 pt
/// inside, 12 pt between its lines, the field's texture. The label line ("Gmail · Send") is gone from the card: the
/// icon says it, and VoiceOver reads it.
struct WaitCardShell<Avatar: View, Timer: View, Actions: View>: View {
    var kind: FieldKind
    /// "Gmail · Send": what the icon shows, for VoiceOver.
    var label: String
    var ask: AskText
    /// The ask's accessibility identifier (UI tests open a card by it).
    var askIdentifier: String? = nil
    @ViewBuilder var avatar: Avatar
    @ViewBuilder var timer: Timer
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                avatar.accessibilityElement(children: .ignore).accessibilityLabel(label)
                Spacer(minLength: 0)
                timer
            }
            ask.foregroundStyle(Palette.onField)
                .accessibilityIdentifier(askIdentifier ?? "")
            actions
        }
        .padding(16)
        .fieldBackground(kind, in: RoundedRectangle(cornerRadius: 26, style: .continuous))
        .accessibilityElement(children: .contain)
    }
}

/// Later on a waiting card (`decideLater`): the request moves to "Waiting for you" and the agent carries on. Round, in
/// the field's wash, beside the other buttons; the full width with its word at accessibility sizes.
struct LaterButton: View {
    var wide = false
    var action: () -> Void

    var body: some View {
        if wide {
            Button(action: action) {
                Label { Text("Later") } icon: { Image(systemName: "clock.arrow.circlepath") }
            }
            .buttonStyle(SignalButtonStyle(.onFieldWash, size: .small))
            .accessibilityIdentifier("later")
        } else {
            Button(action: action) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(Palette.onField)
                    .frame(width: 42, height: 42)
                    .background(Palette.onFieldWash, in: Circle())
                    .contentShape(Circle())
            }
            .buttonStyle(PressDimsStyle())
            .accessibilityLabel("Later")
            .accessibilityIdentifier("later")
        }
    }
}

/// A waiting card's buttons with Later after them when `later` is given: on the same row, or under them at
/// accessibility sizes.
struct WithLater<Buttons: View>: View {
    var later: (() -> Void)?
    @ViewBuilder var buttons: Buttons
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        if let later {
            if typeSize.isAccessibilitySize {
                VStack(spacing: 8) {
                    buttons
                    LaterButton(wide: true, action: later)
                }
            } else {
                HStack(spacing: 8) {
                    buttons
                    LaterButton(action: later)
                }
            }
        } else {
            buttons
        }
    }
}

/// Autopilot's pick, the first button on a waiting card.
enum AutopilotPick: Equatable {
    /// "Allow · 97%": white, with the approve glyph.
    case allow(Int)
    /// "Deny · 91%": white.
    case deny(Int)
    /// Not sure: a half ring and "Unsure", in the wash.
    case unsure
    /// Never decided by Autopilot: a lock and "Always asks", in the wash.
    case alwaysAsks

    var title: String {
        switch self {
        case .allow(let p): "Allow \(p)%"
        case .deny(let p): "Deny \(p)%"
        case .unsure: "Unsure"
        case .alwaysAsks: "Always asks"
        }
    }

    /// A firm pick is white; a doubt or a rule is the wash, and Review takes the white.
    var firm: Bool {
        switch self {
        case .allow, .deny: true
        case .unsure, .alwaysAsks: false
        }
    }
}

/// The pick and Review, as a waiting card's buttons: side by side for a firm pick; for a doubt or a rule the pick is as
/// wide as its words and Review, white, takes the rest.
struct WaitCardButtons: View {
    var pick: AutopilotPick
    var onPick: () -> Void
    var onReview: () -> Void
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        if pick.firm && typeSize.isAccessibilitySize {
            // Accessibility sizes: the pick over Review, each the full width.
            VStack(spacing: 8) {
                Button(action: onPick) { pickLabel }
                    .buttonStyle(SignalButtonStyle(.onField, size: .small))
                review(.onFieldWash)
            }
        } else if pick.firm {
            RatioRow(spacing: 8, ratio: 1) {
                Button(action: onPick) { pickLabel }
                    .buttonStyle(SignalButtonStyle(.onField, size: .small))
                review(.onFieldWash)
            }
        } else {
            // The doubt or the rule takes what its words need; Review, white, the rest.
            HStack(spacing: 8) {
                Group {
                    if pick == .alwaysAsks {
                        // A rule, not a choice: nothing to press.
                        pickLabel
                            .font(TextRole.buttonSmall.font())
                            .foregroundStyle(Palette.onField)
                            .padding(.horizontal, 14)
                            .frame(minHeight: 40)
                            .background(Palette.onFieldWash, in: Capsule())
                            .accessibilityElement(children: .combine)
                    } else {
                        Button(action: onPick) { pickLabel }
                            .buttonStyle(SignalButtonStyle(.onFieldWash, size: .small, fill: false))
                    }
                }
                .fixedSize(horizontal: true, vertical: false)
                review(.onField)
            }
        }
    }

    private func review(_ kind: SignalButtonKind) -> some View {
        Button("Review", action: onReview)
            .buttonStyle(SignalButtonStyle(kind, size: .small))
    }

    private var pickLabel: some View {
        HStack(spacing: 8) {
            switch pick {
            case .allow: GlyphIcon(.faceId, size: 18)
            case .deny: GlyphIcon(.close, size: 18)
            case .unsure: ArcRing(fraction: 0.5, color: .white, track: .white.opacity(0.35), lineWidth: 2).frame(width: 16, height: 16)
            case .alwaysAsks: GlyphIcon(.lock, size: 16)
            }
            switch pick {
            case .allow(let p), .deny(let p):
                HStack(spacing: 6) {
                    Text(pick == .allow(p) ? "Allow" : "Deny")
                    Text("\(p)%").font(RFont.sans(15, .medium)).opacity(0.55)
                }
                .lineLimit(1)
            default:
                Text(pick.title).lineLimit(1)
            }
        }
    }
}
