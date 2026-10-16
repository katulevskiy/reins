import SwiftUI

/// A decided push's changes line by line, as this iPhone has them (`ActivityPush`): kept with the approval, or left out
/// of the copy synced from another device, then loaded from GitHub one commit at a time through this phone's own
/// connection (`activityCommitChange`). Lives with an Activity entry's pages; the waiting sheet has none.
@Observable
@MainActor
final class PushDiffs {
    let entryId: Int64
    /// The changes were not kept on this device (`ActivityPush.diffsNotKept`).
    let notKept: Bool
    private(set) var loaded: [String: GitCommitView] = [:]
    private(set) var loading: Set<String> = []
    private(set) var failed: [String: String] = [:]

    init(entryId: Int64, notKept: Bool) {
        self.entryId = entryId
        self.notKept = notKept
    }

    /// The commit as best known: loaded from GitHub, else as kept.
    func commit(_ kept: GitCommitView) -> GitCommitView { loaded[kept.shortSha] ?? kept }

    /// Whether the commit page offers "Load from GitHub": not loaded yet, and its changes were not kept, or a text
    /// file it changes has no diff here.
    func missing(_ kept: GitCommitView) -> Bool {
        loaded[kept.shortSha] == nil && Self.missing(kept, notKept: notKept)
    }

    static func missing(_ commit: GitCommitView, notKept: Bool) -> Bool {
        notKept || commit.files.contains { !$0.binary && $0.diff.isEmpty }
    }

    func load(_ shortSha: String, core: any ReinsCoreProtocol, feedback: Feedback) async {
        guard !loading.contains(shortSha) else { return }
        loading.insert(shortSha)
        failed[shortSha] = nil
        defer { loading.remove(shortSha) }
        do {
            loaded[shortSha] = try await core.activityCommitChange(entryId: entryId, shortSha: shortSha)
            feedback.play(.refresh)
        } catch {
            feedback.play(.error)
            failed[shortSha] = error.userMessage
        }
    }
}

/// "Diff not kept on this iPhone", and the way to get it: Load from GitHub, a spinner while it comes, Retry if not.
struct DiffNotKept: View {
    var diffs: PushDiffs
    var shortSha: String
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        let busy = diffs.loading.contains(shortSha)
        let failure = diffs.failed[shortSha]
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                GlyphIcon(.cloud, size: 16).foregroundStyle(Palette.ink2)
                Text("Diff not kept on this iPhone").textRole(.body).foregroundStyle(Palette.ink2)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("diffNotKept")
            if let failure {
                Text(untrusted(failure)).textRole(.meta).foregroundStyle(Palette.danger)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("diffLoadError")
            }
            Button {
                Task { await diffs.load(shortSha, core: model.core, feedback: feedback) }
            } label: {
                HStack(spacing: 8) {
                    if busy { ProgressView().controlSize(.small) } else { GlyphIcon(.download, size: 16) }
                    Text(failure == nil ? "Load from GitHub" : "Retry")
                }
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
            .disabled(busy)
            .accessibilityIdentifier(failure == nil ? "loadDiff" : "retryDiff")
        }
        .padding(.horizontal, 22)
    }
}
