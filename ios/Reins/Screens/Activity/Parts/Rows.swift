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

/// A connection's avatar as it shows everywhere, looked up by id; the one its name suggests when it is gone.
struct ConnectionIcon: View {
    var connectionId: String
    var label: String
    var size: CGFloat = 40
    @Environment(AppModel.self) private var model

    var body: some View {
        if let connection = findConnection(model.connections, id: connectionId, label: label) {
            ConnectionAvatar(connection: connection, size: size)
        } else {
            ConnectionAvatar(label: untrusted(label), pick: nil, size: size)
        }
    }
}

/// How an operation ended, as a coloured pill.
struct OutcomeTag: View {
    var outcome: String

    var body: some View {
        switch outcome {
        case "denied": OutcomePill(text: "Denied", tone: .bad)
        case "error": TintTag(text: "Failed", tint: Palette.warning)
        case "started": TintTag(text: "Awaiting outcome", tint: Palette.warning)
        case "unknown": TintTag(text: "Outcome unknown", tint: Palette.warning)
        case "succeeded_withheld": TintTag(text: "Response withheld", tint: Palette.warning)
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
        case "started": "Awaiting outcome"
        case "unknown": "Outcome unknown"
        case "succeeded_withheld": "Completed; response withheld"
        case "granted": "Granted"
        case "sent": "Sent"
        case "released": "Allowed"
        default: outcome.prefix(1).uppercased() + outcome.dropFirst()
        }
    }
}

/// An entry in words and marks: its sentence, the day it belongs to, and how it ended (pure, unit-tested).
enum EntryText {
    /// "Read 2 emails", "Search Gmail · invoice", "Access granted": one line of what happened.
    static func title(_ entry: ActivityEntry) -> String {
        if ActionKind.of(entry.action) == .grant {
            let full = entryTitle(label: "", action: entry.action, count: 1, service: entry.service, outcome: entry.outcome)
            return full.hasPrefix(": ") ? String(full.dropFirst(2)) : full
        }
        return operationTitle(action: entry.action, count: Int(entry.count), service: entry.service, title: entry.opTitle, op: entry.op)
    }

    /// How an entry ended, as the history row's trailing mark.
    enum Mark: Equatable { case approved, denied, failed, auto, grant, permission, overnight, bypass, lockdown, started, unknown, withheld }

    static func mark(_ entry: ActivityEntry) -> Mark {
        switch entry.outcome {
        case "started": return .started
        case "unknown": return .unknown
        case "succeeded_withheld": return .withheld
        case "error": return .failed
        default: break
        }
        switch entry.decidedBy {
        case "account_permission", "temporary_permission": return entry.outcome == "denied" ? .denied : .permission
        case "autopilot": return .auto
        case "overnight": return .overnight
        case "bypass": return .bypass
        case "lockdown": return .lockdown
        default: break
        }
        if entry.grantId != nil && entry.outcome != "denied" { return .grant }
        switch entry.outcome {
        case "denied": return .denied
        case "error": return .failed
        default: return .approved
        }
    }

    /// "TODAY", "YESTERDAY", else the date ("Mon, Oct 12").
    static func day(_ start: Date, now: Date, calendar: Calendar = .current) -> String {
        if calendar.isDate(start, inSameDayAs: now) { return "Today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: now), calendar.isDate(start, inSameDayAs: yesterday) { return "Yesterday" }
        let sameYear = calendar.component(.year, from: start) == calendar.component(.year, from: now)
        return sameYear
            ? start.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day())
            : start.formatted(date: .abbreviated, time: .omitted)
    }

    /// The line under a row's title: who asked and when, "M2A · 5 min ago"; with the agent that asked on it when it
    /// named itself, "Claude Code · M2A · 5 min ago".
    static func byline(_ entry: ActivityEntry, now: Int64) -> String {
        byline(label: entry.connectionLabel, agent: entry.request?.agent, time: time(entry.at, now: now))
    }

    static func byline(label: String, agent: String?, time: String) -> String {
        let label = untrusted(label)
        let agent = untrusted(agent ?? "").trimmingCharacters(in: .whitespaces)
        let who = agent.isEmpty || agent == label ? [label] : [agent, label]
        return (who.filter { !$0.isEmpty } + [time]).joined(separator: " · ")
    }

    /// The time on the row: "5 min ago" today, "18:20" before.
    static func time(_ at: Int64, now: Int64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(at))
        if Calendar.current.isDate(date, inSameDayAs: Date(timeIntervalSince1970: TimeInterval(now))) { return TimeText.relative(at, now: now) }
        return date.formatted(date: .omitted, time: .shortened)
    }

    /// The entries by day, newest first, keeping their order.
    static func days(_ entries: [ActivityEntry], calendar: Calendar = .current) -> [(day: Date, entries: [ActivityEntry])] {
        var out: [(day: Date, entries: [ActivityEntry])] = []
        for entry in entries {
            let day = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(entry.at)))
            if let last = out.indices.last, out[last].day == day {
                out[last].entries.append(entry)
            } else {
                out.append((day, [entry]))
            }
        }
        return out
    }
}

/// The trailing mark of a river row, always with its word where colour would say it alone: a green check, "Denied",
/// "Auto" in violet, a key for a grant, "Overnight", "Bypass", "Lockdown".
struct EntryMark: View {
    var mark: EntryText.Mark

    var body: some View {
        Group {
            switch mark {
            case .approved: GlyphIcon(.check, size: 18).foregroundStyle(Palette.success)
            case .permission: GlyphIcon(.key, size: 17).foregroundStyle(Palette.violet)
            case .grant: GlyphIcon(.key, size: 17).foregroundStyle(Palette.violet)
            case .denied: word("Denied", Palette.danger)
            case .failed: word("Failed", Palette.warning)
            case .started: word("Awaiting outcome", Palette.warning)
            case .unknown: word("Outcome unknown", Palette.warning)
            case .withheld: word("Response withheld", Palette.warning)
            case .auto: word("Auto", Palette.violet)
            case .overnight: word(AutopilotText.overnightName, Palette.violet)
            case .bypass: word("Bypass", Palette.danger)
            case .lockdown: word("Lockdown", Palette.warning)
            }
        }
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Self.spoken(mark))
    }

    private func word(_ text: String, _ tint: Color) -> some View {
        Text(text).font(RFont.sans(12, .semibold)).foregroundStyle(tint)
    }

    static func spoken(_ mark: EntryText.Mark) -> String {
        switch mark {
        case .approved: "Approved"
        case .permission: "Allowed by an explicit permission"
        case .grant: "Under a grant"
        case .denied: "Denied"
        case .failed: "Failed"
        case .started: "Awaiting outcome"
        case .unknown: "Outcome unknown; do not repeat"
        case .withheld: "Completed; response withheld; do not repeat"
        case .auto: "Decided by Autopilot"
        case .overnight: "Decided during \(AutopilotText.overnightName)"
        case .bypass: "Approved by a bypass"
        case .lockdown: "Denied by Lockdown"
        }
    }
}

/// What an entry is, as the river shows it: its kind's field (or none) and the glyph on its tile (pure, unit-tested).
enum EntryLook {
    /// The entry written (ASK-TEXT): its sentence, told as it ended, and its icon. `who` starts a past sentence.
    static func line(_ entry: ActivityEntry, who: String? = nil) -> AskLine { AskLabels.entry(entry, field: field(entry), who: who) }

    static func field(_ entry: ActivityEntry) -> FieldKind? {
        let verb = (entry.label?.action ?? entry.op).lowercased()
        if verb.hasPrefix("delete") || entry.opTitle.hasPrefix("Delete") { return .delete }
        return FieldKind.of(ActionKind.of(entry.action))
    }

    static func glyph(_ entry: ActivityEntry) -> Glyph {
        let provider = entry.label?.provider ?? entry.service
        let verb = (entry.label?.action ?? entry.op).lowercased()
        if verb.hasPrefix("delete") || entry.opTitle.hasPrefix("Delete") { return .trash }
        if verb.contains("push") || (provider == "github" && ActionKind.of(entry.action) == .write) { return .gitBranch }
        switch ActionKind.of(entry.action) {
        case .search, .list: return .search
        case .read, .headlines: return provider.isEmpty || provider == "gmail" ? .mail : .eye
        case .send: return .send
        case .write: return .pencil
        case .grant: return .key
        case .accounts: return .person
        case .pair: return .link
        case .join: return .phone
        case .upload: return .file
        case .other: return .shield
        }
    }
}

/// One operation in the river (iOS-Activity.png): its kind tile, what happened in a line or two, who and when, and how
/// it ended. Hairlines, never cards. The newest row glows faintly in its kind colour for a moment after an answer
/// folds into it.
struct HistoryRow: View {
    var entry: ActivityEntry
    var selected = false
    var glowing = false
    var now: Int64 = Timers.nowSeconds()
    /// The words a search found: set apart in the title.
    var terms: [String] = []
    /// Where the search matched outside the title, as one line under the row.
    var snippet: ActivitySnippet? = nil

    var body: some View {
        let field = EntryLook.field(entry)
        let line = EntryLook.line(entry)
        HStack(alignment: .center, spacing: 14) {
            RequestIcon(spec: line.icon, size: 36, ring: selected ? Palette.violetWash : Palette.paper)
                .foldTile(entry.id)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                AskRunsText(runs: line.runs, size: 18, terms: terms)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(2)
                    .accessibilityLabel(line.text)
                    .accessibilityIdentifier("entryTitle:\(entry.id)")
                if let mono = line.mono {
                    Text(verbatim: mono)
                        .font(RFont.mono(12))
                        .foregroundStyle(Palette.inkMeta)
                        .lineLimit(1)
                        .environment(\.layoutDirection, .leftToRight)
                        .accessibilityIdentifier("entryDetails:\(entry.id)")
                }
                Text(EntryText.byline(entry, now: now))
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                if let risk = entry.autopilot?.risk, RiskLook.listed(risk) {
                    RiskLine(risk: risk)
                }
                if let snippet {
                    Text(ActivitySearchText.snippet(snippet))
                        .lineLimit(2)
                        .padding(.top, 2)
                        .accessibilityIdentifier("snippet:\(entry.id)")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            EntryMark(mark: EntryText.mark(entry))
        }
        .padding(.vertical, 10)
        .frame(minHeight: 60)
        .padding(.horizontal, 24)
        .background {
            if selected {
                Palette.violetWash
            } else if glowing, let field {
                field.tile.transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 1.2), value: glowing)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    /// What happened, in a line (ASK-TEXT): in the past when it was carried out, else what was asked.
    static func title(_ entry: ActivityEntry) -> String { EntryLook.line(entry).text }
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
                        .foregroundStyle(Palette.ink2)
                    Text(name)
                        .font(RFont.sans(16.5, .semibold))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                Text(FileText.size(blob.size) + " · " + untrusted(blob.contentType))
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
                Text("SHA-256 " + FileText.shortSha(blob.sha256))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.inkMeta)
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
                .background(Palette.paperSunken)
                .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                .padding(.top, 6)
                .accessibilityLabel("Preview of \(name)")
        } else if let text, FileText.isText(blob.contentType) {
            Text(text)
                .font(RFont.mono(12.5))
                .foregroundStyle(Palette.ink)
                .lineLimit(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .environment(\.layoutDirection, .leftToRight)
                .padding(.top, 6)
        } else if let text {
            Text(text)
                .font(RFont.sans(14, .medium))
                .foregroundStyle(Palette.ink2)
                .lineLimit(2)
                .padding(.top, 4)
        }
    }
}
