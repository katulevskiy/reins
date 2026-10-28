import SwiftUI

/// What a change to Auto would have done on this phone's own last 30 days (Decisions v2, 02 §4): "Last 30 days: 12
/// Auto → Ask, 0 Ask → Auto". Worked out by the core on the phone; nothing is sent.
struct PreviewLine: View {
    var line: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "clock.arrow.circlepath")
                .font(.system(size: 14, weight: .regular))
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 20)
            Text(verbatim: line)
                .font(RFont.sans(14))
                .foregroundStyle(Palette.ink2)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("previewLineText")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("previewLine")
    }
}

/// A preview asked of the core; "" when it couldn't be worked out (the change itself still asks the owner).
@MainActor
enum PolicyPreview {
    static func line(_ core: ReinsCoreProtocol, _ change: PolicyChange) async -> String {
        (try? await core.previewPolicyChange(change: change).line) ?? ""
    }

    /// What a dialog says when there is no line.
    static func shown(_ line: String) -> String {
        line.isEmpty ? "No history yet" : line
    }
}
