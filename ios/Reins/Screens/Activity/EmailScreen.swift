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

    var body: some View {
        let entry = model.activity.first { $0.id == entryId }
        let logged = entry.flatMap { $0.info.messages.indices.contains(index) ? $0.info.messages[index] : nil }
        Group {
            if let entry, let logged {
                ScrollView { content(entry, logged).padding(.bottom, 32) }
            } else {
                EmptyState(symbol: "envelope", title: "Not found", message: "That email is no longer in the history.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .accessibilityIdentifier("emailGone")
            }
        }
        .pageBackground()
        .navigationTitle("Email")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: attempt) {
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
            VStack(alignment: .leading, spacing: 0) {
                let subject = untrusted(email?.subject ?? logged.subject)
                Text(subject.isEmpty ? "(no subject)" : subject)
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityIdentifier("emailSubject")
                ConnectorTags(service: entry.service, account: entry.account).padding(.top, 10)
                HeaderLine(label: "From", value: untrusted(email?.from ?? logged.from)).padding(.top, 10).accessibilityIdentifier("emailFrom")
                if let to = email?.to, !to.isEmpty {
                    HeaderLine(label: "To", value: to.map(untrusted).joined(separator: ", ")).accessibilityIdentifier("emailTo")
                }
                if let cc = email?.cc, !cc.isEmpty {
                    HeaderLine(label: "Cc", value: cc.map(untrusted).joined(separator: ", "))
                }
                HeaderLine(label: "Date", value: TimeText.full(email?.date ?? logged.date))
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 8)

            switch load {
            case .loading:
                HStack(spacing: 12) {
                    ProgressView().tint(Palette.accent)
                    Text("Fetching it from Gmail…").font(RFont.sans(15)).foregroundStyle(Palette.secondary)
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
                    .font(RFont.sans(16))
                    .foregroundStyle(Palette.text)
                    .lineSpacing(4)
                    .textSelection(.enabled)
                    .padding(.horizontal, 20)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("emailBody")
            }
        }
    }
}

private struct HeaderLine: View {
    var label: String
    var value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 0) {
            Text(label).font(RFont.sans(13, .medium)).foregroundStyle(Palette.tertiary).frame(width: 52, alignment: .leading)
            Text(value)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.text)
                .frame(maxWidth: .infinity, alignment: .leading)
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
        }
        .padding(.top, 6)
        .accessibilityElement(children: .combine)
    }
}

#if DEBUG
#Preview {
    PreviewHost { NavigationStack { EmailScreen(entryId: 10, index: 0) } }
}
#endif
