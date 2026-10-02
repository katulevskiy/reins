import Foundation

/// How a sign-in to an MCP server ended (the Android app's `McpSignInResult`).
enum McpSignInResult: Equatable {
    case done(McpServerView)
    case failed(serverId: String, message: String)
    /// The user closed the page: nothing was sent to the core, nothing to show.
    case cancelled
}

/// The sign-in to an MCP server that needs one (`McpAddStep.needsSignIn`, `mcpRefresh`): its page opens in the system's
/// web sheet, the redirect to the core's fixed `dev.rewarden.android://mcp-oauth` comes straight back here, and the
/// core finishes it (it checks state and PKCE). On iOS the sheet returns in-process, so nothing waits on disk the way
/// Android's sign-in survives the app being stopped.
///
/// The MCP screens:
///
///     case let .needsSignIn(serverId, authorizeUrl):
///         let result = await McpSignIn.run(serverId: serverId, authorizeUrl: authorizeUrl, model: model)
///
/// It plays `.connected` / `.error`, sets `model.mcpNotice` with Android's wording and reloads the servers.
@MainActor
enum McpSignIn {
    static let redirectScheme = "dev.rewarden.android"
    static let redirectHost = "mcp-oauth"
    private static let maxRedirectChars = 8_192

    static func run(serverId: String, authorizeUrl: String, model: AppModel) async -> McpSignInResult {
        guard let url = URL(string: authorizeUrl), isWebPage(url) else {
            return finish(.failed(serverId: serverId, message: "The server's sign-in page is not a web address."), model: model)
        }
        let redirect: URL
        do {
            redirect = try await WebAuth.run(url, callbackScheme: redirectScheme)
        } catch WebAuth.Failure.cancelled {
            return .cancelled
        } catch WebAuth.Failure.noWindow {
            return finish(.failed(serverId: serverId, message: "Open Reins to sign in."), model: model)
        } catch let WebAuth.Failure.failed(reason) {
            return finish(.failed(serverId: serverId, message: reason), model: model)
        } catch {
            return finish(.failed(serverId: serverId, message: error.userMessage), model: model)
        }
        guard isRedirect(redirect.absoluteString) else {
            return finish(.failed(serverId: serverId, message: "The sign-in page sent the browser somewhere else."), model: model)
        }
        let result: McpSignInResult
        do {
            result = .done(try await model.core.mcpFinishSignIn(serverId: serverId, redirectUrl: redirect.absoluteString))
        } catch {
            result = .failed(serverId: serverId, message: error.userMessage)
        }
        return finish(result, model: model)
    }

    private static func finish(_ result: McpSignInResult, model: AppModel) -> McpSignInResult {
        switch result {
        case let .done(server):
            model.mcpNotice = McpNotice(serverId: server.id, text: "Signed in to \(untrusted(server.name)). Its tools can be used now.", failed: false)
            model.feedback.play(.connected)
        case let .failed(id, message):
            model.mcpNotice = McpNotice(serverId: id, text: "Signing in did not work: \(message)", failed: true)
            model.feedback.play(.error)
        case .cancelled:
            return result
        }
        Task { await model.refreshPending() }
        return result
    }

    /// The core's own redirect address (anything else is never handed to it).
    static func isRedirect(_ uri: String) -> Bool {
        guard uri.count <= maxRedirectChars, let c = URLComponents(string: uri) else { return false }
        return c.scheme == redirectScheme && c.host == redirectHost && c.user == nil && c.port == nil
    }

    /// A sign-in page may only be a web page (never a `file:` or an app's address).
    static func isWebPage(_ url: URL) -> Bool {
        (url.scheme == "https" || url.scheme == "http") && !(url.host ?? "").isEmpty
    }
}
