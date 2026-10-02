import SwiftUI

// Rows and tags for operations (the Android app's ui/common Rows.kt, Connections.kt and design/Tags.kt).

/// The integration and the account an operation used: "Gmail" "me@gmail.com".
struct ConnectorTags: View {
    var service: String
    var account: String?
    /// Overrides the integration's name (an MCP server's own).
    var name: String?

    var body: some View {
        FlowRow(spacing: 6, lineSpacing: 6) {
            if !service.trimmingCharacters(in: .whitespaces).isEmpty { ServiceTag(text: name ?? serviceName(service)) }
            if let account, !account.trimmingCharacters(in: .whitespaces).isEmpty { AccountTag(text: account) }
        }
    }
}

/// The connection an entry belongs to: by id, or, for entries logged before ids were recorded (an empty id), by the
/// label it had then, when exactly one connection carries it.
func findConnection(_ connections: [ConnectionView], id: String, label: String) -> ConnectionView? {
    if let c = connections.first(where: { $0.id == id }) { return c }
    guard id.isEmpty else { return nil }
    let named = connections.filter { $0.label == label }
    return named.count == 1 ? named[0] : nil
}

/// An AI connection's avatar, with the icon the user picked for it.
struct ConnectionIcon: View {
    var connectionId: String
    var label: String
    var size: CGFloat = 40
    @Environment(AppModel.self) private var model

    var body: some View {
        ConnectionAvatar(
            label: untrusted(label),
            pick: findConnection(model.connections, id: connectionId, label: label)?.icon,
            size: size
        )
    }
}

/// How an operation ended, as a coloured pill.
struct OutcomeTag: View {
    var outcome: String

    var body: some View {
        switch outcome {
        case "denied": OutcomePill(text: "Denied", tone: .bad)
        case "error": TintTag(text: "Failed", tint: Palette.warning)
        case "granted": OutcomePill(text: "Granted", tone: .accent)
        case "sent": OutcomePill(text: "Sent")
        case "released": OutcomePill(text: "Allowed")
        default: OutcomePill(text: outcome.prefix(1).uppercased() + outcome.dropFirst())
        }
    }

    static func word(_ outcome: String) -> String {
        switch outcome {
        case "denied": "Denied"
        case "error": "Failed"
        case "granted": "Granted"
        case "sent": "Sent"
        case "released": "Allowed"
        default: outcome.prefix(1).uppercased() + outcome.dropFirst()
        }
    }
}

/// The small mark on entries that Autopilot, a bypass or Lockdown decided.
struct DecidedByBadge: View {
    var decidedBy: String

    var body: some View {
        let (label, symbol, tint): (String, String, Color) = switch decidedBy {
        case "bypass": ("Bypass", "bolt.fill", Palette.danger)
        case "lockdown": ("Lockdown", "lock.fill", Palette.warning)
        default: ("Autopilot", "sparkles", Palette.accent)
        }
        HStack(spacing: 4) {
            Image(systemName: symbol).font(.system(size: 10, weight: .bold))
            Text(label).font(RFont.sans(11.5, .semibold)).lineLimit(1)
        }
        .foregroundStyle(tint)
        .padding(.leading, 6)
        .padding(.trailing, 8)
        .padding(.vertical, 3)
        .background(tint.opacity(0.12), in: Capsule())
    }
}

/// One operation in a list: what it was, who asked, which account, when, and how it ended.
struct ActivityRow: View {
    var entry: ActivityEntry
    var selected = false

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            ActionTile(kind: ActionKind.of(entry.action), count: Int(entry.count), size: 44)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .center, spacing: 8) {
                    ConnectionIcon(connectionId: entry.connectionId, label: entry.connectionLabel, size: 20)
                    Text(entry.headline)
                        .font(RFont.sans(16, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                if !entry.detail.isEmpty {
                    Text(untrusted(entry.detail))
                        .font(RFont.sans(14))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(2)
                }
                ConnectorTags(service: entry.service, account: entry.account).padding(.top, 3)
            }
            VStack(alignment: .trailing, spacing: 6) {
                Text(TimeText.relative(entry.at))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                OutcomeTag(outcome: entry.outcome)
                if !entry.decidedBy.isEmpty { DecidedByBadge(decidedBy: entry.decidedBy) }
            }
            .fixedSize()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .background(selected ? Palette.accentSoft : Color.clear)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// A file the server holds, as it saw the bytes: its name (from the AI), size, type and SHA-256, and a preview: the
/// start of a text file, an image, or what kind of file it is.
struct FileCard: View {
    var blob: BlobView

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 10) {
                    Image(systemName: "tray.full")
                        .font(.system(size: 17, weight: .medium))
                        .foregroundStyle(Palette.secondary)
                    Text(name)
                        .font(RFont.sans(16.5, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                Text(FileText.size(blob.size) + " · " + untrusted(blob.contentType))
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .lineLimit(1)
                Text("SHA-256 " + FileText.shortSha(blob.sha256))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
                preview
            }
        }
    }

    private var name: String {
        let n = untrusted(blob.name)
        return n.isEmpty ? "(no name)" : n
    }

    @ViewBuilder private var preview: some View {
        let text = blob.previewText.map(untrusted).flatMap { $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : $0 }
        if let data = blob.previewImage, let image = UIImage(data: data) {
            Image(uiImage: image)
                .resizable()
                .scaledToFit()
                .frame(maxWidth: .infinity, maxHeight: 240)
                .background(Palette.controlFill)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .padding(.top, 6)
                .accessibilityLabel("Preview of \(name)")
        } else if let text, FileText.isText(blob.contentType) {
            Text(text)
                .font(RFont.mono(12.5))
                .foregroundStyle(Palette.text)
                .lineLimit(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .environment(\.layoutDirection, .leftToRight)
                .padding(.top, 6)
        } else if let text {
            Text(text)
                .font(RFont.sans(14, .medium))
                .foregroundStyle(Palette.secondary)
                .lineLimit(2)
                .padding(.top, 4)
        }
    }
}
