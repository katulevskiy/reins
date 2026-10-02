import SwiftUI

// What a request is about, by kind: the emails found, an email to send, a permission, accounts, a change elsewhere.

/// The emails (or items) found, each ticked to be shared; covered ones are ticked and locked.
struct MessagesSection: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        let draft = vm.draft
        let allIds = view.messages.map(\.id)
        let allTicked = !allIds.isEmpty && view.messages.allSatisfy { $0.coveredByGrant || draft.selected.contains($0.id) }
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text((view.kind == .fetch ? "Found" : "Emails found") + " (\(view.messages.count))")
                    .font(RFont.sans(13, .semibold))
                    .foregroundStyle(Palette.secondary)
                Spacer()
                if !view.messages.isEmpty {
                    if allTicked {
                        Button("Clear") {
                            vm.draft.selected = []
                            vm.draft.allMail = nil
                            feedback.play(.tap)
                        }
                        .font(RFont.sans(14.5, .medium))
                        .foregroundStyle(Palette.accent)
                        .accessibilityIdentifier("clearAll")
                    } else {
                        Button {
                            vm.draft.selected = Set(allIds)
                            vm.draft.allMail = nil
                            feedback.play(.tap)
                        } label: {
                            Label("Select all", systemImage: "checkmark").font(RFont.sans(14.5, .medium))
                        }
                        .buttonStyle(.bordered)
                        .buttonBorderShape(.capsule)
                        .tint(Palette.accent)
                        .controlSize(.small)
                        .accessibilityIdentifier("selectAll")
                    }
                }
            }
            .padding(.leading, 20)
            .padding(.trailing, 16)
            .padding(.top, 14)
            .padding(.bottom, 6)

            if view.messages.isEmpty {
                Text("Nothing matched.")
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 8)
            } else {
                GroupCard {
                    ForEach(Array(view.messages.enumerated()), id: \.element.id) { i, message in
                        if i > 0 { Hairline(inset: 52) }
                        MessageRow(
                            message: message,
                            checked: message.coveredByGrant || draft.selected.contains(message.id) || draft.allMail != nil,
                            enabled: !message.coveredByGrant && draft.allMail == nil
                        ) { on in
                            if on { vm.draft.selected.insert(message.id) } else { vm.draft.selected.remove(message.id) }
                        }
                    }
                }
            }
        }
    }
}

private struct MessageRow: View {
    var message: MessageView
    var checked: Bool
    var enabled: Bool
    var onChange: (Bool) -> Void

    var body: some View {
        CheckRow(checked: checked, enabled: enabled, onChange: onChange) {
            VStack(alignment: .leading, spacing: 2) {
                if !message.from.isEmpty {
                    Text(untrusted(message.from))
                        .font(RFont.sans(15, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                }
                if !message.subject.isEmpty {
                    Text(untrusted(message.subject)).font(RFont.sans(14.5)).foregroundStyle(Palette.text).lineLimit(2)
                }
                if !message.snippet.isEmpty {
                    Text(untrusted(message.snippet)).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(3)
                }
                let when = [message.date != 0 ? TimeText.dateTime(message.date) : nil, message.coveredByGrant ? "already allowed by a grant" : nil]
                    .compactMap { $0 }.joined(separator: " · ")
                if !when.isEmpty {
                    Text(when).font(RFont.sans(12)).foregroundStyle(Palette.tertiary).padding(.top, 2)
                }
                if message.sensitive {
                    TintTag(text: "Looks like a code or a password", tint: Palette.warning).padding(.top, 4)
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityIdentifier("message:\(message.id)")
    }
}

/// An email to be sent, in full: who it goes to, the subject and every word of it.
struct EmailPreview: View {
    var email: EmailView
    /// On its own card (the approval sheet), or bare inside a group (an activity entry).
    var framed = true

    var body: some View {
        if framed {
            Card(padding: 16) { fields }.accessibilityContainer("emailPreview")
        } else {
            fields.padding(16).frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private var fields: some View {
        VStack(alignment: .leading, spacing: 4) {
            Caption("To")
            ForEach(Array(email.to.enumerated()), id: \.offset) { _, a in address(a) }
            if !email.cc.isEmpty {
                Caption("Cc").padding(.top, 6)
                ForEach(Array(email.cc.enumerated()), id: \.offset) { _, a in address(a) }
            }
            Caption("Subject").padding(.top, 6)
            Text(untrusted(email.subject)).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
            Caption("Message").padding(.top, 6)
            Text(untrusted(email.body)).font(RFont.sans(15)).foregroundStyle(Palette.text).lineSpacing(3)
        }
        .textSelection(.enabled)
    }

    private func address(_ text: String) -> some View {
        Text(untrusted(text)).font(RFont.mono(14)).foregroundStyle(Palette.text).environment(\.layoutDirection, .leftToRight)
    }
}

/// What will be done in another integration, spelled out; nothing happens until the user approves.
struct WritePreview: View {
    var view: ApprovalView

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 6) {
                if let first = view.resources.first { Caption(untrusted(first.label)) }
                ForEach(Array(view.preview.enumerated()), id: \.offset) { i, line in
                    Text(untrusted(line))
                        .font(i == 0 ? RFont.sans(17, .semibold) : RFont.sans(15))
                        .foregroundStyle(i == 0 ? Palette.text : Palette.secondary)
                        .lineSpacing(2)
                        .accessibilityIdentifier("previewLine:\(i)")
                }
            }
            .textSelection(.enabled)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("writePreview")
    }
}

/// A permission an AI asks for: set apart in the accent colour so it cannot be mistaken for a one-off request.
struct GrantRequestCard: View {
    var grant: GrantRequestView
    var label: String

    var body: some View {
        let (tone, toneText): (Color, String) = switch grant.breadth {
        case "everything": (Palette.danger, "Everything")
        case "broad": (Palette.warning, "Broad")
        default: (Palette.success, "Narrow")
        }
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "checkmark.shield.fill").font(.system(size: 18, weight: .semibold)).foregroundStyle(Palette.accent)
                Text("PERMISSION REQUEST")
                    .font(RFont.sans(12.5, .semibold))
                    .tracking(0.8)
                    .foregroundStyle(Palette.accent)
                    .frame(maxWidth: .infinity, alignment: .leading)
                TintTag(text: toneText, tint: tone)
            }
            Text(grant.action == "send" ? "Send emails without asking" : "Read emails without asking")
                .font(RFont.sans(19, .semibold))
                .foregroundStyle(Palette.text)
            ForEach(Array(grant.lines.enumerated()), id: \.offset) { _, line in
                Text("• \(untrusted(line))").font(RFont.sans(15)).foregroundStyle(Palette.text)
            }
            Text("For \(TimeText.duration(Int64(grant.durationSecs)))" + (grant.maxUses.map { " · at most \($0) uses" } ?? ""))
                .font(RFont.sans(15, .medium))
                .foregroundStyle(Palette.text)
            if !grant.reason.trimmingCharacters(in: .whitespaces).isEmpty {
                QuoteBox(caption: "\(untrusted(label)) says", text: untrusted(grant.reason))
            }
            Text("Nothing is shared now. Requests that match this are answered without asking until it ends; you can revoke it any time in Grants.")
                .font(RFont.sans(13))
                .foregroundStyle(Palette.secondary)
        }
        .padding(18)
        .background(Palette.accent.opacity(0.09), in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.accent.opacity(0.55), lineWidth: 1.5))
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("grantCard")
    }
}

/// The accounts an AI would be shown, each with a round tick. Unticking one crosses it out and greys it: the AI does
/// not get that address (it is only told how many it did not get). Accounts it can already see are ticked and locked.
struct AccountsCard: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView

    var body: some View {
        let who = untrusted(view.connectionLabel)
        let service = serviceName(view.service)
        VStack(alignment: .leading, spacing: 0) {
            Text(view.sharedAccounts.isEmpty
                ? "\(who) will see the \(service) addresses you tick. It will not see anything in them, and it is told how many it did not get."
                : "\(who) can already see \(view.sharedAccounts.count) of your \(service) addresses. Tick the ones to add.")
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.secondary)
                .padding(.horizontal, 4)
                .padding(.vertical, 8)
            Card(padding: 0) {
                VStack(spacing: 0) {
                    ForEach(Array(view.accounts.enumerated()), id: \.element) { i, address in
                        if i > 0 { Hairline(inset: 68) }
                        let locked = view.sharedAccounts.contains(address)
                        let ticked = locked || vm.draft.selected.contains(address)
                        CheckRow(checked: ticked, enabled: !locked, onChange: { on in
                            if on { vm.draft.selected.insert(address) } else { vm.draft.selected.remove(address) }
                        }) {
                            HStack(spacing: 14) {
                                BlobAvatar(seed: address, size: 36)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(address)
                                        .font(RFont.sans(16, .medium))
                                        .foregroundStyle(Palette.text)
                                        .strikethrough(!ticked, color: Palette.secondary)
                                        .lineLimit(1)
                                        .truncationMode(.middle)
                                    if locked { Text("Already shared").font(RFont.sans(12)).foregroundStyle(Palette.tertiary) }
                                }
                            }
                            .opacity(ticked ? 1 : 0.42)
                            .animation(.easeInOut(duration: 0.28), value: ticked)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .accessibilityIdentifier("shared:\(i)")
                    }
                }
            }
            Text("Allowing it here also lets \(who) ask again later without bothering you, for the time you choose under More options (a month to start with). Asking for an address you kept private still needs your OK.")
                .font(RFont.sans(13))
                .foregroundStyle(Palette.tertiary)
                .padding(.horizontal, 4)
                .padding(.vertical, 10)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("accountsCard")
    }
}
