import SwiftUI

/// The words of the starting rule, kept apart so they can be tested (the Android app's StartingRuleText).
enum StartingRuleText {
    static let header = "New AI connections"

    /// Behind the (?).
    static let help = "What a newly connected AI may do before it asks. Codes and passwords always ask."

    /// The choices, the recommended one first.
    static let order: [StartingPolicy] = [.askEveryTime, .headlinesFirstHour]

    static func title(_ policy: StartingPolicy) -> String {
        switch policy {
        case .askEveryTime: "Ask every time"
        case .headlinesFirstHour: "Headlines for its first hour"
        }
    }

    static func detail(_ policy: StartingPolicy) -> String {
        switch policy {
        case .askEveryTime: "Nothing is allowed until you say so."
        case .headlinesFirstHour: "Searches show sender, subject and date. Opening anything still asks."
        }
    }

    static func recommended(_ policy: StartingPolicy) -> Bool { policy == .askEveryTime }

    /// The identifier of a choice's row ("rule:askEveryTime").
    static func id(_ policy: StartingPolicy) -> String {
        switch policy {
        case .askEveryTime: "rule:askEveryTime"
        case .headlinesFirstHour: "rule:headlinesFirstHour"
        }
    }

    /// The line on a pairing sheet: what approving the connection gives it, or nil when nothing.
    static func onPairing(_ policy: StartingPolicy?) -> String? {
        policy == .headlinesFirstHour
            ? "For its first hour it sees headlines only: sender, subject and date. Opening anything still asks."
            : nil
    }
}

/// The starting rule, picked in one tap: "Ask every time" (recommended) or "Headlines for its first hour". In the
/// onboarding and at the end of Grants; nothing chosen yet means it asks every time.
struct StartingRuleChooser: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        let chosen = model.startingPolicy ?? .askEveryTime
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 6) {
                Text(StartingRuleText.header.uppercased())
                    .textRole(.label)
                    .foregroundStyle(Palette.inkMeta)
                    .accessibilityAddTraits(.isHeader)
                // The (?) right beside the words it explains.
                HelpButton(text: StartingRuleText.help)
                    .accessibilityIdentifier("startingRuleHelp")
                Spacer(minLength: 0)
            }
            .padding(.leading, 6)
            .padding(.trailing, 2)
            .padding(.bottom, 6)
            // On hairlines, not a card: the chosen one carries the check.
            VStack(spacing: 0) {
                Rectangle().fill(Palette.rule).frame(height: 1)
                ForEach(StartingRuleText.order, id: \.self) { policy in
                    row(policy, on: chosen == policy)
                    Rectangle().fill(Palette.rule).frame(height: 1)
                }
            }
        }
        .padding(.horizontal, 16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("startingRule")
    }

    private func row(_ policy: StartingPolicy, on: Bool) -> some View {
        Button {
            guard !on else { return }
            model.chooseStartingPolicy(policy)
            feedback.play(.selection)
        } label: {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(StartingRuleText.title(policy))
                            .textRole(.row)
                            .foregroundStyle(Palette.ink)
                        if StartingRuleText.recommended(policy) {
                            Text("Recommended")
                                .font(TextRole.tag.font())
                                .foregroundStyle(Palette.success)
                        }
                    }
                    Text(StartingRuleText.detail(policy))
                        .textRole(.meta)
                        .foregroundStyle(Palette.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                GlyphIcon(.check, size: 18)
                    .foregroundStyle(Palette.ink)
                    .opacity(on ? 1 : 0)
                    .scaleEffect(on ? 1 : 0.6)
                    .animation(.easeOut(duration: Motion.fade), value: on)
                    .padding(.top, 1)
                    .accessibilityHidden(true)
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier(StartingRuleText.id(policy))
    }
}
