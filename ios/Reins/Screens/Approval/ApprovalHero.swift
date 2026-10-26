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
    var label: RequestLabel? = nil
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
                WarnTag(text: "No longer waiting", tint: Palette.warning)
                    .padding(.horizontal, 20)
                    .padding(.top, 4)
                    .accessibilityIdentifier("noLongerWaiting")
            }
            if nav.riskInFacts, let risk = vm.suggestion?.risk, !untrusted(risk.why).isEmpty {
                FactGroup {
                    FactRow(key: "Why it's risky", value: untrusted(risk.why), lines: 6)
                }
                .accessibilityIdentifier("riskWhy")
            }
            Facts(vm: vm, view: view)
        }
        .animation(.smooth(duration: 0.25), value: vm.error)
    }
}

/// A small tinted tag with a glyph and a few words (the kit's `tagw`: Changes CI, Hidden, Rewrites history): 24 pt,
/// 12.5 semibold, the tint at 13 %. `lines` > 1 lets a long reason wrap.
struct WarnTag: View {
    var text: String
    var tint: Color
    var small = false
    var glyph: Glyph? = .warning
    /// An SF Symbol in place of the glyph, for a mark the glyph set lacks.
    var symbol: String? = nil
    var lines = 1

    var body: some View {
        HStack(alignment: .top, spacing: 4) {
            if let symbol {
                Image(systemName: symbol).font(.system(size: 11, weight: .semibold)).padding(.top, 2.5)
            } else if let glyph {
                GlyphIcon(glyph, size: 12).padding(.top, 2.5)
            }
            Text(verbatim: text)
                .lineLimit(lines)
                .fixedSize(horizontal: false, vertical: lines > 1)
        }
        .font(RFont.sans(12.5, .semibold))
        .foregroundStyle(tint)
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .frame(minHeight: 24)
        .background(tint.opacity(0.13), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// What deserves a closer look in one commit, file or email: the warning sign and its 1 to 3 reasons. Close is in the
/// warning colour; Look is quieter, in ink-2, so "Changes CI" reads as more serious than "Runs on build". Red stays for
/// danger. VoiceOver reads "Look closer: Changes CI".
struct AttentionTag: View {
    var mark: AttentionMark
    var id: String

    var body: some View {
        WarnTag(text: mark.text, tint: mark.level == .close ? Palette.warning : Palette.ink2, small: true, lines: 2)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(mark.spoken)
            .accessibilityIdentifier(id)
    }
}
