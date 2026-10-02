import SwiftUI

/// Placeholder: replaced by the real screen.
struct ApprovalSheet: View {
    var requestId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Approval", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Approval")
    }
}
