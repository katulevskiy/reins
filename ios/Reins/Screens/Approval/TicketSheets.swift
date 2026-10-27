import SwiftUI

// The approval sheet's words to the agent (decisions v2, 03 §5.2): a Why? it answers on the card, and a note it gets
// with a denial. Both are short and the reviewer's own; neither is needed for a plain answer.

/// The dock's row for a request that can wait (Later), be asked about (Why?) or be denied with a note.
struct TicketDockRow: View {
    var vm: ApprovalModel
    var item: PendingItem
    @Environment(AppModel.self) private var model
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    /// A Why? that waits for the agent's answer is not asked again.
    private var canAsk: Bool { item.ticket.map { $0.question == nil || $0.agentReply != nil } ?? true }

    var body: some View {
        HStack(spacing: 8) {
            if TicketInbox.canLater(item) {
                Button {
                    Task { await vm.decideLater(model) }
                } label: {
                    Label { Text("Later") } icon: { Image(systemName: "clock.arrow.circlepath") }
                }
                .buttonStyle(SignalButtonStyle(.soft, size: .small))
                .accessibilityIdentifier("later")
            }
            if canAsk {
                Button {
                    nav.overlay = .why
                    feedback.play(.open)
                } label: {
                    Label { Text("Why?") } icon: { Image(systemName: "questionmark.bubble") }
                }
                .buttonStyle(SignalButtonStyle(.soft, size: .small))
                .accessibilityIdentifier("askWhy")
            }
            Button {
                nav.overlay = .denyNote
                feedback.play(.open)
            } label: {
                Label { Text("Note") } icon: { Image(systemName: "text.bubble") }
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small))
            .accessibilityLabel("Deny with note")
            .accessibilityIdentifier("denyWithNote")
        }
        .disabled(vm.busy)
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
    }
}

/// A short text to the agent: a title, the field, and the button that sends it.
private struct AgentTextSheet: View {
    var title: String
    var placeholder: String
    var button: String
    var symbol: String
    var kind: SignalButtonKind
    /// Whether the button needs words (a Why?) or works without (a denial).
    var needsText: Bool
    var identifier: String
    var send: (String) -> Void
    @State private var text = ""
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                Text(title).textRole(.cardAsk).foregroundStyle(Palette.ink).accessibilityAddTraits(.isHeader)
                Spacer()
                RoundHeaderButton(glyph: .close, label: "Close") { dismiss() }
                    .accessibilityIdentifier("closeOverlay")
            }
            TextField(placeholder, text: $text, axis: .vertical)
                .lineLimit(3...6)
                .font(RFont.sans(16))
                .foregroundStyle(Palette.ink)
                .focused($focused)
                .padding(14)
                .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .onChange(of: text) { _, new in
                    if new.count > TicketInbox.maxText { text = String(new.prefix(TicketInbox.maxText)) }
                }
                .accessibilityIdentifier("\(identifier)Text")
            Button {
                dismiss()
                send(text)
            } label: {
                Label { Text(button) } icon: { Image(systemName: symbol) }
            }
            .buttonStyle(SignalButtonStyle(kind))
            .disabled(needsText && TicketInbox.question(text) == nil)
            .accessibilityIdentifier(identifier)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        .padding(.top, 22)
        .background(Palette.paper.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .onAppear { focused = true }
    }
}

/// Why?: the question goes to the agent; the request waits for its answer under "Waiting for you".
struct WhySheet: View {
    var vm: ApprovalModel
    @Environment(AppModel.self) private var model

    var body: some View {
        AgentTextSheet(
            title: "Why?", placeholder: "Ask the agent", button: "Ask", symbol: "paperplane", kind: .inverse, needsText: true,
            identifier: "sendWhy"
        ) { question in
            Task { await vm.askWhy(model, question: question) }
        }
    }
}

/// Deny with a note the agent receives; without words it is a plain denial.
struct DenyNoteSheet: View {
    var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model

    var body: some View {
        AgentTextSheet(
            title: ApprovalWords.denyLabel(view), placeholder: "Note to the agent (optional)", button: ApprovalWords.denyLabel(view),
            symbol: "nosign", kind: .danger, needsText: false, identifier: "sendDenyNote"
        ) { note in
            Task { await vm.deny(model, note: note) }
        }
    }
}
