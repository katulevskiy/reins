import Foundation

/// The sign-in page of a service the core signs in to itself (`kind` "oauth": X). The core makes the page's address
/// (`oauthBegin`, with its state and PKCE challenge), the page opens in the system's web sheet, and the redirect to the
/// service's `com.reins2fa.app://<host>` comes straight back here. Only that exact address is returned for
/// `oauthFinish`, which checks the state and trades the code.
///
///     let authorizeUrl = try await core.oauthBegin(service: "x")
///     guard let redirect = try await OAuthSignIn.run(service: "x", authorizeUrl: authorizeUrl) else { return }
///     let account = try await core.oauthFinish(service: "x", callback: redirect)
@MainActor
enum OAuthSignIn {
    /// The address the page came back to; nil when the user closed it. Throws `IntegrationsFailure` in words.
    static func run(service: String, authorizeUrl: String) async throws -> String? {
        guard let host = OAuthLogic.redirectHost(service) else {
            throw IntegrationsFailure("This integration cannot be added here.")
        }
        guard OAuthLogic.isSignInPage(authorizeUrl), let url = URL(string: authorizeUrl) else {
            throw IntegrationsFailure("The sign-in page is not a web address.")
        }
        let redirect: URL
        do {
            redirect = try await WebAuth.run(url, callbackScheme: AppRedirect.scheme)
        } catch WebAuth.Failure.cancelled {
            return nil
        } catch WebAuth.Failure.noWindow {
            throw IntegrationsFailure("Open Reins to sign in.")
        } catch let WebAuth.Failure.failed(reason) {
            throw IntegrationsFailure(reason)
        }
        guard AppRedirect.matches(redirect.absoluteString, host: host) else {
            throw IntegrationsFailure("The sign-in page sent the browser somewhere else.")
        }
        return redirect.absoluteString
    }
}
