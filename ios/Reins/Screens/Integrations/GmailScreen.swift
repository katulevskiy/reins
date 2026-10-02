import SwiftUI

/// The Gmail accounts: add as many as you like, or remove any of them. The same page as every other integration;
/// Gmail only has its own calls in the core.
struct GmailScreen: View {
    var body: some View {
        AccountsScreen(serviceId: "gmail")
    }
}
