import SwiftUI
import UniformTypeIdentifiers

/// What the export of the decision history says and names (pure, unit-tested).
enum ActivityExportText {
    static let title = "Export decision history"
    static let message = "Includes up to the last 5,000 decisions on this phone. Message contents aren't included. The file isn't encrypted, so keep it somewhere safe."
    static let csvButton = "Spreadsheet (CSV)"
    static let jsonButton = "JSON"
    static let empty = "No decisions to export yet"
    static let unavailable = "Set a passcode on this iPhone to export. Exporting always needs Face ID, Touch ID or the passcode."
    /// The system prompt's reason, under Face ID or the passcode.
    static let reason = "Export your decision history"

    /// "Exported 1 decision" / "Exported 2,400 decisions".
    static func exported(_ entries: UInt32) -> String {
        "Exported \(Int(entries).formatted()) \(entries == 1 ? "decision" : "decisions")"
    }

    /// The type the save sheet writes for a format.
    static func contentType(_ format: ActivityExportFormat) -> UTType {
        switch format {
        case .csv: .commaSeparatedText
        case .json: .json
        }
    }

    /// The suggested name without its extension: the save sheet adds the type's own, so `x.csv` would end `x.csv.csv`.
    static func baseName(_ fileName: String, format: ActivityExportFormat) -> String {
        let ext = switch format {
        case .csv: ".csv"
        case .json: ".json"
        }
        let trimmed = fileName.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.count > ext.count, trimmed.lowercased().hasSuffix(ext) { return String(trimmed.dropLast(ext.count)) }
        return trimmed.isEmpty ? "reins-decisions" : trimmed
    }
}

/// The exported text, held only until the save sheet is done; it is written as UTF-8 to where the person picks.
struct ActivityExportDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.commaSeparatedText, .json] }

    var contents: String
    var type: UTType

    init(contents: String, type: UTType) {
        self.contents = contents
        self.type = type
    }

    init(configuration: ReadConfiguration) throws {
        guard let data = configuration.file.regularFileContents, let text = String(data: data, encoding: .utf8) else {
            throw CocoaError(.fileReadCorruptFile)
        }
        contents = text
        type = configuration.contentType
    }

    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper {
        FileWrapper(regularFileWithContents: Data(contents.utf8))
    }
}

/// Settings > Data > Export decision history: the owner confirms, picks a format, and saves the file where they choose.
struct ExportDecisionHistoryRow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var choosing = false
    @State private var busy = false
    @State private var saving = false
    @State private var document: ActivityExportDocument?
    @State private var type: UTType = .commaSeparatedText
    @State private var filename = "reins-decisions"
    @State private var entries: UInt32 = 0

    var body: some View {
        SettingsTileButton(ActivityExportText.title, glyph: .download, id: "exportDecisionHistory", chevron: false) {
            guard !busy else { return }
            choosing = true
        }
        .disabled(busy)
        .accessibilityHint("Save this phone's Activity as a file")
        .confirmationDialog(ActivityExportText.title, isPresented: $choosing, titleVisibility: .visible) {
            Button(ActivityExportText.csvButton) { start(.csv) }
            Button(ActivityExportText.jsonButton) { start(.json) }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(ActivityExportText.message)
        }
        .presentationFeedback(choosing)
        .fileExporter(
            isPresented: $saving, document: document, contentType: type, defaultFilename: filename,
            onCompletion: { result in
                switch result {
                case .success:
                    feedback.play(.copied)  // the quiet "saved" cue
                    model.notice = ActivityExportText.exported(entries)
                case let .failure(error):
                    feedback.play(.error)
                    model.notice = error.userMessage
                }
                clear()
            }
        )
        // Swiped away or cancelled: the sheet closes without onCompletion, so drop the held contents here too.
        .onChange(of: saving) { _, shown in
            if !shown { document = nil }
        }
    }

    private func clear() {
        document = nil
        entries = 0
    }

    private func start(_ format: ActivityExportFormat) {
        guard !busy else { return }
        busy = true
        feedback.play(.tap)
        Task {
            defer { busy = false }
            switch await OwnerPrompt.confirm(model, reason: ActivityExportText.reason) {
            case .confirmed:
                break
            case .cancelled:
                return
            case .unavailable:
                model.notice = ActivityExportText.unavailable
                return
            }
            do {
                let export = try await model.core.exportActivity(format: format, query: nil)
                if export.entries == 0 {
                    model.notice = ActivityExportText.empty
                    return
                }
                type = ActivityExportText.contentType(format)
                filename = ActivityExportText.baseName(export.fileName, format: format)
                entries = export.entries
                document = ActivityExportDocument(contents: export.contents, type: type)
                saving = true
            } catch {
                feedback.play(.error)
                model.notice = error.userMessage
            }
        }
    }
}
