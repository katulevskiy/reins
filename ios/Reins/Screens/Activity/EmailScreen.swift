import SwiftUI

/// One email that an AI was given, opened in full. The phone keeps only which emails were shared, so the text is
/// fetched from Gmail now (and an email deleted since says so). The header from the log shows while it loads.
struct EmailScreen: View {
    var entryId: Int64
    var index: Int
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    private enum Load {
        case loading
        case loaded(EmailContent)
        case failed(String)
    }

    @State private var load: Load = .loading
    @State private var attempt = 0
    /// Set once the core has said it has no such entry (or could not be read); until then a missing entry is read.
    @State private var missing: EntryMissing?

    var body: some View {
        let entry = model.entry(entryId)
        let logged = entry.flatMap { $0.info.messages.indices.contains(index) ? $0.info.messages[index] : nil }
        Group {
            if let entry, let logged {
                ScrollView { content(entry, logged).padding(.bottom, 32) }
            } else if case let .failed(message) = missing {
                EmptyState(symbol: "envelope", title: "Can't open it", message: message)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("emailError")
            } else if entry != nil || missing == .gone {
                EmptyState(symbol: "envelope", title: "Not found", message: "That email is no longer in the history.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("emailGone")
            } else {
                // Older than the history read at once: read from the core by its id.
                Color.clear
            }
        }
        .pageBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: "\(attempt):\(entry != nil)") {
            if entry == nil {
                // Once read it is kept, so this task runs again with the entry (its id changes).
                missing = await EntryMissing.read(entryId, model: model)
                return
            }
            guard let entry, let logged, !logged.id.isEmpty else { return }
            load = .loading
            do {
                load = .loaded(try await model.core.fetchEmail(account: entry.account, messageId: logged.id))
            } catch is CancellationError {
                return
            } catch {
                load = .failed(error.userMessage)
            }
        }
    }

    @ViewBuilder private func content(_ entry: ActivityEntry, _ logged: ActivityMessage) -> some View {
        let email: EmailContent? = if case let .loaded(e) = load { e } else { nil }
        VStack(alignment: .leading, spacing: 0) {
            // The quiet template (Next-More, 02): the service's mark and the subject at 24, then the headers as glyph
            // facts in a flat group.
            HStack(spacing: 12) {
                ServiceAvatar(service: entry.service.isEmpty ? "gmail" : entry.service, size: 36)
                let subject = untrusted(email?.subject ?? logged.subject)
                Text(subject.isEmpty ? "(no subject)" : subject)
                    .textRole(.quietTitle)
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("emailSubject")
            }
            .padding(.horizontal, 20)
            .padding(.top, 10)

            FlatGroup(inset: 32) {
                HeaderLine(glyph: .person, label: "From", value: untrusted(email?.from ?? logged.from)).accessibilityIdentifier("emailFrom")
                if let to = email?.to, !to.isEmpty {
                    HeaderLine(glyph: .mail, label: "To", value: to.map(untrusted).joined(separator: ", ")).accessibilityIdentifier("emailTo")
                }
                if let cc = email?.cc, !cc.isEmpty {
                    HeaderLine(glyph: .mail, label: "Cc", value: cc.map(untrusted).joined(separator: ", "))
                }
                if let account = entry.account, !account.isEmpty {
                    HeaderLine(glyph: .tray, label: "Account", value: account, mono: true)
                }
                HeaderLine(glyph: .timer, label: "Date", value: TimeText.full(email?.date ?? logged.date))
            }
            .padding(.top, 16)

            switch load {
            case .loading:
                HStack(spacing: 12) {
                    ProgressView().tint(Palette.violet)
                    Text("Fetching it from Gmail…").textRole(.body).foregroundStyle(Palette.ink2)
                }
                .padding(24)
                .accessibilityIdentifier("emailLoading")
            case let .failed(message):
                VStack(alignment: .leading, spacing: 12) {
                    Banner(untrusted(message), kind: .warning).accessibilityIdentifier("emailError")
                    Button("Try again") {
                        feedback.play(.tap)
                        attempt += 1
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 44))
                    .frame(maxWidth: 200)
                    .accessibilityIdentifier("emailRetry")
                }
                .padding(16)
            case let .loaded(e):
                let body = untrusted(e.body)
                Text(body.isEmpty ? "(This email has no text.)" : body)
                    .font(RFont.sans(15.5))
                    .lineSpacing(5)
                    .foregroundStyle(Palette.ink)
                    .textSelection(.enabled)
                    .padding(.horizontal, 20)
                    .padding(.top, 18)
                    .accessibilityIdentifier("emailBody")
            }
        }
    }
}

/// A header as a glyph fact on paper (the kit's flat `lrow`): the glyph says what it is, VoiceOver says its name.
private struct HeaderLine: View {
    var glyph: Glyph
    var label: String
    var value: String
    var mono = false

    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(glyph, size: 18).foregroundStyle(Palette.inkMeta).frame(width: 20)
            Text(value)
                .font(mono ? RFont.mono(14.5) : TextRole.row.font())
                .foregroundStyle(Palette.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
        }
        .padding(.vertical, 10)
        .frame(minHeight: 52)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(label): \(value)")
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { EmailScreen(entryId: 10, index: 0) } }
}
#endif
