import SwiftUI

/// Placeholder: replaced by the real screen.
struct ConnectionDetailScreen: View {
    var connectionId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Connection", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Connection")
    }
}
