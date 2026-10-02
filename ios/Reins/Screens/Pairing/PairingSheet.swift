import SwiftUI

/// Placeholder: replaced by the real screen.
struct PairingSheet: View {
    var pairingId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "Connect an AI", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("Connect an AI")
    }
}
