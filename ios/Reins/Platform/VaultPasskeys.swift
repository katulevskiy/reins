import AuthenticationServices
import CryptoKit
import UIKit

/// What the platform's passkey UI gave back (the Android app's `PasskeyResult`).
enum PasskeyResult: Equatable {
    /// The passkey and its PRF output. `prf` is nil only from `PasskeyPrompting.create`, when the provider gives it only
    /// as a passkey is used (`VaultPasskeys.add` then asks for it).
    case passkey(credentialId: Data, prf: Data?)
    /// The provider has no PRF, or no passkey for the account: it cannot open the vault.
    case unsupported
    /// The user closed the prompt.
    case cancelled
    /// Anything else, in words to show.
    case failed(String)
}

/// The platform's passkey UI, for the passkey that opens the vault; tests and the demo put a fake in its place.
@MainActor
protocol PasskeyPrompting: AnyObject {
    /// Makes a passkey for the account with the WebAuthn `prf` extension.
    func create(_ options: VaultPasskeyOptions) async -> PasskeyResult
    /// Uses one of the passkeys `allowed` (credential ids), evaluating its PRF on the options' salt.
    func get(_ options: VaultPasskeyOptions, allowed: [Data]) async -> PasskeyResult
}

/// A passkey could not be made to open the vault, or could not open it; the message says why.
struct PasskeyError: LocalizedError, Equatable {
    var message: String
    var errorDescription: String? { message }
}

/// The vault passkey flows over a `PasskeyPrompting`, apart from any screen (the Android app's `addVaultPasskey`).
@MainActor
enum VaultPasskeys {
    /// The provider has no PRF: its passkeys cannot open the vault, so none is added.
    static let unsupported = "This password manager can't unlock Reins. Choose another one, or keep the recovery code."
    /// The prompt found none of the account's passkeys, or one without PRF.
    static let noneHere = "No passkey on this phone opens your vault. Ask your other phone, or enter your recovery code."

    /// "Add a passkey": makes one and has the core keep the vault's copy for it, named `name` (the phone it was made
    /// on). A provider that gives the PRF output only as a passkey is used is asked once more, for the one just made.
    /// Nil when the user closed a prompt; `PasskeyError` when the provider cannot open the vault, and nothing is added
    /// then. Only where the vault is open (the core refuses otherwise).
    static func add(core: any ReinsCoreProtocol, prompt: PasskeyPrompting, name: String) async throws -> [VaultPasskeyView]? {
        let options = try await core.vaultPasskeyOptions()
        guard let made = try passkey(await prompt.create(options), unsupported: unsupported) else { return nil }
        let prf: Data
        if let output = made.prf {
            prf = output
        } else {
            guard let used = try passkey(await prompt.get(options, allowed: [made.credentialId]), unsupported: unsupported) else { return nil }
            guard let output = used.prf else { throw PasskeyError(message: unsupported) }
            prf = output
        }
        return try await core.addVaultPasskey(credentialId: made.credentialId, prfOutput: prf, name: name)
    }

    /// "Unlock with passkey": one of the account's passkeys gives the PRF output that opens the copy of the vault's key
    /// kept for it. False when the user closed the prompt; `PasskeyError` when none of them is on this phone.
    static func unlock(core: any ReinsCoreProtocol, prompt: PasskeyPrompting) async throws -> Bool {
        let options = try await core.vaultPasskeyOptions()
        guard let used = try passkey(await prompt.get(options, allowed: options.credentialIds), unsupported: noneHere) else { return false }
        guard let prf = used.prf else { throw PasskeyError(message: noneHere) }
        try await core.unlockWithVaultPasskey(credentialId: used.credentialId, prfOutput: prf)
        return true
    }

    /// The passkey, or nil when the user cancelled; a provider that cannot give one throws.
    private static func passkey(_ result: PasskeyResult, unsupported: String) throws -> (credentialId: Data, prf: Data?)? {
        switch result {
        case let .passkey(credentialId, prf): return (credentialId, prf)
        case .cancelled: return nil
        case .unsupported: throw PasskeyError(message: unsupported)
        case let .failed(message): throw PasskeyError(message: message)
        }
    }

    /// The name a new passkey gets on the server: the phone's name, or its model where the system only gives that.
    static func deviceName() -> String {
        deviceName(name: UIDevice.current.name, model: UIDevice.current.model)
    }

    static func deviceName(name: String, model: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? model : trimmed
    }
}

/// What a registration or an assertion says about PRF, apart from the platform's types so it is unit tested.
enum PasskeyParse {
    private static let unreadable = PasskeyResult.failed("The password manager gave an answer Reins cannot read. Try again.")

    /// A new passkey: its PRF output; or none yet from a provider that has PRF (it gives it as the passkey is used); or a
    /// provider without PRF (`prfSupported` false or absent).
    static func registration(credentialId: Data, prfSupported: Bool?, first: Data?) -> PasskeyResult {
        guard !credentialId.isEmpty else { return unreadable }
        if let first, !first.isEmpty { return .passkey(credentialId: credentialId, prf: first) }
        return prfSupported == true ? .passkey(credentialId: credentialId, prf: nil) : .unsupported
    }

    /// A passkey used: its PRF output, or a provider without PRF.
    static func assertion(credentialId: Data, first: Data?) -> PasskeyResult {
        guard !credentialId.isEmpty else { return unreadable }
        if let first, !first.isEmpty { return .passkey(credentialId: credentialId, prf: first) }
        return .unsupported
    }

    /// The prompt ended with an error: closing it is no error; making one the password manager has already is said
    /// plainly; a request no provider took is a provider without the passkey (or without passkeys at all).
    static func error(_ error: Error, creating: Bool) -> PasskeyResult {
        guard let code = (error as? ASAuthorizationError)?.code else {
            return .failed(creating ? "The passkey could not be made. Try again." : "The passkey could not be used. Try again.")
        }
        switch code {
        case .canceled: return .cancelled
        case .matchedExcludedCredential: return .failed("This password manager has a passkey for your vault already.")
        case .notHandled: return .unsupported
        default: return .failed(creating ? "The passkey could not be made. Try again." : "The passkey could not be used. Try again.")
        }
    }

    /// The 32 bytes of a PRF output.
    static func bytes(_ key: SymmetricKey) -> Data { key.withUnsafeBytes { Data($0) } }
}

/// The system's passkey sheet (iCloud Keychain or the password manager the user picked) through AuthenticationServices.
@MainActor
final class PlatformPasskeys: NSObject, PasskeyPrompting {
    private var continuation: CheckedContinuation<PasskeyResult, Never>?
    private var controller: ASAuthorizationController?
    private var window: UIWindow?
    private var creating = false

    func create(_ options: VaultPasskeyOptions) async -> PasskeyResult {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: options.rpId)
        let request = provider.createCredentialRegistrationRequest(challenge: options.challenge, name: options.userName, userID: options.userHandle)
        request.userVerificationPreference = .required
        request.prf = .inputValues(.init(saltInput1: options.prfSalt))
        return await perform(request, creating: true)
    }

    func get(_ options: VaultPasskeyOptions, allowed: [Data]) async -> PasskeyResult {
        let provider = ASAuthorizationPlatformPublicKeyCredentialProvider(relyingPartyIdentifier: options.rpId)
        let request = provider.createCredentialAssertionRequest(challenge: options.challenge)
        request.allowedCredentials = allowed.map { ASAuthorizationPlatformPublicKeyCredentialDescriptor(credentialID: $0) }
        request.userVerificationPreference = .required
        request.prf = .inputValues(.init(saltInput1: options.prfSalt))
        return await perform(request, creating: false)
    }

    private func perform(_ request: ASAuthorizationRequest, creating: Bool) async -> PasskeyResult {
        guard continuation == nil else { return .failed("A passkey prompt is open already.") }
        guard let window = WebAuth.Anchor.window() else { return .failed("Open Reins and try again.") }
        return await withCheckedContinuation { continuation in
            self.continuation = continuation
            self.window = window
            self.creating = creating
            let controller = ASAuthorizationController(authorizationRequests: [request])
            controller.delegate = self
            controller.presentationContextProvider = self
            self.controller = controller
            controller.performRequests()
        }
    }

    private func finish(_ result: PasskeyResult) {
        let continuation = continuation
        self.continuation = nil
        controller = nil
        window = nil
        continuation?.resume(returning: result)
    }
}

extension PlatformPasskeys: ASAuthorizationControllerDelegate, ASAuthorizationControllerPresentationContextProviding {
    nonisolated func authorizationController(controller: ASAuthorizationController, didCompleteWithAuthorization authorization: ASAuthorization) {
        MainActor.assumeIsolated {
            switch authorization.credential {
            case let made as ASAuthorizationPlatformPublicKeyCredentialRegistration:
                finish(PasskeyParse.registration(
                    credentialId: made.credentialID, prfSupported: made.prf?.isSupported, first: made.prf?.first.map(PasskeyParse.bytes)
                ))
            case let used as ASAuthorizationPlatformPublicKeyCredentialAssertion:
                finish(PasskeyParse.assertion(credentialId: used.credentialID, first: used.prf.map { PasskeyParse.bytes($0.first) }))
            default:
                finish(.unsupported)
            }
        }
    }

    nonisolated func authorizationController(controller: ASAuthorizationController, didCompleteWithError error: Error) {
        MainActor.assumeIsolated { finish(PasskeyParse.error(error, creating: creating)) }
    }

    nonisolated func presentationAnchor(for controller: ASAuthorizationController) -> ASPresentationAnchor {
        // `perform` keeps the window for as long as the sheet shows.
        MainActor.assumeIsolated { window! }
    }
}
