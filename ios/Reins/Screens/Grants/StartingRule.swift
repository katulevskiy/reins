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
                    .font(RFont.sans(12.5, .semibold))
                    .tracking(0.9)
                    .foregroundStyle(Palette.tertiary)
                    .accessibilityAddTraits(.isHeader)
                // The (?) right beside the words it explains.
                HelpButton(text: StartingRuleText.help)
                    .accessibilityIdentifier("startingRuleHelp")
                Spacer(minLength: 0)
            }
            .padding(.leading, 6)
            .padding(.trailing, 2)
            .padding(.bottom, 6)
            VStack(spacing: 0) {
                ForEach(Array(StartingRuleText.order.enumerated()), id: \.offset) { i, policy in
                    if i > 0 { Hairline(inset: 50) }
                    row(policy, on: chosen == policy)
                }
            }
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
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
                RadioMark(on: on).padding(.top, 1)
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 8) {
                        Text(StartingRuleText.title(policy))
                            .font(RFont.sans(15.5, .semibold))
                            .foregroundStyle(Palette.text)
                        if StartingRuleText.recommended(policy) {
                            Text("Recommended")
                                .font(RFont.sans(11, .semibold))
                                .foregroundStyle(Palette.success)
                                .padding(.horizontal, 8)
                                .padding(.vertical, 3)
                                .background(Palette.success.opacity(0.13), in: Capsule())
                        }
                    }
                    Text(StartingRuleText.detail(policy))
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.tertiary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 14)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier(StartingRuleText.id(policy))
    }
}
