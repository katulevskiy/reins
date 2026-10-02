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
    }

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
        clearError()
        defer { busy = false }
        do {
            try await block()
        } catch is CancellationError {
        } catch {
            model.feedback.play(.error)
            self.error = error.userMessage
        }
    }

    /// An account was added: it sounds, and the lists catch up.
    private func connected() async {
        model.feedback.play(.connected)
        await refresh()
    }

    // MARK: Google

    /// Google's sign-in for this service, then the account is registered in the core (which checks with Google
    /// that the access works). `hint` asks one account again.
    func addGoogle(hint: String? = nil) async {
        await operation {
            guard platform.googleConfigured else { throw IntegrationsFailure(platform.googleSetupMessage) }
            guard let address = try await platform.authorizeGoogle(service: service, loginHint: hint) else { return }
            if isGmail {
                _ = try await model.core.addAccount(hint: address)
            } else {
                _ = try await model.core.addServiceAccount(service: service, hint: address)
            }
            await connected()
        }
    }

    // MARK: An access token or the vault's master password

    func addSecret(_ secret: String) async {
        await operation {
            _ = try await model.core.addTokenAccount(service: service, token: secret.trimmingCharacters(in: .whitespacesAndNewlines))
            await connected()
        }
    }

    // MARK: This phone's calendar and contacts

    /// Asks iOS for the permission, then connects the phone's calendar or contacts.
    func addDevice() async {
        guard !busy else { return }
        guard await platform.requestPhoneAccess(service: service) else {
            fail(ServiceCopy.permissionRefused, refusedByIOS: true)
            return
        }
        await operation {
            _ = try await model.core.addServiceAccount(service: service, hint: "")
            await connected()
        }
    }

    // MARK: A phone number and a code (Telegram)

    func loginBegin(_ phone: String) async {
        let number = phone.trimmingCharacters(in: .whitespacesAndNewlines)
        await operation {
            try await model.core.loginBegin(service: service, phone: number)
            login = .code(phone: number)
        }
    }

    func loginCode(_ code: String) async {
        await operation {
            switch try await model.core.loginCode(service: service, code: code.trimmingCharacters(in: .whitespaces)) {
            case let .needsPassword(hint):
                login = .password(hint: hint)
            case .done:
                login = .phone
                await connected()
            }
        }
    }

    func loginPassword(_ password: String) async {
        await operation {
            _ = try await model.core.loginPassword(service: service, password: password)
            login = .phone
            await connected()
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
            if isGmail {
                try await model.core.removeAccount(account: account)
            } else {
                try await model.core.removeServiceAccount(service: service, account: account)
            }
            statuses[account] = nil
            await refresh()
            if google { await platform.revokeGoogle(account: account, service: service) }
        }
    }
}
