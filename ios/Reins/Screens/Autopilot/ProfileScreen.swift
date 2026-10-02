import SwiftUI

/// Placeholder: replaced by the real screen.
struct ProfileScreen: View {
    var profileId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Profile", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Profile")
    }
}
