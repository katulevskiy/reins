import Foundation

/// Builds the Rust core the same way in the app and in the notification extension: one store in the App Group
/// container, the keychain key wrapper, Google tokens from the keychain, and this phone's calendar and contacts.
enum CoreFactory {
    /// `<App Group>/Library/Application Support/core`.
    static var dataDir: URL { AppGroup.directory("core") }

    /// Whether the app ever opened the store here (the notification extension never creates one).
    static var storeExists: Bool { FileManager.default.fileExists(atPath: dataDir.appendingPathComponent("dek.bin").path) }

    /// `keys`: the extension passes `KeychainKeyWrapper(mayReset: false)` so it can never make the core start over.
    static func make(
        notifier: Notifier,
        google: GoogleTokenProvider = GoogleTokens.shared,
        keys: KeyWrapper = KeychainKeyWrapper()
    ) throws -> RewardenCore {
        let info = Bundle.main.infoDictionary ?? [:]
        let apiId = Int32((info["ReinsTelegramApiId"] as? String) ?? "") ?? 0
        let apiHash = (info["ReinsTelegramApiHash"] as? String) ?? ""
        return try RewardenCore(
            dataDir: dataDir.path,
            keys: keys,
            google: google,
            notifier: notifier,
            device: PhoneBridge.shared,
            telegramApiId: apiId,
            telegramApiHash: apiHash
        )
    }
}

extension Error {
    /// A sentence safe to show: the core's messages never contain secrets or URLs.
    var userMessage: String {
        switch self {
        case let e as CoreError:
            switch e {
            case .NotLoggedIn: return "You are signed out."
            case .TwoFactorRequired: return "Enter the code from your authenticator app."
            case .UnsupportedTwoFactor: return "This account's two-step method is not supported. Use an authenticator app."
            case .InvalidCredentials: return "Wrong email, password or code."
            case let .Network(reason): return "No connection: \(reason)"
            case let .Server(status, reason): return reason.isEmpty ? "The server answered \(status)." : reason
            case .GmailNeedsConsent: return "Gmail needs your consent again."
            case let .Gmail(reason): return reason
            case let .Service(reason): return reason
            case let .ServiceNeedsAttention(reason): return reason
            case .NotFound: return "That is no longer there."
            case let .Invalid(reason): return reason
            case let .Storage(reason): return "Storage problem: \(reason)"
            }
        case let e as ForeignError:
            switch e {
            case .NeedsUserInteraction: return "This needs you to allow it first."
            case let .Failed(reason): return reason
            }
        default:
            return localizedDescription
        }
    }
}
