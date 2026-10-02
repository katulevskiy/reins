import AuthenticationServices
import Contacts
import EventKit
import UIKit

/// What the integration screens need from the platform layer: Google's sign-in, this phone's permissions, and the
/// browser sheet an MCP server's sign-in runs in. The screens only talk to this, so the platform pieces
/// (`GoogleSignIn`, `PhoneBridge.requestAccess`, `McpSignIn`) plug in here in one place, and tests use a fake.
@MainActor
protocol IntegrationsPlatform: AnyObject {
    /// This build has a Google OAuth client. Without one every Google integration "needs setup".
    var googleConfigured: Bool { get }
    /// What to tell the user when `googleConfigured` is false.
    var googleSetupMessage: String { get }
    /// Shows Google's consent page for `service` ("gmail", "gcalendar", "gcontacts"), for `loginHint` when asking an
    /// account again, and returns the address that signed in; nil when the user closed the page.
    func authorizeGoogle(service: String, loginHint: String?) async throws -> String?
    /// Takes this phone's Google access for (account, service) away. Best effort: without a network the access is
    /// still gone on this phone.
    func revokeGoogle(account: String, service: String) async
    /// Shows the system's prompt for this phone's calendar or contacts (iOS asks once; after that only Settings can
    /// change it) and says whether access is granted now.
    func requestPhoneAccess(service: String) async -> Bool
    /// Runs an MCP server's sign-in page and finishes the sign-in with the core. Sets `model.mcpNotice` and plays
    /// `.connected` / `.error` the way Android's redirect handling does.
    func mcpSignIn(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInOutcome
}

/// How an MCP sign-in ended.
enum McpSignInOutcome: Equatable {
    case done(serverId: String)
    case failed(serverId: String)
    /// The user closed the page: nothing was sent to the core, nothing to show.
    case cancelled
}

/// A failure the platform layer explains in its own words.
struct IntegrationsFailure: LocalizedError, Equatable {
    var message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
}

extension AppModel {
    /// The platform the integration screens use: the real one, or the `-demo` stand-in that needs no Google client,
    /// no permissions and no network.
    var integrations: IntegrationsPlatform {
        demo ? DemoIntegrations.shared : SystemIntegrations.shared
    }

    /// Records how an MCP sign-in ended, in Android's words, plays its sound and re-reads the servers.
    func finishMcpSignIn(_ result: Result<McpServerView, Error>, serverId: String) async -> McpSignInOutcome {
        let outcome: McpSignInOutcome
        switch result {
        case let .success(server):
            mcpNotice = McpNotice(serverId: server.id, text: "Signed in to \(untrusted(server.name)). Its tools can be used now.", failed: false)
            feedback.play(.connected)
            outcome = .done(serverId: server.id)
        case let .failure(error):
            mcpNotice = McpNotice(serverId: serverId, text: "Signing in did not work: \(error.userMessage)", failed: true)
            feedback.play(.error)
            outcome = .failed(serverId: serverId)
        }
        await refreshPending()
        return outcome
    }
}

// MARK: The real platform

/// The system's pieces. Google's sign-in comes with the platform layer (`GoogleSignIn`); until it is wired in here
/// a build with a client id says so instead of failing somewhere deeper. Phone permissions and the MCP browser
/// sheet work as they are.
@MainActor
final class SystemIntegrations: NSObject, IntegrationsPlatform, ASWebAuthenticationPresentationContextProviding {
    static let shared = SystemIntegrations()

    /// The client id and redirect scheme from Info.plist (`REINS_GOOGLE_CLIENT_ID`, `REINS_GOOGLE_REDIRECT_SCHEME`);
    /// an unset build setting leaves them empty.
    var googleConfigured: Bool {
        let info = Bundle.main.infoDictionary ?? [:]
        return [info["ReinsGoogleClientId"], info["ReinsGoogleRedirectScheme"]].allSatisfy {
            let value = ($0 as? String ?? "").trimmingCharacters(in: .whitespaces)
            return !value.isEmpty && !value.hasPrefix("$")
        }
    }

    let googleSetupMessage =
        "Google access needs setup: create an iOS OAuth client in Google Cloud, put its client id and reversed client id in REINS_GOOGLE_CLIENT_ID and REINS_GOOGLE_REDIRECT_SCHEME, and enable the Gmail, Calendar and People APIs."

    func authorizeGoogle(service: String, loginHint: String?) async throws -> String? {
        guard googleConfigured else { throw IntegrationsFailure(googleSetupMessage) }
        throw IntegrationsFailure("Google sign-in is not part of this build.")
    }

    func revokeGoogle(account: String, service: String) async {}

    func requestPhoneAccess(service: String) async -> Bool {
        switch service {
        case "device_calendar":
            return (try? await EKEventStore().requestFullAccessToEvents()) ?? false
        case "device_contacts":
            let granted = (try? await CNContactStore().requestAccess(for: .contacts)) ?? false
            let status = CNContactStore.authorizationStatus(for: .contacts)
            return granted || status == .authorized || status == .limited
        default:
            return false
        }
    }

    func mcpSignIn(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInOutcome {
        guard let url = URL(string: authorizeUrl), McpLogic.isWebPage(authorizeUrl) else {
            return await model.finishMcpSignIn(.failure(IntegrationsFailure("The server's sign-in page is not a web address.")), serverId: serverId)
        }
        let redirect: URL
        do {
            redirect = try await webAuth(url, callbackScheme: McpLogic.redirectScheme)
        } catch ASWebAuthenticationSessionError.canceledLogin {
            return .cancelled
        } catch {
            return await model.finishMcpSignIn(.failure(error), serverId: serverId)
        }
        guard McpLogic.isRedirect(redirect.absoluteString) else {
            return await model.finishMcpSignIn(.failure(IntegrationsFailure("The sign-in page sent the browser somewhere else.")), serverId: serverId)
        }
        let result: Result<McpServerView, Error>
        do {
            result = .success(try await model.core.mcpFinishSignIn(serverId: serverId, redirectUrl: redirect.absoluteString))
        } catch {
            result = .failure(error)
        }
        return await model.finishMcpSignIn(result, serverId: serverId)
    }

    /// The window the web sheet is shown over, found before it starts.
    private var anchor: UIWindow!

    /// The system's web sheet; it returns the address the page redirected to.
    private func webAuth(_ url: URL, callbackScheme: String) async throws -> URL {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let windows = scenes.filter { $0.activationState == .foregroundActive }.flatMap(\.windows) + scenes.flatMap(\.windows)
        guard let window = windows.first(where: \.isKeyWindow) ?? windows.first else {
            throw IntegrationsFailure("Open Reins to sign in.")
        }
        anchor = window
        return try await withCheckedThrowingContinuation { continuation in
            let session = ASWebAuthenticationSession(url: url, callback: .customScheme(callbackScheme)) { callback, error in
                if let callback {
                    continuation.resume(returning: callback)
                } else {
                    continuation.resume(throwing: error ?? ASWebAuthenticationSessionError(.canceledLogin))
                }
            }
            session.presentationContextProvider = self
            // Keep the browser's cookies: the user is usually signed in to the service there already.
            session.prefersEphemeralWebBrowserSession = false
            if !session.start() {
                continuation.resume(throwing: IntegrationsFailure("The sign-in page could not be opened. Open Reins and try again."))
            }
        }
    }

    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        // Only asked after `webAuth` set `anchor` and started the session.
        MainActor.assumeIsolated { anchor }
    }
}

// MARK: The -demo platform

/// Signs in to anything at once: Google answers with the asked address (or a new one), the phone allows
/// everything, and an MCP sign-in comes back after a moment as if the user had signed in.
@MainActor
final class DemoIntegrations: IntegrationsPlatform {
    static let shared = DemoIntegrations()

    var googleConfigured: Bool { true }
    var googleSetupMessage: String { SystemIntegrations.shared.googleSetupMessage }

    func authorizeGoogle(service: String, loginHint: String?) async throws -> String? {
        try await Task.sleep(for: .milliseconds(500))
        return loginHint ?? (service == "gmail" ? "new.account@gmail.com" : "me@gmail.com")
    }

    func revokeGoogle(account: String, service: String) async {}

    func requestPhoneAccess(service: String) async -> Bool { true }

    func mcpSignIn(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInOutcome {
        try? await Task.sleep(for: .milliseconds(900))
        let redirect = "\(McpLogic.redirectScheme)://\(McpLogic.redirectHost)?code=demo&state=demo"
        do {
            return await model.finishMcpSignIn(.success(try await model.core.mcpFinishSignIn(serverId: serverId, redirectUrl: redirect)), serverId: serverId)
        } catch {
            return await model.finishMcpSignIn(.failure(error), serverId: serverId)
        }
    }
}
