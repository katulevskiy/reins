import SwiftUI

/// A waiting ask as a card in its field colour (Components.png, Waiting cards): the AI's mark, the label line and the
/// timer; the ask in `ask-small`; then the buttons, Autopilot's pick first. Radius 26, 16 / 18 pt inside, 12 pt between
/// its lines, grain on the field. The Activity stack fills it in phase 2.
struct WaitCardShell<Avatar: View, Timer: View, Actions: View>: View {
    var kind: FieldKind
    /// "Gmail · Send": set in mono capitals.
    var label: String
    var ask: AskText
    @ViewBuilder var avatar: Avatar
    @ViewBuilder var timer: Timer
    @ViewBuilder var actions: Actions

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                avatar
                Text(label)
                    .textRole(.label)
                    .foregroundStyle(Palette.onField2)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                timer
            }
            ask.foregroundStyle(Palette.onField)
            actions
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 16)
        .fieldBackground(kind, in: RoundedRectangle(cornerRadius: 26, style: .continuous))
        .accessibilityElement(children: .contain)
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
        case .allow(let p): "Allow · \(p)%"
        case .deny(let p): "Deny · \(p)%"
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

    var body: some View {
        if pick.firm {
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
            Text(pick.title).lineLimit(1)
        }
    }
}
