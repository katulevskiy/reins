import Foundation
import Observation

/// The accounts of one integration: how each is doing, adding one (the way depends on the kind of service), removing
/// one (the Android app's `GmailViewModel` and `ServiceViewModel`). Gmail has its own calls in the core
/// (`accounts`, `accountStatus`, `addAccount`, `removeAccount`); every other service goes through the service ones.
@Observable
@MainActor
final class AccountsModel {
    let service: String
    /// Per account: nil while it is being checked.
    private(set) var statuses: [String: GmailStatus] = [:]
    var error: String?
    /// The last failure was iOS refusing a permission: Settings is the only place to change that now.
    private(set) var refusedByIOS = false
    private(set) var busy = false
    private(set) var login: TelegramStep = .phone

    private let model: AppModel
    private let platform: IntegrationsPlatform

    init(service: String, model: AppModel, platform: IntegrationsPlatform? = nil) {
        self.service = service
        self.model = model
        self.platform = platform ?? model.integrations
        #if DEBUG
        // `-demo -integration telegram -telegramCode`: the code step, for screenshots.
        if model.demo, service == "telegram", ProcessInfo.processInfo.arguments.contains("-telegramCode") { login = .code(phone: "+1 555 010 0199") }
        #endif
    }

    var connectedAccount: String? { model.connectedAccounts[service] }
    func dismissConnection() { model.connectedAccounts[service] = nil }

    var isGmail: Bool { service == "gmail" }

    /// The service as the core lists it; "sms", which the app never offers, as unavailable.
    var view: ServiceView? {
        if isGmail {
            let base = model.services.first { $0.service == "gmail" }
                ?? ServiceView(service: "gmail", name: "Gmail", kind: "google", available: true, note: nil, accounts: [])
            var v = base
            v.accounts = model.accounts.filter { $0.service == "gmail" }
            return v
        }
        if service == ServiceCopy.sms.service { return ServiceCopy.sms }
        return model.services.first { $0.service == service }
    }

    var accounts: [AccountView] { view?.accounts ?? [] }

    // MARK: Status

    /// Re-reads the accounts, then checks each one (a network call each).
    func refresh() async {
        await model.refreshPending()
        for account in accounts.map(\.account) {
            let status = isGmail
                ? await model.core.accountStatus(account: account)
                : await model.core.serviceAccountStatus(service: service, account: account)
            statuses[account] = status
        }
    }

    /// Shows a failure that happened outside the core (iOS refused a permission).
    func fail(_ message: String, refusedByIOS: Bool = false) {
        model.feedback.play(.error)
        error = message
        self.refusedByIOS = refusedByIOS
    }

    func clearError() {
        error = nil
        refusedByIOS = false
    }

    /// Runs `block` as one operation: no second one starts meanwhile, and failures are shown in plain words.
    private func operation(_ block: () async throws -> Void) async {
        guard !busy else { return }
        busy = true
        let epoch = model.accountEpoch
        clearError()
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

    /// An account was added: it sounds, and the lists catch up.
    private func connected(_ account: AccountView, epoch: UInt64) async {
        guard epoch == model.accountEpoch else { return }
        model.accountConnected(account, epoch: epoch)
        model.feedback.play(.connected)
        await refresh()
    }

    // MARK: Google

    /// Google's sign-in for this service, then the account is registered in the core (which checks with Google
    /// that the access works). `hint` asks one account again.
    func addGoogle(hint: String? = nil) async {
        await operation {
            let epoch = model.accountEpoch
            guard platform.googleConfigured else { throw IntegrationsFailure(platform.googleSetupMessage) }
            guard let address = try await platform.authorizeGoogle(service: service, loginHint: hint) else { return }
            guard epoch == model.accountEpoch else { return }
            let account = isGmail
                ? try await model.core.addAccount(hint: address)
                : try await model.core.addServiceAccount(service: service, hint: address)
            await connected(account, epoch: epoch)
        }
    }

    // MARK: An access token or the vault's master password

    func addSecret(_ secret: String) async {
        await operation {
            let epoch = model.accountEpoch
            let account = try await model.core.addTokenAccount(service: service, token: secret.trimmingCharacters(in: .whitespacesAndNewlines))
            await connected(account, epoch: epoch)
        }
    }

    // MARK: This phone's calendar and contacts

    /// Asks iOS for the permission, then connects the phone's calendar or contacts.
    func addDevice() async {
        await operation {
            let epoch = model.accountEpoch
            let allowed = await platform.requestPhoneAccess(service: service)
            guard epoch == model.accountEpoch else { return }
            guard allowed else {
                fail(ServiceCopy.permissionRefused, refusedByIOS: true)
                return
            }
            let account = try await model.core.addServiceAccount(service: service, hint: "")
            await connected(account, epoch: epoch)
        }
    }

    // MARK: A phone number and a code (Telegram)

    func loginBegin(_ phone: String) async {
        let number = phone.trimmingCharacters(in: .whitespacesAndNewlines)
        await operation {
            let epoch = model.accountEpoch
            try await model.core.loginBegin(service: service, phone: number)
            guard epoch == model.accountEpoch else { return }
            login = .code(phone: number)
        }
    }

    func loginCode(_ code: String) async {
        await operation {
            let epoch = model.accountEpoch
            let progress = try await model.core.loginCode(service: service, code: code.trimmingCharacters(in: .whitespaces))
            guard epoch == model.accountEpoch else { return }
            switch progress {
            case let .needsPassword(hint):
                login = .password(hint: hint)
            case let .done(account):
                guard epoch == model.accountEpoch else { return }
                login = .phone
                model.accountConnected(service: service, account: account, epoch: epoch)
                model.feedback.play(.connected)
                await refresh()
            }
        }
    }

    func loginPassword(_ password: String) async {
        await operation {
            let epoch = model.accountEpoch
            let account = try await model.core.loginPassword(service: service, password: password)
            guard epoch == model.accountEpoch else { return }
            login = .phone
            await connected(account, epoch: epoch)
        }
    }

    /// Starts the sign-in over (a wrong number, a code that never came).
    func loginRestart() {
        login = .phone
        clearError()
    }

    // MARK: Removing

    /// Disconnects `account`: Reins forgets it (signing it out where that applies) and Google is told to revoke.
    func remove(_ account: String) async {
        let google = view?.kind == "google"
        await operation {
            model.feedback.play(.revoked)
            let epoch = model.accountEpoch
            if isGmail {
                try await model.core.removeAccount(account: account)
            } else {
                try await model.core.removeServiceAccount(service: service, account: account)
            }
            guard epoch == model.accountEpoch else { return }
            if connectedAccount == account { dismissConnection() }
            statuses[account] = nil
            await refresh()
            if google { await platform.revokeGoogle(account: account, service: service) }
        }
    }
}
