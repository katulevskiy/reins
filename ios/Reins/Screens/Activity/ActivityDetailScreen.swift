import SwiftUI

/// Placeholder: replaced by the real screen.
struct ActivityDetailScreen: View {
    var entryId: Int64
    var body: some View {
        EmptyState(symbol: "hammer", title: "Activity", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Activity")
    }
}
