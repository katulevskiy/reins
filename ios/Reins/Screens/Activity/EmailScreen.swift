import SwiftUI

/// Placeholder: replaced by the real screen.
struct EmailScreen: View {
    var entryId: Int64
    var index: Int
    var body: some View {
        EmptyState(symbol: "hammer", title: "Email", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Email")
    }
}
