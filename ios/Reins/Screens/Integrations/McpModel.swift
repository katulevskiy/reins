import Foundation
import Observation

/// The MCP server screens: adding a server, signing in to it, its tools, removing it (the Android app's
/// `McpViewModel`). The list itself is `model.mcpServers`, re-read with `refreshPending()`.
@Observable
@MainActor
final class McpModel {
    private(set) var busy = false
    var error: String?
    /// The server whose sign-in page is open, until it comes back.
    private(set) var signingIn: String?

    private let model: AppModel
    private let platform: IntegrationsPlatform

    init(model: AppModel, platform: IntegrationsPlatform? = nil) {
        self.model = model
        self.platform = platform ?? model.integrations
    }

    func server(_ id: String) -> McpServerView? { model.mcpServers.first { $0.id == id } }

    /// Runs `block` as one operation: no second one starts meanwhile, and failures are shown in plain words.
    private func operation(_ block: () async throws -> Void) async {
        guard !busy else { return }
        busy = true
        let epoch = model.accountEpoch
        error = nil
        defer { busy = false }
        do {
            try await block()
        } catch is CancellationError {
        } catch {
            guard epoch == model.accountEpoch else { return }
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// Adds the server at `url`: with `token` as its access token when one is given, else by asking it (it may want a
    /// sign-in, whose page opens). Returns the new server's id once there is one to show.
    func add(url: String, name: String, token: String) async -> String? {
        var added: String?
        await operation {
            let epoch = model.accountEpoch
            signingIn = nil
            let address = url.trimmingCharacters(in: .whitespacesAndNewlines)
            let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
            let label = trimmed.isEmpty ? nil : trimmed
            let secret = token.trimmingCharacters(in: .whitespacesAndNewlines)
            if !secret.isEmpty {
                let server = try await model.core.mcpAddWithToken(url: address, token: secret, name: label)
                guard epoch == model.accountEpoch else { return }
                model.serverConnected(server, epoch: epoch)
                if server.status == "ok" { model.feedback.play(.connected) }
                await model.refreshPending()
                if epoch == model.accountEpoch { added = server.id }
            } else {
                added = try await step(model.core.mcpAdd(url: address, name: label), epoch: epoch)
            }
        }
        return added
    }

    /// Connects to the server again; a sign-in that ended opens its page again.
    func refresh(_ id: String) async {
        await operation {
            model.feedback.play(.refresh)
            signingIn = nil
            let epoch = model.accountEpoch
            _ = try await step(model.core.mcpRefresh(id: id), epoch: epoch)
        }
    }

    /// What the server answered: added, or a sign-in first. Returns the server to show, nil when there is none (the
    /// page was not opened, or closed before the sign-in finished).
    private func step(_ step: McpAddStep, epoch: UInt64) async throws -> String? {
        guard epoch == model.accountEpoch else { return nil }
        switch step {
        case let .added(server):
            model.serverConnected(server, epoch: epoch)
            if server.status == "ok" { model.feedback.play(.connected) }
            await model.refreshPending()
            return epoch == model.accountEpoch ? server.id : nil
        case let .needsSignIn(serverId, authorizeUrl):
            await model.refreshPending()
            guard epoch == model.accountEpoch else { return nil }
            guard McpLogic.isWebPage(authorizeUrl) else {
                throw IntegrationsFailure("This server's sign-in page is not a web page, so it was not opened.")
            }
            signingIn = serverId
            defer { signingIn = nil }
            switch await platform.mcpSignIn(serverId: serverId, authorizeUrl: authorizeUrl, model: model) {
            case let .done(id), let .failed(id): return epoch == model.accountEpoch ? id : nil
            case .cancelled: return nil
            }
        }
    }

    /// The switch moves at once; only the servers are read again (the old value comes back after a refusal).
    func setHeavy(_ id: String, tool: String, heavy: Bool) async {
        guard !busy else { return }
        model.patchMcpTool(serverId: id, tool: tool, heavy: heavy)
        await operation {
            try await model.core.mcpSetHeavy(id: id, tool: tool, heavy: heavy)
        }
        await model.refreshMcpServers()
    }

    /// Removes the server; true once it is gone.
    func remove(_ id: String) async -> Bool {
        var removed = false
        await operation {
            model.feedback.play(.revoked)
            let epoch = model.accountEpoch
            try await model.core.mcpRemove(id: id)
            guard epoch == model.accountEpoch else { return }
            model.connectedServers[id] = nil
            await model.refreshPending()
            removed = true
        }
        return removed
    }
}
