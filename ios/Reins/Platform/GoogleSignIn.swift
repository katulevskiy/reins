import CryptoKit
import Foundation

/// Signing in to Google for Gmail, Calendar or Contacts: OAuth 2.0 for installed apps with PKCE in the system's web
/// sheet. What it returns is kept by `GoogleTokens` (keychain), which the core then asks for access tokens.
///
/// The integration screens:
///
///     let email = try await GoogleSignIn.shared.authorize(service: "gmail", loginHint: nil)
///     _ = try await model.core.addAccount(hint: email)                         // Gmail
///     _ = try await model.core.addServiceAccount(service: "gcalendar", hint: email) // Calendar, Contacts
///
/// and, removing an account: `await GoogleSignIn.shared.revoke(account:service:)` then `core.removeAccount` /
/// `core.removeServiceAccount`. `GoogleSignIn.shared.isConfigured == false` means the build has no OAuth client: show
/// `GoogleConfig.setupMessage` ("needs setup") instead of the button. Errors are `GoogleAuthError`; `.cancelled` is the
/// user closing the page and should show nothing.
@MainActor
final class GoogleSignIn {
    static let shared = GoogleSignIn()

    private let tokens: GoogleTokens
    private let http: GoogleHTTP

    init(tokens: GoogleTokens = .shared, http: GoogleHTTP = GoogleHTTP()) {
        self.tokens = tokens
        self.http = http
    }

    var isConfigured: Bool { GoogleConfig.current.isConfigured }

    /// Shows Google's consent page for `service` ("gmail", "gcalendar", "gcontacts"), optionally for one address, and
    /// returns the address the user signed in with (lowercased).
    func authorize(service: String, loginHint: String? = nil) async throws -> String {
        let config = GoogleConfig.current
        guard config.isConfigured else { throw GoogleAuthError.notConfigured }
        let request = GoogleAuthRequest(config: config, service: service, loginHint: loginHint)
        let redirect: URL
        do {
            redirect = try await WebAuth.run(request.url, callbackScheme: config.redirectScheme)
        } catch WebAuth.Failure.cancelled {
            throw GoogleAuthError.cancelled
        } catch WebAuth.Failure.noWindow {
            throw GoogleAuthError.failed("Open Reins to sign in to Google.")
        } catch let WebAuth.Failure.failed(reason) {
            throw GoogleAuthError.failed(reason)
        }
        let code = try request.code(from: redirect)
        let response = try await http.token([
            "grant_type": "authorization_code",
            "code": code,
            "client_id": config.clientId,
            "redirect_uri": config.redirectURI,
            "code_verifier": request.verifier,
        ])
        guard let access = response.accessToken else {
            throw GoogleAuthError.failed(response.errorDescription ?? "Google did not finish the sign-in (\(response.error ?? "no reason")).")
        }
        let granted = Set((response.scope ?? "").split(separator: " ").map(String.init))
        let needed = GoogleScopes.of(service)
        guard Set(needed).isSubset(of: granted) else { throw GoogleAuthError.missingScopes(GoogleScopes.name(service)) }
        guard let email = response.idToken.flatMap(GoogleAuthRequest.email(fromIdToken:)) else {
            throw GoogleAuthError.failed("Google did not say which account signed in.")
        }
        guard let refresh = response.refreshToken else {
            throw GoogleAuthError.failed("Google did not hand over lasting access. Remove Reins at myaccount.google.com/permissions and try again.")
        }
        try await tokens.store(service: service, email: email, refreshToken: refresh, scopes: needed, accessToken: access, expiresIn: response.expiresIn ?? 3600)
        return GoogleTokens.normalized(email)
    }

    /// Takes this phone's access to `service` for `account` away.
    func revoke(account: String, service: String) async {
        await tokens.revoke(account: account, service: service)
    }
}

/// One authorization request: PKCE verifier, state, and the page's address; checks the redirect that comes back.
struct GoogleAuthRequest {
    let config: GoogleConfig
    let service: String
    let loginHint: String?
    let verifier: String
    let state: String

    init(config: GoogleConfig, service: String, loginHint: String?, verifier: String = Self.random(32), state: String = Self.random(16)) {
        self.config = config
        self.service = service
        self.loginHint = loginHint
        self.verifier = verifier
        self.state = state
    }

    var challenge: String { Self.base64URL(Data(SHA256.hash(data: Data(verifier.utf8)))) }

    var url: URL {
        var c = URLComponents(url: GoogleConfig.authorizeURL, resolvingAgainstBaseURL: false)!
        var items = [
            URLQueryItem(name: "client_id", value: config.clientId),
            URLQueryItem(name: "redirect_uri", value: config.redirectURI),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: (GoogleScopes.identity + GoogleScopes.of(service)).joined(separator: " ")),
            URLQueryItem(name: "code_challenge", value: challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
            URLQueryItem(name: "state", value: state),
        ]
        if let hint = loginHint?.trimmingCharacters(in: .whitespaces), !hint.isEmpty {
            items.append(URLQueryItem(name: "login_hint", value: hint))
        }
        c.queryItems = items
        return c.url!
    }

    /// The authorization code in `redirect`, if it is this request's answer.
    func code(from redirect: URL) throws -> String {
        guard redirect.scheme == config.redirectScheme,
              let c = URLComponents(url: redirect, resolvingAgainstBaseURL: false)
        else { throw GoogleAuthError.failed("Google sent the sign-in somewhere else.") }
        let query = Dictionary((c.queryItems ?? []).map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { a, _ in a })
        guard query["state"] == state else { throw GoogleAuthError.failed("That sign-in was not started here. Try again.") }
        if let error = query["error"] {
            throw error == "access_denied" ? GoogleAuthError.denied : GoogleAuthError.failed("Google said: \(error)")
        }
        guard let code = query["code"], !code.isEmpty else { throw GoogleAuthError.failed("Google sent no authorization code.") }
        return code
    }

    /// The `email` claim of an ID token straight from Google's token endpoint (over TLS, so its signature need not be
    /// checked for this purpose: it only names the account).
    static func email(fromIdToken token: String) -> String? {
        let parts = token.split(separator: ".")
        guard parts.count >= 2 else { return nil }
        var b64 = parts[1].replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while b64.count % 4 != 0 { b64 += "=" }
        guard let data = Data(base64Encoded: b64),
              let claims = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let email = claims["email"] as? String, email.contains("@")
        else { return nil }
        return email
    }

    static func random(_ bytes: Int) -> String {
        var raw = [UInt8](repeating: 0, count: bytes)
        _ = SecRandomCopyBytes(kSecRandomDefault, bytes, &raw)
        return base64URL(Data(raw))
    }

    static func base64URL(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
    }
}
