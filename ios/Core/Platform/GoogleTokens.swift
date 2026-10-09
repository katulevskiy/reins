import Foundation
import Security

/// The Google OAuth client this build uses (`ReinsGoogleClientId` / `ReinsGoogleRedirectScheme` in Info.plist, from
/// `REINS_GOOGLE_CLIENT_ID` / `REINS_GOOGLE_REDIRECT_SCHEME`). An "iOS" OAuth client: no secret, PKCE, and a redirect to
/// its reversed client id.
struct GoogleConfig: Equatable {
    var clientId: String
    var redirectScheme: String

    static var current: GoogleConfig {
        let info = Bundle.main.infoDictionary ?? [:]
        return GoogleConfig(
            clientId: (info["ReinsGoogleClientId"] as? String ?? "").trimmingCharacters(in: .whitespaces),
            redirectScheme: (info["ReinsGoogleRedirectScheme"] as? String ?? "").trimmingCharacters(in: .whitespaces)
        )
    }

    /// Without a client id every Google integration "needs setup", as on Android without an OAuth client.
    var isConfigured: Bool {
        !clientId.isEmpty && !clientId.hasPrefix("$") && !redirectScheme.isEmpty && !redirectScheme.hasPrefix("$")
    }

    var redirectURI: String { "\(redirectScheme):/oauth2redirect" }

    static let setupMessage =
        "Google access needs setup: create an iOS OAuth client in Google Cloud, put its client id and reversed client id in REINS_GOOGLE_CLIENT_ID and REINS_GOOGLE_REDIRECT_SCHEME, and enable the Gmail, Calendar and People APIs."

    static let authorizeURL = URL(string: "https://accounts.google.com/o/oauth2/v2/auth")!
    static let tokenURL = URL(string: "https://oauth2.googleapis.com/token")!
    static let revokeURL = URL(string: "https://oauth2.googleapis.com/revoke")!
}

/// The OAuth scopes each Google service needs (the core's `GoogleTokenProvider` contract; the same lists as the
/// Android app's `GoogleAuthorizer.scopesOf`).
enum GoogleScopes {
    /// Read, search, send, drafts, labels, archive, spam and Trash (never deleting for good).
    static let gmailModify = "https://www.googleapis.com/auth/gmail.modify"
    /// Filters (never forwarding) and the vacation reply.
    static let gmailSettings = "https://www.googleapis.com/auth/gmail.settings.basic"
    static let calendarEvents = "https://www.googleapis.com/auth/calendar.events"
    static let calendarReadonly = "https://www.googleapis.com/auth/calendar.readonly"
    static let contactsReadonly = "https://www.googleapis.com/auth/contacts.readonly"
    /// Asked for along with a service's scopes so the sign-in names the account (the ID token's `email`).
    static let identity = ["openid", "email"]

    static func of(_ service: String) -> [String] {
        switch service {
        case "gcalendar": [calendarEvents, calendarReadonly]
        case "gcontacts": [contactsReadonly]
        default: [gmailModify, gmailSettings]
        }
    }

    static func name(_ service: String) -> String {
        switch service {
        case "gcalendar": "Google Calendar"
        case "gcontacts": "Google Contacts"
        default: "Gmail"
        }
    }
}

/// Why getting at Google did not work, in words the integration screens can show (`error.userMessage`).
enum GoogleAuthError: LocalizedError, Equatable {
    case notConfigured
    /// The user closed the sign-in page.
    case cancelled
    /// The user said no on Google's page.
    case denied
    /// Google's page let the user leave out some of the access asked for.
    case missingScopes(String)
    /// Google no longer accepts the stored refresh token (revoked, password changed, unused for months).
    case revoked
    case network(String)
    case failed(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured: GoogleConfig.setupMessage
        case .cancelled: "Signing in to Google was cancelled."
        case .denied: "Google access was not allowed."
        case let .missingScopes(service): "\(service) needs every box on Google's page ticked. Try again and allow all of them."
        case .revoked: "Google access was taken back. Sign in again."
        case let .network(reason): "No connection to Google: \(reason)"
        case let .failed(reason): reason
        }
    }
}

/// Google access tokens for Gmail, Calendar and Contacts (the core's `GoogleTokenProvider`). Signing in
/// (`GoogleSignIn.authorize`, in the app) stores one refresh token per account and service in the keychain, readable
/// after the first unlock and in the shared group so the notification extension can refresh too; this turns them into
/// access tokens, cached in memory until shortly before they expire. Without a stored token, or once Google refuses
/// it, the core hears `NeedsUserInteraction` and the screens ask the user to sign in again.
final class GoogleTokens: GoogleTokenProvider, @unchecked Sendable {
    static let shared = GoogleTokens()

    let keychain: GoogleKeychain
    private let cache = GoogleTokenCache()
    private let http: GoogleHTTP

    init(keychain: GoogleKeychain = GoogleKeychain(), http: GoogleHTTP = GoogleHTTP()) {
        self.keychain = keychain
        self.http = http
    }

    func accessToken(account: String, service: String) async throws -> String {
        let config = GoogleConfig.current
        guard config.isConfigured else { throw ForeignError.NeedsUserInteraction }
        // An empty account is Android's "the phone's default Google account"; here: the one signed in for this service.
        let email = account.isEmpty ? keychain.accounts(service: service).first : Self.normalized(account)
        guard let email else { throw ForeignError.NeedsUserInteraction }
        do {
            return try await cache.token(for: Self.key(service, email)) {
                try await self.refresh(service: service, email: email, config: config)
            }
        } catch let error as GoogleAuthError {
            switch error {
            case .revoked, .missingScopes, .notConfigured, .cancelled, .denied: throw ForeignError.NeedsUserInteraction
            case .network, .failed: throw ForeignError.Failed(reason: error.errorDescription ?? "Google did not answer")
            }
        }
    }

    /// Whether `email` is signed in for `service` on this phone.
    func isSignedIn(_ email: String, service: String) -> Bool { keychain.load(service: service, email: Self.normalized(email)) != nil }

    /// Keeps what a sign-in returned.
    func store(service: String, email: String, refreshToken: String, scopes: [String], accessToken: String, expiresIn: Int) async throws {
        let email = Self.normalized(email)
        try keychain.save(GoogleKeychain.Grant(refreshToken: refreshToken, scopes: scopes), service: service, email: email)
        await cache.put(accessToken, expiresIn: expiresIn, for: Self.key(service, email))
    }

    /// Forgets `account`'s access to `service` on this phone, and asks Google to revoke it once no other service of
    /// that account still uses this app (Google ends every grant of the app for an account at once).
    func revoke(account: String, service: String) async {
        let email = Self.normalized(account)
        let grant = keychain.load(service: service, email: email)
        keychain.delete(service: service, email: email)
        await cache.drop(Self.key(service, email))
        let stillUsed = ["gmail", "gcalendar", "gcontacts"].contains { $0 != service && keychain.load(service: $0, email: email) != nil }
        if let grant, !stillUsed { await http.revoke(grant.refreshToken) }
    }

    private func refresh(service: String, email: String, config: GoogleConfig) async throws -> (String, Int) {
        guard let grant = keychain.load(service: service, email: email) else { throw GoogleAuthError.revoked }
        // A build that asks for more than the stored grant covers needs a new consent.
        guard Set(GoogleScopes.of(service)).isSubset(of: Set(grant.scopes)) else { throw GoogleAuthError.revoked }
        let response = try await http.token([
            "grant_type": "refresh_token",
            "refresh_token": grant.refreshToken,
            "client_id": config.clientId,
        ])
        if response.error == "invalid_grant" {
            keychain.delete(service: service, email: email)
            throw GoogleAuthError.revoked
        }
        guard let token = response.accessToken else {
            throw GoogleAuthError.failed("Google did not hand out an access token (\(response.error ?? "no reason")).")
        }
        if let rotated = response.refreshToken, rotated != grant.refreshToken {
            try? keychain.save(GoogleKeychain.Grant(refreshToken: rotated, scopes: grant.scopes), service: service, email: email)
        }
        return (token, response.expiresIn ?? 3600)
    }

    static func normalized(_ email: String) -> String { email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }

    private static func key(_ service: String, _ email: String) -> String { "\(service) \(email)" }
}

/// Access tokens in memory, and one refresh at a time per account and service.
actor GoogleTokenCache {
    private var tokens: [String: (token: String, expires: Date)] = [:]
    private var running: [String: Task<(String, Int), Error>] = [:]
    /// Renewed this long before Google's expiry.
    static let margin: TimeInterval = 60

    func token(for key: String, refresh: @escaping @Sendable () async throws -> (String, Int)) async throws -> String {
        if let cached = tokens[key], cached.expires.timeIntervalSinceNow > Self.margin { return cached.token }
        let task: Task<(String, Int), Error>
        if let existing = running[key] {
            task = existing
        } else {
            task = Task { try await refresh() }
            running[key] = task
        }
        defer { running[key] = nil }
        let (token, expiresIn) = try await task.value
        tokens[key] = (token, Date().addingTimeInterval(TimeInterval(expiresIn)))
        return token
    }

    func put(_ token: String, expiresIn: Int, for key: String) {
        tokens[key] = (token, Date().addingTimeInterval(TimeInterval(expiresIn)))
    }

    func drop(_ key: String) { tokens[key] = nil }
}

/// Google's token and revoke endpoints.
struct GoogleHTTP: Sendable {
    struct TokenResponse: Decodable {
        var accessToken: String?
        var expiresIn: Int?
        var refreshToken: String?
        var scope: String?
        var idToken: String?
        var error: String?
        var errorDescription: String?
    }

    var session: URLSession = .shared

    func token(_ form: [String: String]) async throws -> TokenResponse {
        var request = URLRequest(url: GoogleConfig.tokenURL, timeoutInterval: 20)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Self.formBody(form)
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw GoogleAuthError.network(error.localizedDescription)
        }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let parsed = try? decoder.decode(TokenResponse.self, from: data) else {
            let status = (response as? HTTPURLResponse)?.statusCode ?? 0
            throw GoogleAuthError.failed("Google answered \(status).")
        }
        return parsed
    }

    func revoke(_ token: String) async {
        var request = URLRequest(url: GoogleConfig.revokeURL, timeoutInterval: 15)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.httpBody = Self.formBody(["token": token])
        _ = try? await session.data(for: request)
    }

    static func formBody(_ form: [String: String]) -> Data {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        return form.sorted { $0.key < $1.key }
            .map { "\($0.key)=\($0.value.addingPercentEncoding(withAllowedCharacters: allowed) ?? "")" }
            .joined(separator: "&")
            .data(using: .utf8) ?? Data()
    }
}

/// One refresh token per (service, account) in the keychain: generic passwords of service
/// `com.reins2fa.app.google`, account `<service> <address>`, not synced, readable after the first unlock, in the
/// shared group when the build has one.
struct GoogleKeychain: Sendable {
    struct Grant: Codable, Equatable {
        var refreshToken: String
        var scopes: [String]
    }

    var service = "com.reins2fa.app.google"

    private func base() -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let group = AppGroup.keychainGroup { query[kSecAttrAccessGroup as String] = group }
        return query
    }

    private func item(_ googleService: String, _ email: String) -> String { "\(googleService) \(email)" }

    func save(_ grant: Grant, service googleService: String, email: String) throws {
        let data = try JSONEncoder().encode(grant)
        var query = base()
        query[kSecAttrAccount as String] = item(googleService, email)
        let update: [String: Any] = [kSecValueData as String: data]
        var status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            query[kSecValueData as String] = data
            query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            status = SecItemAdd(query as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw GoogleAuthError.failed("The keychain did not keep the Google sign-in (\(status)).") }
    }

    func load(service googleService: String, email: String) -> Grant? {
        var query = base()
        query[kSecAttrAccount as String] = item(googleService, email)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let data = out as? Data else { return nil }
        return try? JSONDecoder().decode(Grant.self, from: data)
    }

    func delete(service googleService: String, email: String) {
        var query = base()
        query[kSecAttrAccount as String] = item(googleService, email)
        SecItemDelete(query as CFDictionary)
    }

    /// The addresses signed in for `googleService`, sorted.
    func accounts(service googleService: String) -> [String] {
        var query = base()
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var out: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &out) == errSecSuccess, let rows = out as? [[String: Any]] else { return [] }
        let prefix = googleService + " "
        return rows.compactMap { $0[kSecAttrAccount as String] as? String }
            .filter { $0.hasPrefix(prefix) }
            .map { String($0.dropFirst(prefix.count)) }
            .sorted()
    }
}
