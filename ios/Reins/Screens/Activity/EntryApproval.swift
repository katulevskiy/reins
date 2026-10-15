import SwiftUI

// An Activity entry's request in full (the Android app's ActivityDetailScreen parts): the approval as it was shown,
// read-only, and what was asked ("What it asked", "Full request").

/// "What it asked" in words (pure, unit-tested).
enum AskedText {
    /// "Claude Code · M2A": the agent as it named itself and the connection it asked through (the computer's name,
    /// which says more than its 127.0.0.1 host); without an agent, the connection's name and where it connects from.
    static func who(agent: String, connection: String, host: String?) -> String {
        let agent = untrusted(agent).trimmingCharacters(in: .whitespaces)
        let connection = untrusted(connection)
        let host = host.map(untrusted) ?? ""
        let first = agent.isEmpty ? connection : agent
        let second = agent.isEmpty ? host : connection
        var parts: [String] = []
        if !first.isEmpty { parts.append(first) }
        if !second.isEmpty && !parts.contains(second) { parts.append(second) }
        return parts.joined(separator: " · ")
    }

    /// " · answered in 9 s", from when it was asked to when it was decided; empty when that is not known.
    static func answeredIn(askedAt: Int64, decidedAt: Int64) -> String {
        guard askedAt > 0, decidedAt >= askedAt else { return "" }
        let d = decidedAt - askedAt
        let span = d < 60 ? "\(d) s" : d < 3_600 ? "\(d / 60) min" : "\(d / 3_600) h"
        return " · answered in " + span
    }

    /// "Today, 14:32", "Yesterday, 09:10", else the date and the time.
    static func dayTime(_ at: Int64, now: Int64 = Timers.nowSeconds()) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(at))
        let today = Date(timeIntervalSince1970: TimeInterval(now))
        let time = date.formatted(date: .omitted, time: .shortened)
        let calendar = Calendar.current
        if calendar.isDate(date, inSameDayAs: today) { return "Today, \(time)" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: today), calendar.isDate(date, inSameDayAs: yesterday) {
            return "Yesterday, \(time)"
        }
        return TimeText.dateTime(at)
    }

    /// The command or tool call as kept, without the blank lines and spaces after it.
    static func fullText(_ full: String) -> String {
        var text = untrustedLine(full)
        while let last = text.last, last.isWhitespace { text.removeLast() }
        return text
    }

    /// What the copy button puts on the clipboard: the request, a blank line, then what the agent said it was for.
    static func copyText(full: String, purpose: String) -> String {
        [full, purpose].filter { !$0.isEmpty }.joined(separator: "\n\n")
    }

    /// `# Claude Code: “Force-push the rebased branch”`: what the agent said, as a comment under the request.
    static func purposeLine(who: String, purpose: String) -> String {
        "# \(who): “\(purpose)”"
    }
}

/// What the entry asked: who (the agent and its computer), where it ran, why it was asked and when (with how long the
/// answer took); then the full request, the whole command or tool call wrapped in a mono well with what the agent
/// said, and a copy button. Nothing for entries logged before the request was kept.
struct AskedSections: View {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        if let r = entry.request {
            let host = findConnection(model.connections, id: entry.connectionId, label: entry.connectionLabel)?.clientHost
            let cwd = untrusted(r.cwd)
            let why = untrusted(r.why)
            GroupCard(header: "What it asked") {
                AskedRow(label: "Who", value: AskedText.who(agent: r.agent, connection: entry.connectionLabel, host: host), id: "askedWho") {
                    ConnectionIcon(connectionId: entry.connectionId, label: entry.connectionLabel, size: 18)
                }
                if !cwd.isEmpty {
                    Hairline()
                    AskedRow(label: "In", value: cwd, mono: true, id: "askedIn") { EmptyView() }
                }
                if !why.isEmpty {
                    Hairline()
                    AskedRow(label: "Why asked", value: why, id: "askedWhy") { EmptyView() }
                }
                Hairline()
                AskedRow(label: "When", value: AskedText.dayTime(entry.at) + AskedText.answeredIn(askedAt: r.askedAt, decidedAt: entry.at), id: "askedWhen") {
                    EmptyView()
                }
            }
            fullRequest(r)
        }
    }

    @ViewBuilder private func fullRequest(_ r: ActivityRequest) -> some View {
        let full = AskedText.fullText(r.full)
        let purpose = untrusted(r.purpose)
        if !full.isEmpty || !purpose.isEmpty {
            let agent = untrusted(r.agent)
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 4) {
                    SectionHeader("Full request")
                    Button {
                        UIPasteboard.general.string = AskedText.copyText(full: full, purpose: purpose)
                        feedback.play(.copied)
                    } label: {
                        Image(systemName: "doc.on.doc")
                            .font(.system(size: 15, weight: .medium))
                            .foregroundStyle(Palette.violet)
                            .frame(width: 36, height: 32)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Copy the full request")
                    .accessibilityIdentifier("copyFullRequest")
                }
                .padding(.leading, 12)
                VStack(alignment: .leading, spacing: 12) {
                    if !full.isEmpty {
                        // Wrapped, never cut: the whole request is what this is for.
                        Text(full)
                            .font(RFont.mono(12.5))
                            .foregroundStyle(Palette.ink)
                            .lineSpacing(5)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("fullRequestText")
                    }
                    if !purpose.isEmpty {
                        Text(AskedText.purposeLine(who: agent.isEmpty ? untrusted(entry.connectionLabel) : agent, purpose: purpose))
                            .font(RFont.mono(12.5))
                            .foregroundStyle(Palette.inkMeta)
                            .lineSpacing(5)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("fullRequestPurpose")
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .background(Self.well, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Self.wellEdge, lineWidth: 1))
                .environment(\.layoutDirection, .leftToRight)
                .textSelection(.enabled)
                .accessibilityContainer("fullRequest")
            }
            .padding(.horizontal, 16)
            .padding(.top, 22)
        }
    }

    /// The mono well: near black with a faint edge on dark, the elevated surface on light.
    static let well = Palette.dynamic(0xFFFFFF, 0x0A0A0C)
    static let wellEdge = Palette.dynamic(0xE2E2E6, 0x1C1C20)
}

/// A row of "What it asked": the label in a fixed column, the value beside it, wrapping.
private struct AskedRow<Leading: View>: View {
    var label: String
    var value: String
    var mono = false
    var id: String
    @ViewBuilder var leading: Leading
    @Environment(\.layoutDirection) private var direction

    var body: some View {
        HStack(spacing: 8) {
            Text(label)
                .font(RFont.sans(16))
                .foregroundStyle(Palette.inkMeta)
                .lineLimit(1)
                .frame(width: 96, alignment: .leading)
            leading
            Text(value)
                .font(mono ? RFont.mono(14) : RFont.sans(15))
                .foregroundStyle(Palette.ink)
                .lineLimit(4)
                .frame(maxWidth: .infinity, alignment: .leading)
                // A path reads left to right in every language.
                .environment(\.layoutDirection, mono ? .leftToRight : direction)
                .textSelection(.enabled)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .frame(minHeight: 52)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier(id)
    }
}

/// An entry that kept its approval: its decided header, then the approval screen's own facts and pages (commits, a
/// commit's lines, the emails), read-only, and `below` under the facts. Its pages push onto the Activity
/// stack like any page.
struct EntryApprovalPage<Below: View>: View {
    var entry: ActivityEntry
    var vm: ApprovalModel
    var view: ApprovalView
    @ViewBuilder var below: Below
    @State private var nav = ApprovalNav()

    var body: some View {
        let record = ApprovalRecord.of(entry)
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                DecidedHeader(entry: entry)
                Facts(vm: vm, view: view).padding(.top, 4)
                below
            }
            .padding(.bottom, 24)
        }
        .environment(nav)
        .environment(\.approvalRecord, record)
        .modifier(EntryPagesLevel(nav: nav, view: view, record: record, level: 0))
        .accessibilityContainer("entryApproval")
    }
}

/// The approval's pages as pushed pages: the page at `level` of the approval's own path, pushed from the one before it,
/// and popping it (back, or a swipe) drops it and the ones after it.
private struct EntryPagesLevel: ViewModifier {
    var nav: ApprovalNav
    var view: ApprovalView
    var record: ApprovalRecord
    var level: Int

    func body(content: Content) -> some View {
        // Read here, so a page pushed or popped draws this level again.
        let path = nav.path
        return content.navigationDestination(item: Binding(
            get: { path.count > level ? path[level] : nil },
            set: { page in
                if page == nil, nav.path.count > level { nav.path.removeSubrange(level...) }
            }
        )) { page in
            ScrollView {
                page.content(view).padding(.bottom, 20)
            }
            .background(Palette.paper.ignoresSafeArea())
            .navigationTitle(page.title(in: view))
            .navigationBarTitleDisplayMode(.inline)
            .environment(nav)
            .environment(\.approvalRecord, record)
            .modifier(EntryPagesLevel(nav: nav, view: view, record: record, level: level + 1))
        }
    }
}
