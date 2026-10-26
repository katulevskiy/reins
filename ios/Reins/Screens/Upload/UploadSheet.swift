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
        switch await OwnerPrompt.confirm(app, reason: "Approve the file from \(untrusted(view.connectionLabel))") {
        case .confirmed:
            app.feedback.play(.uploadApproved)
            await answer(app, approve: true)
        case .cancelled:
            busy = false
        case .unavailable:
            app.feedback.play(.error)
            busy = false
            error = OwnerPrompt.unavailableMessage
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

/// The sheet (Next-Connect 05, a paper sheet): who uploaded it with the address it came from and when, close; the
/// request's icon and "Share a file"; why it says it uploaded it; the file (its name, size, SHA-256 and a preview);
/// when the server deletes it either way, with what approving does behind its (?); then the dock.
private struct UploadContent: View {
    @Bindable var vm: UploadModel
    var view: BlobView
    @Environment(AppModel.self) private var model

    var body: some View {
        let who = untrustedName(view.connectionLabel, limit: 40)
        DecisionLayout(closeInField: true) {
            VStack(alignment: .leading, spacing: 0) {
                header
                VStack(alignment: .leading, spacing: 12) {
                    RequestIcon(spec: AskIcon(mark: .glyph(.cloudUpload), action: .file, kind: .read), size: 40, ring: Palette.paper)
                    Text("Share a file")
                        .font(RFont.display(26.5))
                        .tracking(RFont.displayTracking(26.5))
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                        .accessibilityIdentifier("what")
                }
                .padding(.horizontal, 20)
                .padding(.top, 18)
                if !view.purpose.trimmingCharacters(in: .whitespaces).isEmpty {
                    FactGroup {
                        HStack(alignment: .top, spacing: 12) {
                            GlyphIcon(.fileText, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
                            Text("“\(untrusted(view.purpose))”")
                                .font(RFont.sans(15.5, .medium))
                                .lineHeight(.multiple(factor: 21 / 15.5))
                                .foregroundStyle(Palette.ink)
                                .lineLimit(6)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 14)
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel("\(who) says it is for: \(untrusted(view.purpose))")
                    }
                    .padding(.top, 16)
                    .accessibilityIdentifier("uploadReason")
                }
                FactHeader(text: name, glyph: .file) {
                    Text(FileText.size(view.size)).font(RFont.sans(13, .semibold)).foregroundStyle(Palette.ink2)
                }
                .padding(.top, 18)
                .padding(.bottom, 8)
                FactGroup {
                    // One row: the hash and the preview under it, no hairline between.
                    VStack(alignment: .leading, spacing: 0) {
                        FactLine(glyph: .fingerprint, value: FileText.shortSha(untrusted(view.sha256)), mono: true, valueId: "uploadSha")
                            .accessibilityLabel("SHA-256 \(untrusted(view.sha256))")
                        preview
                    }
                }
                .accessibilityIdentifier("uploadFile")
                HStack(spacing: 6) {
                    Text("Deleted \(Self.when(view.expiresAt)) either way")
                        .textRole(.meta)
                        .foregroundStyle(Palette.inkMeta)
                        .fixedSize(horizontal: false, vertical: true)
                    HelpDot(
                        id: "upload",
                        text: "\(who) uploaded this file to your Reins server. If you approve, its download link works and \(who) can hand it to another tool. If you deny, the server deletes it now.",
                        size: 28
                    )
                }
                .padding(.horizontal, 24)
                .padding(.top, 8)
                .accessibilityIdentifier("uploadExpiry")
                if let error = vm.error {
                    WarnTag(text: error, tint: Palette.danger, lines: 4)
                        .padding(.horizontal, 20)
                        .padding(.top, 12)
                        .accessibilityIdentifier("uploadError")
                }
            }
        } recap: {
            VStack(alignment: .leading, spacing: 4) {
                Text(who).font(RFont.sans(14, .medium)).foregroundStyle(Palette.ink2)
                Text("Share \(name)?").textRole(.cardAsk).foregroundStyle(Palette.ink).lineLimit(3)
            }
        } decision: {
            DecisionBar(
                busy: vm.busy,
                docked: true,
                onDeny: { Task { await vm.deny(model) } },
                onApprove: { Task { await vm.approve(model) } }
            )
        }
        // Near the top, as the board's sheet (the app's sheets otherwise stop at 86 %).
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(32)
        .accessibilityContainer("uploadSheet")
    }

    private var name: String {
        let n = untrusted(view.name)
        return n.isEmpty ? "(no name)" : n
    }

    /// Who uploaded it (its avatar, the name it was given and the address it came from, never part of the sentence),
    /// when, and close.
    private var header: some View {
        let connection = findConnection(model.connections, id: "", label: view.connectionLabel)
        let host = connection.map { asciiHost(untrustedName($0.clientHost)) } ?? ""
        let who = untrustedName(view.connectionLabel, limit: 40)
        return HStack(spacing: 10) {
            HStack(spacing: 10) {
                if let connection {
                    ConnectionAvatar(connection: connection, size: 28)
                } else {
                    ConnectionAvatar(label: who, pick: nil, size: 28)
                }
                (Text(verbatim: who).foregroundStyle(Palette.ink) + Text(verbatim: host.isEmpty ? "" : " · \(host)").foregroundStyle(Palette.inkMeta))
                    .font(RFont.sans(15, .semibold))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel("Uploaded by \(who)\(host.isEmpty ? "" : ", \(host)")")
            .accessibilityIdentifier("whoAsks")
            Text(Self.when(view.createdAt)).textRole(.meta).foregroundStyle(Palette.inkMeta).lineLimit(1)
            RoundHeaderButton(glyph: .close, label: "Close") { model.closeSheet() }
                .accessibilityIdentifier("closeSheet")
        }
        .padding(.leading, 20)
        .padding(.trailing, 16)
        .padding(.top, 16)
    }

    /// "2:31 PM" today, else the date and time.
    static func when(_ epochSeconds: Int64) -> String {
        let d = Date(timeIntervalSince1970: TimeInterval(epochSeconds))
        let today = Calendar.current.isDate(d, inSameDayAs: Date(timeIntervalSince1970: TimeInterval(Timers.nowSeconds())))
        return today ? d.formatted(date: .omitted, time: .shortened) : TimeText.dateTime(epochSeconds)
    }

    /// The start of the file: an image, the first lines of text in mono, or what the server says a binary file is.
    @ViewBuilder private var preview: some View {
        let text = view.previewText.map(untrusted).flatMap { $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : $0 }
        if let data = view.previewImage, let image = UIImage(data: data) {
            Image(uiImage: image)
                .resizable()
                .scaledToFit()
                .frame(maxWidth: .infinity, maxHeight: 240)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .padding(12)
                .accessibilityLabel("Preview of \(name)")
        } else if let text, FileText.isText(view.contentType) {
            Text(text)
                .font(RFont.mono(12.5))
                .lineHeight(.multiple(factor: 18 / 12.5))
                .foregroundStyle(Palette.ink2)
                .lineLimit(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 16)
                .padding(.top, 4)
                .padding(.bottom, 14)
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
                .accessibilityIdentifier("uploadPreview")
        } else {
            FactRow(key: "Type", value: untrusted(view.contentType), mono: true)
            if let text { Text(text).font(RFont.sans(14, .medium)).foregroundStyle(Palette.ink2).lineLimit(2).padding(16) }
        }
    }
}

#if DEBUG
#Preview("Upload") {
    PreviewHost { UploadSheet(blobId: "blob_q3numbers") }
}
#endif
