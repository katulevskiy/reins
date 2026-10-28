import SwiftUI

// "Trust this folder?": a folder a paired computer proposes for what it may touch (its `[workspace]`), as a card with
// the waiting ones, in the question cards' shell. The computer and the folder's kind, the path exactly as the computer
// gave it (whole, wrapped), then Trust and Not now. Trust loosens, so the core asks for the owner check; Not now puts
// the card away and the folder stays suggested on the computer's page. Listed only, never notified.

/// The folder cards and rows in words (pure, unit-tested).
enum FolderText {
    /// "Projects" for a project root, "Scratch" for a scratch folder.
    static func kindName(_ kind: FolderKindChoice) -> String {
        switch kind {
        case .root: "Projects"
        case .scratch: "Scratch"
        }
    }

    static func kindId(_ kind: FolderKindChoice) -> String {
        switch kind {
        case .root: "root"
        case .scratch: "scratch"
        }
    }

    /// One card's id: the computer, the kind and the path (a path can be proposed as both kinds).
    static func id(_ q: FolderQuestion) -> String { "\(q.connectionId):\(kindId(q.kind)):\(q.path)" }

    /// A row's id under a computer's Folders.
    static func id(_ f: FolderView) -> String { "\(kindId(f.kind)):\(f.path)" }

    /// The path from the computer, whole: nothing hidden or reordering, never cut.
    static func path(_ path: String) -> String { untrusted(path) }

    /// The quiet line under the ask: "M2A · Projects".
    static func from(_ q: FolderQuestion) -> String {
        let name = untrustedName(q.computer, limit: WaitCardIdentity.limit)
        return name.isEmpty ? kindName(q.kind) : "\(name) · \(kindName(q.kind))"
    }

    static let title = "Trust this folder?"

    /// The card's field: Auto's violet, like the question cards.
    static let field = FieldKind.grant

    /// Behind the (?) of a computer's Folders.
    static let help = "Folders the computer may work in. Trusting one asks you first. Removing one applies at once."
}

/// The computers' open folder proposals under what waits: the newest as a card, "+N" opens the rest.
struct FolderCards: View {
    var questions: [FolderQuestion]
    @State private var expanded = false
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 10) {
            ForEach(expanded ? questions : Array(questions.prefix(1)), id: \.self) { q in
                FolderCard(question: q)
                    .transition(reduceMotion ? .opacity : .asymmetric(insertion: .move(edge: .top).combined(with: .opacity), removal: .opacity))
            }
            if questions.count > 1 {
                Button {
                    withAnimation(Motion.of(Motion.easeOut(Motion.fold), reduce: reduceMotion)) { expanded.toggle() }
                    feedback.play(.expand(expanded))
                } label: {
                    Group {
                        if expanded {
                            Image(systemName: "chevron.up").font(.system(size: 14, weight: .semibold))
                        } else {
                            Text("+\(questions.count - 1)").font(RFont.sans(12, .bold)).monospacedDigit()
                        }
                    }
                    .foregroundStyle(Palette.onInverse)
                    .padding(.horizontal, 9)
                    .frame(minWidth: 26, minHeight: 22)
                    .background(Palette.inverse, in: Capsule())
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel(expanded ? "Show one" : "\(questions.count - 1) more folders")
                .accessibilityIdentifier("foldersToggle")
            }
        }
        .padding(.horizontal, 16)
        .animation(Motion.of(Motion.cardSpring(0.32), reduce: reduceMotion), value: questions.map(FolderText.id))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("folderQuestions")
    }
}

/// One proposal: the folder symbol; "Trust this folder?"; the computer and kind, quietly; the path in mono, whole;
/// Trust (white) and Not now.
struct FolderCard: View {
    var question: FolderQuestion
    @Environment(AppModel.self) private var model
    @State private var busy = false
    @State private var error: String?

    private var id: String { FolderText.id(question) }

    var body: some View {
        WaitCardShell(
            kind: FolderText.field, label: "Folder",
            ask: AskText([AskText.Run(text: FolderText.title)], role: .cardAsk), askIdentifier: "folderTitle:\(id)"
        ) {
            Image(systemName: "folder.badge.questionmark")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Palette.onField)
                .frame(width: 34, height: 34)
                .background(Palette.onFieldWash, in: Circle())
        } timer: {
            EmptyView()
        } actions: {
            VStack(alignment: .leading, spacing: 12) {
                Text(verbatim: FolderText.from(question))
                    .font(RFont.sans(13.5, .medium))
                    .foregroundStyle(Palette.onField2)
                    .lineLimit(1)
                    .padding(.top, -4)
                    .accessibilityIdentifier("folderFrom:\(id)")
                Text(verbatim: FolderText.path(question.path))
                    .font(RFont.mono(14))
                    .foregroundStyle(Palette.onField)
                    .fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("folderPath:\(id)")
                if let error {
                    Text(error).font(RFont.sans(13.5, .medium)).foregroundStyle(Palette.onField).lineLimit(3)
                        .accessibilityIdentifier("folderError")
                }
                HStack(spacing: 6) {
                    Button("Trust") { Task { await trust() } }
                        .buttonStyle(SignalButtonStyle(.onField, size: .small))
                        .accessibilityIdentifier("trustFolder:\(id)")
                    Button("Not now") { Task { await dismiss() } }
                        .buttonStyle(SignalButtonStyle(.onFieldWash, size: .small))
                        .accessibilityIdentifier("dismissFolder:\(id)")
                }
            }
        }
        .disabled(busy)
        .accessibilityIdentifier("folderQuestion:\(id)")
    }

    private func trust() async {
        busy = true
        error = await model.trustFolder(connectionId: question.connectionId, path: question.path, kind: question.kind)
        busy = false
    }

    private func dismiss() async {
        busy = true
        error = await model.dismissFolder(connectionId: question.connectionId, path: question.path, kind: question.kind)
        busy = false
    }
}
