import SwiftUI

/// Placeholder: replaced by the real screen.
struct GrantDetailScreen: View {
    var grantId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Grant", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Grant")
    }
}
