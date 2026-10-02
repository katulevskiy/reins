import Foundation

/// Google access tokens for Gmail, Calendar and Contacts (the core's `GoogleTokenProvider`). Placeholder until the
/// keychain-backed OAuth implementation lands: every call asks for user interaction.
final class GoogleTokens: GoogleTokenProvider, @unchecked Sendable {
    static let shared = GoogleTokens()

    func accessToken(account: String, service: String) async throws -> String {
        throw ForeignError.NeedsUserInteraction
    }
}
