import SwiftUI

/// Placeholder: replaced by the real screen.
struct TryItScreen: View {
    var profileId: String?
    var body: some View {
        EmptyState(symbol: "hammer", title: "Try it", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Try it")
    }
}
