import SwiftUI

/// One server: whether it works, its tools and what each may do, and the ways to sign in again or remove it.
struct McpServerScreen: View {
    var serverId: String

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var mcp: McpModel?
    @State private var removing = false

    var body: some View {
        Group {
            if let server = model.mcpServers.first(where: { $0.id == serverId }) {
                content(server)
            } else {
                EmptyState(symbol: "server.rack", title: "MCP server", message: "This server is no longer added.")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
                    .navigationTitle("MCP server")
                    .accessibilityIdentifier("mcpGone")
            }
        }
        .task(id: serverId) {
            if mcp == nil { mcp = McpModel(model: model) }
            mcp?.error = nil
        }
        // How a sign-in ended is shown while this page is open, and only once.
        .onDisappear {
            if model.mcpNotice?.serverId == serverId { model.mcpNotice = nil }
        }
    }

    private var busy: Bool { mcp?.busy ?? false }

    @ViewBuilder
    private func content(_ server: McpServerView) -> some View {
        List {
            if let notice = model.mcpNotice, notice.serverId == serverId {
                Section {
                    IntegrationBanner(text: untrusted(notice.text), kind: notice.failed ? .error : .success)
                        .accessibilityIdentifier("mcpNotice")
                        .plainListRow(top: 8, bottom: 0)
                }
            }

            Section {
                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 12) {
                        ServiceAvatar(service: "mcp", size: 40)
                        Text(untrusted(server.url))
                            .font(RFont.mono(13.5))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(2)
                            .truncationMode(.middle)
                            .environment(\.layoutDirection, .leftToRight)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                        McpStatusPill(status: server.status)
                    }
                    if let error = server.error.map(untrusted), !error.isEmpty {
                        Text(error)
                            .textRole(.body)
                            .foregroundStyle(Palette.danger)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("mcpServerError")
                    }
                    if server.status == "needs_sign_in" {
                        Text("Sign in to use its tools.")
                            .textRole(.body)
                            .foregroundStyle(Palette.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                        ActionButton(title: "Sign in", symbol: "person.badge.key", busy: busy) {
                            Task { await mcp?.refresh(serverId) }
                        }
                        .padding(.top, 4)
                        .accessibilityIdentifier("mcpSignIn")
                    }
                }
                .padding(.vertical, 6)
            } header: {
                GroupHeading("Server")
            }
            .listRowBackground(Color.clear)

            if mcp?.signingIn == serverId || mcp?.error != nil {
                Section {
                    VStack(spacing: 10) {
                        if mcp?.signingIn == serverId {
                            IntegrationBanner(text: "Finish signing in on the page that opened.")
                                .accessibilityIdentifier("mcpSigningIn")
                        }
                        if let error = mcp?.error {
                            IntegrationBanner(text: untrusted(error), kind: .error)
                                .accessibilityIdentifier("mcpError")
                        }
                    }
                    .plainListRow(top: 0, bottom: 0)
                }
            }

            Section {
                if server.tools.isEmpty {
                    Text(server.status == "ok" ? "This server has no tools." : "No tools yet.")
                        .textRole(.body)
                        .foregroundStyle(Palette.ink2)
                        .padding(.vertical, 6)
                        .accessibilityIdentifier("noTools")
                }
                ForEach(server.tools, id: \.name) { tool in
                    ToolRow(tool: tool, enabled: !busy) { heavy in
                        feedback.play(.toggle(heavy))
                        Task { await mcp?.setHeavy(serverId, tool: tool.name, heavy: heavy) }
                    }
                }
            } header: {
                HStack {
                    GroupHeading("Tools · \(server.tools.count)")
                    Spacer()
                    Text("You approve each call").textRole(.meta).foregroundStyle(Palette.inkMeta).textCase(nil)
                }
            } footer: {
                GroupFootnote("Your AIs can ask to use these tools. Tools that only read need a read permission; anything else is a change you approve. Large results go through your Reins server instead of this phone.")
            }
            .listRowBackground(Color.clear)

            Section {
                VStack(spacing: 10) {
                    ActionButton(title: "Refresh", kind: .secondary, enabled: !busy) {
                        Task { await mcp?.refresh(serverId) }
                    }
                    .accessibilityIdentifier("mcpRefresh")
                    // Asks once: its tools, sign-in and permissions go.
                    ActionButton(title: "Remove server", kind: .destructive, enabled: !busy) {
                        removing = true
                    }
                    .accessibilityIdentifier("mcpRemove")
                }
                .plainListRow(top: 0, bottom: 28)
            }
        }
        .integrationList()
        .navigationTitle(untrusted(server.name))
        .navigationSubtitle(McpLogic.host(server.url))
        .navigationBarTitleDisplayMode(.large)
        .accessibilityIdentifier("mcpDetail")
        .animation(.smooth, value: model.mcpNotice)
        .animation(.smooth, value: mcp?.error)
        .confirmationDialog("Remove \(untrusted(server.name))?", isPresented: $removing, titleVisibility: .visible) {
            Button("Remove", role: .destructive) {
                feedback.quietClose()
                Task {
                    guard await mcp?.remove(serverId) == true else { return }
                    if model.path(model.section).last == .mcpServer(serverId) { model.back() }
                }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Its tools, sign-in and permissions go.")
        }
        .presentationFeedback(removing)
    }
}

/// One tool: its name, what it does, what it may do, and whether its large results go through the server.
private struct ToolRow: View {
    var tool: McpToolView
    var enabled: Bool
    var onHeavy: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            let title = untrusted(tool.title)
            Text(title.isEmpty ? tool.name : title)
                .textRole(.row)
                .foregroundStyle(Palette.ink)
                .lineLimit(2)
            if tool.title != tool.name {
                Text(untrusted(tool.name))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
            }
            if !tool.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(untrusted(tool.description))
                    .textRole(.meta)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(4)
            }
            // Badges are words: a glyph and what it means, never a coloured dot.
            BadgeFlow(spacing: 14) {
                ForEach(ToolBadge.of(tool), id: \.self) { badge in
                    HStack(spacing: 5) {
                        GlyphIcon(Self.glyph(badge), size: 14)
                        Text(badge.label)
                    }
                    .font(TextRole.tag.font())
                    .foregroundStyle(Self.tint(badge))
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("badge:\(tool.name):\(badge.rawValue)")
                }
            }
            .padding(.top, 6)
            Toggle(isOn: Binding(get: { tool.heavy }, set: onHeavy)) {
                Text("Send large results through the server")
                    .textRole(.meta)
                    .foregroundStyle(Palette.ink2)
            }
            .disabled(!enabled)
            .padding(.top, 4)
            .accessibilityIdentifier("heavy:\(tool.name)")
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("tool:\(tool.name)")
    }

    /// Only "Asks every time" is coloured: it is the one that changes what happens.
    static func tint(_ badge: ToolBadge) -> Color {
        switch badge {
        case .readOnly, .changes, .heavy: Palette.ink2
        case .asksEveryTime: Palette.danger
        }
    }

    static func glyph(_ badge: ToolBadge) -> Glyph {
        switch badge {
        case .readOnly: .eye
        case .changes: .pencil
        case .asksEveryTime: .lock
        case .heavy: .cloudUpload
        }
    }
}

/// Lays its children out in rows, wrapping to the next line when one is full.
private struct BadgeFlow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(rows.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in arrange(width: bounds.width, subviews: subviews) {
            var x = bounds.minX
            for i in row.items {
                let size = subviews[i].sizeThatFits(.unspecified)
                subviews[i].place(at: CGPoint(x: x, y: y + (row.height - size.height) / 2), proposal: .unspecified)
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var items: [Int] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for (i, subview) in subviews.enumerated() {
            let size = subview.sizeThatFits(.unspecified)
            let needed = row.items.isEmpty ? size.width : row.width + spacing + size.width
            if needed > width, !row.items.isEmpty {
                rows.append(row)
                row = Row()
            }
            row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.items.append(i)
        }
        if !row.items.isEmpty { rows.append(row) }
        return rows
    }
}
