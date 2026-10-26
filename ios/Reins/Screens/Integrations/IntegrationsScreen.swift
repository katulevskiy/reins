import SwiftUI

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

/// `-demo -integration <gmail | telegram | mcp:linear | mcpAdd | ...>`: opens one integration screen on Connections at
/// launch, for screenshots and a look at a screen without tapping there.
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
