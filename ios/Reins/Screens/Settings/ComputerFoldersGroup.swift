import SwiftUI

/// A computer's Folders, as its page reads and changes them: read when the page shows and after each change. Trusting a
/// suggested folder loosens (the core asks for the owner check); removing a trusted one only tightens and applies at
/// once.
@Observable
@MainActor
final class ComputerFoldersModel {
    let connectionId: String
    /// nil until first read.
    private(set) var folders: ComputerFolders?
    private(set) var busy = false
    var error: String?

    init(connectionId: String) {
        self.connectionId = connectionId
    }

    var trusted: [FolderView] { folders?.confirmed ?? [] }
    var suggested: [FolderView] { folders?.suggested ?? [] }

    /// Reads them again; a read that fails keeps the last ones.
    func load(_ app: AppModel) async {
        if let read = try? await app.core.computerFolders(connectionId: connectionId) { folders = read }
    }

    func trust(_ folder: FolderView, _ app: AppModel) async {
        await run(app) { await app.trustFolder(connectionId: self.connectionId, path: folder.path, kind: folder.kind) }
    }

    func remove(_ folder: FolderView, _ app: AppModel) async {
        await run(app) { await app.removeFolder(connectionId: self.connectionId, path: folder.path, kind: folder.kind) }
    }

    private func run(_ app: AppModel, _ change: () async -> String?) async {
        guard !busy else { return }
        busy = true
        error = await change()
        await load(app)
        busy = false
    }
}

/// Settings › Computers › a computer › Folders: the trusted folders, each with remove (at once); then the suggested
/// ones the computer proposes (put away ones too), each with Trust. The paths whole, in mono. Its words behind (?).
struct ComputerFoldersGroup: View {
    var connectionId: String
    @Environment(AppModel.self) private var model
    @State private var vm: ComputerFoldersModel

    init(connectionId: String) {
        self.connectionId = connectionId
        _vm = State(initialValue: ComputerFoldersModel(connectionId: connectionId))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            QuietHelpHeader(title: "Folders", help: FolderText.help, count: vm.trusted.isEmpty ? nil : "\(vm.trusted.count)")
            FlatGroup {
                if vm.trusted.isEmpty {
                    Text(vm.folders == nil ? "…" : "None yet")
                        .textRole(.row)
                        .foregroundStyle(Palette.inkMeta)
                        .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
                        .accessibilityIdentifier("noFolders")
                }
                ForEach(vm.trusted, id: \.self) { folder in
                    FolderRow(folder: folder, symbol: "folder") {
                        Button {
                            Task { await vm.remove(folder, model) }
                        } label: {
                            Image(systemName: "trash")
                                .font(.system(size: 16, weight: .semibold))
                                .foregroundStyle(Palette.inkMeta)
                                .frame(width: 44, height: 44)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(PressDimsStyle())
                        .accessibilityLabel("Remove")
                        .accessibilityIdentifier("removeFolder:\(FolderText.id(folder))")
                    }
                    .accessibilityIdentifier("trustedFolder:\(FolderText.id(folder))")
                }
            }
            if !vm.suggested.isEmpty {
                SectionHeader("Suggested", quiet: true)
                FlatGroup {
                    ForEach(vm.suggested, id: \.self) { folder in
                        FolderRow(folder: folder, symbol: "folder.badge.questionmark") {
                            Button {
                                Task { await vm.trust(folder, model) }
                            } label: {
                                Label { Text("Trust") } icon: { Image(systemName: "checkmark") }
                            }
                            .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                            .accessibilityIdentifier("trustSuggested:\(FolderText.id(folder))")
                        }
                        .accessibilityIdentifier("suggestedFolder:\(FolderText.id(folder))")
                    }
                }
            }
            if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("foldersError") }
        }
        .disabled(vm.busy)
        .task { await vm.load(model) }
        // A card trusted or put away in Activity shows here when the page comes back.
        .onChange(of: model.folderQuestions) { Task { await vm.load(model) } }
    }
}

/// One folder: its symbol on a tile, the path whole in mono (wrapped, never cut), its kind quietly under it, then its
/// action.
private struct FolderRow<Action: View>: View {
    var folder: FolderView
    var symbol: String
    @ViewBuilder var action: Action

    var body: some View {
        HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 9, style: .continuous)
                .fill(Palette.paperSunken)
                .frame(width: 32, height: 32)
                .overlay {
                    Image(systemName: symbol).font(.system(size: 15, weight: .medium)).foregroundStyle(Palette.ink2)
                }
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: FolderText.path(folder.path))
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                Text(FolderText.kindName(folder.kind))
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            action
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
    }
}
