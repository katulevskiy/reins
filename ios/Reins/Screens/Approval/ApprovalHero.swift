import SwiftUI

// The approval's colours and the page under its field (ApprovalField): what kind of field an approval is painted in,
// how an Activity entry's ending is passed down, and the main page.

extension HeroTone {
    /// The tone as a Signal field: writes and sends push, searches read, grants grant, what can't be undone delete.
    var field: FieldKind {
        switch self {
        case .write, .send: .push
        case .search: .read
        case .grant: .grant
        case .accounts: .accounts
        case .danger: .delete
        }
    }

    var color: Color { field.ink }
}

/// An Activity entry shown again with the approval's own screen, read-only (the Android app's `ApprovalRecord`): how
/// it ended, which the hero shows in place of Autopilot's score.
struct ApprovalRecord: Equatable {
    var outcome: String
    var symbol: String
    var tint: Color
    /// The risk the model saw in it, when it rated one.
    var risk: RiskView? = nil
}

private struct ApprovalRecordKey: EnvironmentKey {
    static let defaultValue: ApprovalRecord? = nil
}

extension EnvironmentValues {
    /// Set for an Activity entry's request as its approval showed it; nil for a request that waits.
    var approvalRecord: ApprovalRecord? {
        get { self[ApprovalRecordKey.self] }
        set { self[ApprovalRecordKey.self] = newValue }
    }
}

/// The main page: the field, then the facts of the request, by kind. When the risk's reason did not fit the field's
/// two lines, it is the first fact.
struct MainPage: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    var gone: Bool
    /// The status bar's height: the field runs under it.
    var top: CGFloat = 0
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ApprovalField(vm: vm, view: view, autopilot: autopilot, top: top)
            if gone {
                Label("No longer waiting", systemImage: "exclamationmark.triangle.fill")
                    .font(RFont.sans(13.5, .semibold))
                    .foregroundStyle(Palette.warning)
                    .padding(.horizontal, 22)
                    .accessibilityIdentifier("noLongerWaiting")
            }
            if nav.riskInFacts, let risk = vm.suggestion?.risk, !untrusted(risk.why).isEmpty {
                FactGroup {
                    FactRow(key: "Why it's risky", value: untrusted(risk.why), lines: 6)
                }
                .accessibilityIdentifier("riskWhy")
            }
            Facts(vm: vm, view: view).padding(.top, 4)
        }
        .animation(.smooth(duration: 0.25), value: vm.error)
    }
}

/// A tinted tag with a real symbol (the warning sign unless said otherwise).
struct WarnTag: View {
    var text: String
    var tint: Color
    var small = false
    var symbol = "exclamationmark.triangle.fill"
    var lines = 1

    var body: some View {
        Label(text, systemImage: symbol)
            .labelStyle(.titleAndIcon)
            .font(RFont.sans(small ? 11.5 : 13.5, .semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, small ? 8 : 12)
            .padding(.vertical, small ? 4 : 7)
            .background(tint.opacity(0.14), in: RoundedRectangle(cornerRadius: small ? 11 : 16, style: .continuous))
            .lineLimit(lines)
            .fixedSize(horizontal: false, vertical: lines > 1)
    }
}

/// What deserves a closer look in one commit, file or email: the warning sign and its 1 to 3 reasons. Close is the
/// filled sign in the warning colour; Look is quieter, the outline sign in a muted ink, so "Changes CI" reads as more
/// serious than "Runs on build". Red stays for danger. VoiceOver reads "Look closer: Changes CI".
struct AttentionTag: View {
    var mark: AttentionMark
    var id: String

    var body: some View {
        WarnTag(
            text: mark.text, tint: mark.level == .close ? Palette.warning : Palette.ink2, small: true,
            symbol: mark.level == .close ? "exclamationmark.triangle.fill" : "exclamationmark.triangle", lines: 2
        )
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(mark.spoken)
        .accessibilityIdentifier(id)
    }
}
