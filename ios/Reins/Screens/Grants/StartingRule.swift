import SwiftUI

/// The words of the starting rule, kept apart so they can be tested (the Android app's StartingRuleText).
enum StartingRuleText {
    static let header = "When you connect a new AI"

    static func title(_ policy: StartingPolicy) -> String {
        switch policy {
        case .readsForADay: "Let it read for a day"
        case .askEveryTime: "Ask me every time"
        }
    }

    static func detail(_ policy: StartingPolicy) -> String {
        switch policy {
        case .readsForADay:
            "It can search and read your connected services for 24 hours without asking. Anything that looks like a code or a password still waits for you."
        case .askEveryTime: "Every search and every read waits for your OK, like everything else."
        }
    }

    static let alwaysAsks =
        "Always asks, whatever you choose: sending, changing or deleting anything; passwords, codes and secrets; force pushes; new connections."

    /// The line on a pairing sheet: what approving the connection gives it, or nil when nothing.
    static func onPairing(_ policy: StartingPolicy?) -> String? {
        policy == .readsForADay
            ? "Your starting rule lets it search and read for 24 hours without asking. Sending and changing anything still ask."
            : nil
    }
}

/// The starting rule, picked in one tap: "Let it read for a day" (recommended) or "Ask me every time". In the
/// onboarding and at the end of Grants; nil means nothing was chosen yet (which asks for everything).
struct StartingRuleChooser: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        GroupCard(header: StartingRuleText.header, footer: StartingRuleText.alwaysAsks) {
            ForEach(Array([StartingPolicy.readsForADay, .askEveryTime].enumerated()), id: \.offset) { i, policy in
                if i > 0 { Hairline(inset: 52) }
                CheckRow(checked: model.startingPolicy == policy, onChange: { _ in model.chooseStartingPolicy(policy) }) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(StartingRuleText.title(policy)).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
                        Text(StartingRuleText.detail(policy))
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                        if policy == .readsForADay {
                            TintTag(text: "Recommended", tint: Palette.success).padding(.top, 4)
                        }
                    }
                }
                .padding(16)
                .accessibilityIdentifier(policy == .readsForADay ? "rule:readsForADay" : "rule:askEveryTime")
            }
        }
        .accessibilityIdentifier("startingRule")
    }
}
