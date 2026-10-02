import SwiftUI

/// Placeholder: replaced by the real screen.
struct McpServerScreen: View {
    var serverId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "MCP server", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("MCP server")
    }
}
