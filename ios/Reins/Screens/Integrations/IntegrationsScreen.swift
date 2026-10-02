import SwiftUI

/// Every service Reins can connect, with how many accounts each one has, and the MCP servers the user added.
struct IntegrationsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    /// The services as listed, text messages last (iOS has no way to them, so they are only shown as unavailable).
    private var services: [ServiceView] { model.services + [ServiceCopy.sms] }

    var body: some View {
        List {
            Section {
                ForEach(services, id: \.service) { service in
                    LinkRow(identifier: "service:\(service.service)", action: { open(service.service) }) {
                        ServiceAvatar(service: service.service, size: 44)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(service.name).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
                            Text(ServiceCopy.summary(service))
                                .font(RFont.sans(13))
                                .foregroundStyle(service.accounts.isEmpty ? Palette.tertiary : Palette.secondary)
                        }
                    }
                }
            } header: {
                GroupHeading("Services")
            } footer: {
                GroupFootnote("Each service can have as many accounts as you like. Your AIs can see which services and accounts exist and choose one; you approve what they do with it.")
            }
            .listRowBackground(Palette.elevated)

            McpServersSection(servers: model.mcpServers)
        }
        .integrationList()
        .navigationTitle("Integrations")
        .navigationBarTitleDisplayMode(.large)
        .refreshable {
            feedback.play(.refresh)
            await model.refreshPending()
        }
        .task {
            await model.refreshPending()
            DemoIntegrationsLaunch.openOnce(model)
        }
    }

    private func open(_ service: String) {
        model.push(service == "gmail" ? .gmail : .service(service))
    }
}

/// A list row that opens another screen: the content, then a chevron.
struct LinkRow<Content: View>: View {
    var identifier: String
    var action: () -> Void
    @ViewBuilder var content: Content
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 14) {
                content
                Spacer(minLength: 6)
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Palette.tertiary)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, 4)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier(identifier)
    }
}

// MARK: MCP servers

/// The MCP servers the user added, under the services, with the way to add another.
struct McpServersSection: View {
    var servers: [McpServerView]
    @Environment(AppModel.self) private var model

    var body: some View {
        Section {
            if servers.isEmpty {
                NoAccountsRow(title: "No MCP servers yet", message: "Add one by its address, like https://mcp.linear.app/mcp.")
                    .accessibilityIdentifier("noMcp")
            }
            ForEach(servers, id: \.id) { server in
                LinkRow(identifier: "mcp:\(server.id)", action: { model.push(.mcpServer(server.id)) }) {
                    ServiceAvatar(service: "mcp", size: 44)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(untrusted(server.name))
                            .font(RFont.sans(16, .medium))
                            .foregroundStyle(Palette.text)
                            .lineLimit(1)
                        Text(McpLogic.host(server.url) + " · " + McpLogic.toolCount(server.tools.count))
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .environment(\.layoutDirection, .leftToRight)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    McpStatusPill(status: server.status)
                }
            }
        } header: {
            GroupHeading("MCP servers")
        } footer: {
            GroupFootnote("Tools of the MCP servers you add here are offered to your AIs through Reins. You approve each call, the way you approve everything else.")
        }
        .listRowBackground(Palette.elevated)

        Section {
            ActionButton(title: "Add MCP server", symbol: "plus", kind: .secondary) { model.push(.mcpAdd) }
                .accessibilityIdentifier("addMcp")
                .plainListRow(top: 0, bottom: 24)
        }
    }
}

/// An MCP server's status on its colour.
struct McpStatusPill: View {
    var status: String

    var body: some View {
        StatusPill(text: McpLogic.statusLabel(status), tint: Self.tint(status))
            .accessibilityIdentifier("mcpStatus")
    }

    static func tint(_ status: String) -> Color {
        switch status {
        case "ok": Palette.success
        case "needs_sign_in": Palette.warning
        default: Palette.danger
        }
    }
}

/// `-demo -integration <gmail | telegram | mcp:linear | mcpAdd | ...>`: opens one integration screen at launch, for
/// screenshots and a look at a screen without tapping there.
@MainActor
enum DemoIntegrationsLaunch {
    private static var done = false

    static func openOnce(_ model: AppModel) {
        guard model.demo, !done else { return }
        done = true
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-integration"), i + 1 < args.count else { return }
        let target = args[i + 1]
        if target == "gmail" {
            model.push(.gmail)
        } else if target == "mcpAdd" {
            model.push(.mcpAdd)
        } else if target.hasPrefix("mcp:") {
            model.push(.mcpServer(String(target.dropFirst(4))))
        } else {
            model.push(.service(target))
        }
    }
}
