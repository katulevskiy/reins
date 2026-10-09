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
                            .foregroundStyle(Palette.secondary)
                            .lineLimit(2)
                            .truncationMode(.middle)
                            .environment(\.layoutDirection, .leftToRight)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)
                        McpStatusPill(status: server.status)
                    }
                    if let error = server.error.map(untrusted), !error.isEmpty {
                        Text(error)
                            .font(RFont.sans(14))
                            .foregroundStyle(Palette.danger)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("mcpServerError")
                    }
                    if server.status == "needs_sign_in" {
                        Text("This server wants you to sign in before its tools can be used.")
                            .font(RFont.sans(14))
                            .foregroundStyle(Palette.secondary)
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
            .listRowBackground(Palette.elevated)

            if mcp?.signingIn == serverId || mcp?.error != nil {
                Section {
                    VStack(spacing: 10) {
                        if mcp?.signingIn == serverId {
                            IntegrationBanner(text: "Sign in on the page that opened. When you are done there, Reins comes back by itself.")
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
                    Text(server.status == "ok" ? "This server has no tools." : "The tools show up once the server can be reached.")
                        .font(RFont.sans(14.5))
                        .foregroundStyle(Palette.secondary)
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
                GroupHeading("Tools (\(server.tools.count))")
            } footer: {
                GroupFootnote("Your AIs can ask to use these tools. Tools that only read need a read permission; anything else is a change you approve. Large results go through your Reins server instead of this phone.")
            }
            .listRowBackground(Palette.elevated)

            Section {
                HStack(spacing: 10) {
                    ActionButton(title: "Refresh", symbol: "arrow.clockwise", kind: .secondary, enabled: !busy) {
                        Task { await mcp?.refresh(serverId) }
                    }
                    .accessibilityIdentifier("mcpRefresh")
                    ActionButton(title: "Remove server", symbol: "trash", kind: .destructive, enabled: !busy) {
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
            Text("Your AIs lose its tools, its sign-in is forgotten on this phone, and the permissions given for it are deleted.")
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
                .font(RFont.sans(16, .semibold))
                .foregroundStyle(Palette.text)
                .lineLimit(2)
            if tool.title != tool.name {
                Text(untrusted(tool.name))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
            }
            if !tool.description.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(untrusted(tool.description))
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .lineLimit(4)
            }
            BadgeFlow(spacing: 6) {
                ForEach(ToolBadge.of(tool), id: \.self) { badge in
                    StatusPill(text: badge.label, tint: Self.tint(badge))
                        .accessibilityIdentifier("badge:\(tool.name):\(badge.rawValue)")
                }
            }
            .padding(.top, 6)
            Toggle(isOn: Binding(get: { tool.heavy }, set: onHeavy)) {
                Text("Send large results through the server")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
            }
            .tint(Palette.accent)
            .disabled(!enabled)
            .padding(.top, 4)
            .accessibilityIdentifier("heavy:\(tool.name)")
        }
        .padding(.vertical, 8)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("tool:\(tool.name)")
    }

    static func tint(_ badge: ToolBadge) -> Color {
        switch badge {
        case .readOnly: Palette.success
        case .changes: Palette.send
        case .asksEveryTime: Palette.danger
        case .heavy: Palette.accent
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
