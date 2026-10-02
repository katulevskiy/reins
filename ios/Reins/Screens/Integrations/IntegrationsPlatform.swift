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

/// The system's pieces: Google's sign-in (`GoogleSignIn`), this phone's permissions (`PhoneBridge`) and the MCP
/// browser sheet (`McpSignIn`).
@MainActor
final class SystemIntegrations: IntegrationsPlatform {
    static let shared = SystemIntegrations()

    var googleConfigured: Bool { GoogleSignIn.shared.isConfigured }

    let googleSetupMessage = GoogleConfig.setupMessage

    func authorizeGoogle(service: String, loginHint: String?) async throws -> String? {
        do {
            return try await GoogleSignIn.shared.authorize(service: service, loginHint: loginHint)
        } catch GoogleAuthError.cancelled {
            return nil
        }
    }

    func revokeGoogle(account: String, service: String) async {
        await GoogleSignIn.shared.revoke(account: account, service: service)
    }

    func requestPhoneAccess(service: String) async -> Bool {
        await PhoneBridge.shared.requestAccess(service: service)
    }

    func mcpSignIn(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInOutcome {
        guard McpLogic.isWebPage(authorizeUrl) else {
            return await model.finishMcpSignIn(.failure(IntegrationsFailure("The server's sign-in page is not a web address.")), serverId: serverId)
        }
        switch await McpSignIn.run(serverId: serverId, authorizeUrl: authorizeUrl, model: model) {
        case let .done(server): return .done(serverId: server.id)
        case let .failed(id, _): return .failed(serverId: id)
        case .cancelled: return .cancelled
        }
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
