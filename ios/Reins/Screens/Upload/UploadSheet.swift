import SwiftUI

/// Placeholder: replaced by the real screen.
struct UploadSheet: View {
    var blobId: String
    var body: some View {
        EmptyState(symbol: "hammer", title: "File to check", message: "Coming soon.")
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
            .navigationTitle("File to check")
    }
}
