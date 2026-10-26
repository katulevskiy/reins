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
        NextPage {
            if let receipt = model.connectedServers[serverId], server.status == "ok" {
                IntegrationCompletion(title: "Server connected", identity: "\(receipt.name) · \(McpLogic.host(receipt.url))", onDone: {
                    model.connectedServers[serverId] = nil
                })
                .padding(.horizontal, 20)
                .padding(.vertical, 12)
            }
            QuietTitle(title: untrustedName(server.name, limit: 40)) {
                MarkPlate(size: 36) { ServiceAvatar(service: "mcp", size: 36) }
            }
            if let notice = model.mcpNotice, notice.serverId == serverId, notice.failed || model.connectedServers[serverId] == nil {
                IntegrationBanner(text: untrusted(notice.text), kind: notice.failed ? .error : .success)
                    .padding(.horizontal, 20)
                    .padding(.top, 12)
                    .accessibilityIdentifier("mcpNotice")
            }
            SectionHeader("Server", quiet: true)
            FlatGroup(inset: 44) {
                HStack(spacing: 12) {
                    GlyphTile(.globe)
                    Text(verbatim: untrusted(server.url))
                        .font(RFont.mono(14))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(2)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .textSelection(.enabled)
                    if server.status == "ok" { OkDot(label: McpLogic.statusLabel(server.status)).accessibilityIdentifier("mcpStatus") } else { McpStatusPill(status: server.status) }
                }
                .frame(minHeight: 56)
            }
            VStack(alignment: .leading, spacing: 10) {
                if let error = server.error.map(untrusted), !error.isEmpty {
                    Text(error)
                        .textRole(.sub)
                        .foregroundStyle(Palette.danger)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("mcpServerError")
                }
                if server.status == "needs_sign_in" {
                    Button {
                        Task { await mcp?.refresh(serverId) }
                    } label: {
                        BusyLabel(title: "Sign in", glyph: .person, busy: busy)
                    }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(busy)
                    .accessibilityIdentifier("mcpSignIn")
                }
                if mcp?.signingIn == serverId {
                    IntegrationBanner(text: "Finish signing in on the page that opened.").accessibilityIdentifier("mcpSigningIn")
                }
                if let error = mcp?.error {
                    IntegrationBanner(text: untrusted(error), kind: .error).accessibilityIdentifier("mcpError")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 10)
            SectionHeader("Tools · \(server.tools.count)", quiet: true) {
                Label { Text("Via server") } icon: { GlyphIcon(.server, size: 13) }
                    .labelStyle(SignalLabelStyle())
                    .font(RFont.sans(12.5, .semibold))
                    .foregroundStyle(Palette.inkMeta)
                    .accessibilityLabel("The switches send large results via the server")
            }
            FlatGroup(inset: 44) {
                if server.tools.isEmpty {
                    Text(server.status == "ok" ? "This server has no tools." : "No tools yet.")
                        .font(RFont.sans(15))
                        .foregroundStyle(Palette.ink2)
                        .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
                        .accessibilityIdentifier("noTools")
                }
                ForEach(server.tools, id: \.name) { tool in
                    ToolRow(tool: tool, enabled: !busy) { heavy in
                        feedback.play(.toggle(heavy))
                        Task { await mcp?.setHeavy(serverId, tool: tool.name, heavy: heavy) }
                    }
                }
            }
            GroupFooter(
                "Your AIs can ask to use these tools, and you approve each call. Tools that only read need a read permission; anything else is a change you approve; destructive ones ask every time.\n\n" +
                    "Via server: large results go through your Reins server instead of this phone.\n\n" +
                    "Scoped account permissions are not available for MCP servers yet. Its approval and tool settings still apply."
            )
            .padding(.horizontal, 20)
        }
        .quietPageBar(untrustedName(server.name, limit: 40))
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button { Task { await mcp?.refresh(serverId) } } label: { GlyphIcon(.refresh, size: 20).foregroundStyle(Palette.ink) }
                    .disabled(busy)
                    .accessibilityLabel("Refresh")
                    .accessibilityIdentifier("mcpRefresh")
            }
            ToolbarSpacer(.fixed, placement: .primaryAction)
            ToolbarItem(placement: .primaryAction) {
                // Asks once: its tools, sign-in and permissions go.
                Button { removing = true } label: { GlyphIcon(.trash, size: 20).foregroundStyle(Palette.ink) }
                    .disabled(busy)
                    .accessibilityLabel("Remove server")
                    .accessibilityIdentifier("mcpRemove")
            }
        }
        .accessibilityIdentifier("mcpDetail")
        .animation(.smooth, value: model.mcpNotice)
        .animation(.smooth, value: mcp?.error)
        .confirmationDialog("Remove \(untrustedName(server.name, limit: 40))?", isPresented: $removing, titleVisibility: .visible) {
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

/// One tool (the Connections board, 05): its kind on a tile (red for one that deletes), its title and its name in
/// mono, "Asks" when it asks every time, and the switch that sends its large results via the server.
private struct ToolRow: View {
    var tool: McpToolView
    var enabled: Bool
    var onHeavy: (Bool) -> Void

    var body: some View {
        let title = untrustedName(tool.title, limit: 48)
        HStack(spacing: 12) {
            GlyphTile(tool.destructive ? .trash : tool.readOnly ? .eye : .plus, danger: tool.destructive)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: title.isEmpty ? untrustedName(tool.name, limit: 48) : title)
                    .textRole(.row)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(2)
                Text(verbatim: untrustedName(tool.name, limit: 48))
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if tool.destructive {
                Pill("Asks", tone: .danger, glyph: .lock, size: .pill)
                    .accessibilityLabel(ToolBadge.asksEveryTime.label)
                    .accessibilityIdentifier("badge:\(tool.name):\(ToolBadge.asksEveryTime.rawValue)")
            }
            Toggle(isOn: Binding(get: { tool.heavy }, set: onHeavy)) {
                Text("Send large results via the server")
            }
            .labelsHidden()
            .disabled(!enabled)
            .accessibilityIdentifier("heavy:\(tool.name)")
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("tool:\(tool.name)")
    }
}
