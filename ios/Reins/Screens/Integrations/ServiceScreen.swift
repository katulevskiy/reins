import SwiftUI

/// Placeholder: replaced by the real screen.
struct ServiceScreen: View {
    var serviceId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Integration", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Integration")
    }
}
