import Observation
import SwiftUI

/// A file an AI uploaded through the server: who, what, why, and a preview, then approve (its download link works)
/// or deny (the server deletes it).
struct UploadSheet: View {
    var blobId: String
    @Environment(AppModel.self) private var model
    @State private var vm: UploadModel

    init(blobId: String) {
        self.blobId = blobId
        _vm = State(initialValue: UploadModel(id: blobId))
    }

    init(model: UploadModel) {
        blobId = model.id
        _vm = State(initialValue: model)
    }

    var body: some View {
        Group {
            if let view = vm.view {
                UploadContent(vm: vm, view: view)
            } else {
                DecisionLoading(error: vm.loading ? nil : vm.error)
            }
        }
        .task { await vm.load(model) }
        .onChange(of: vm.finished) { _, done in if done { model.closeSheet() } }
    }
}

/// The upload sheet's state (the Android app's UploadViewModel).
@Observable
@MainActor
final class UploadModel {
    let id: String
    private(set) var loading = true
    private(set) var view: BlobView?
    private(set) var busy = false
    var error: String?
    private(set) var finished = false

    init(id: String) {
        self.id = id
    }

    init(view: BlobView) {
        id = view.id
        self.view = view
        loading = false
    }

    func load(_ app: AppModel) async {
        guard view == nil else { return }
        do {
            view = try await app.core.blobView(id: id)
        } catch {
            self.error = decisionErrorMessage(error)
        }
        loading = false
    }

    /// Approving needs the owner's Face ID, Touch ID or passcode first, as every approval does.
    func approve(_ app: AppModel) async {
        guard let view, !busy else { return }
        busy = true
        error = nil
        switch await OwnerCheck.confirm(app, reason: "Approve the file from \(untrusted(view.connectionLabel))") {
        case .confirmed:
            app.feedback.play(.uploadApproved)
            await answer(app, approve: true)
        case .cancelled:
            busy = false
        case .unavailable:
            app.feedback.play(.error)
            busy = false
            error = OwnerCheck.unavailableMessage
        }
    }

    func deny(_ app: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        app.feedback.play(.denied)
        await answer(app, approve: false)
    }

    private func answer(_ app: AppModel, approve: Bool) async {
        do {
            try await app.core.answerBlob(id: id, approve: approve)
            await app.refreshPending()
            busy = false
            finished = true
        } catch {
            app.feedback.play(.error)
            busy = false
            self.error = decisionErrorMessage(error)
        }
    }
}

private struct UploadContent: View {
    @Bindable var vm: UploadModel
    var view: BlobView
    @Environment(AppModel.self) private var model

    var body: some View {
        let who = untrusted(view.connectionLabel)
        DecisionLayout {
            VStack(alignment: .leading, spacing: 0) {
                DecisionHeader(
                    connectionId: "",
                    label: view.connectionLabel,
                    subtitle: TimeText.dateTime(view.createdAt),
                    kind: .upload,
                    title: "Share a file",
                    known: false
                ) {
                    ConnectorTags(service: "files", account: nil).padding(.top, 12)
                }
                if !view.purpose.trimmingCharacters(in: .whitespaces).isEmpty {
                    QuoteBox(caption: "\(who) says it is for", text: untrusted(view.purpose))
                        .padding(.horizontal, 16)
                        .padding(.vertical, 8)
                        .accessibilityIdentifier("uploadReason")
                }
                FileCard(blob: view).padding(.horizontal, 16).padding(.vertical, 8)
                Text("\(who) uploaded this file to your Rewarden server. If you approve, its download link works and \(who) can hand it to another tool. If you deny, the server deletes it. Either way it is gone at \(TimeText.dateTime(view.expiresAt)).")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 10)
                if let error = vm.error {
                    Banner(error, kind: .error).padding(16).accessibilityIdentifier("uploadError")
                }
            }
        } recap: {
            VStack(alignment: .leading, spacing: 4) {
                Text(who).font(RFont.sans(14, .medium)).foregroundStyle(Palette.secondary)
                Text("Share \(untrusted(view.name))?").font(RFont.sans(22, .semibold)).foregroundStyle(Palette.text).lineLimit(3)
            }
        } decision: {
            DecisionBar(
                busy: vm.busy,
                onDeny: { Task { await vm.deny(model) } },
                onApprove: { Task { await vm.approve(model) } }
            )
        }
        .accessibilityContainer("uploadSheet")
    }
}

#Preview("Upload") {
    PreviewHost { UploadSheet(blobId: "blob_q3numbers") }
}
