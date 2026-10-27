import Foundation

/// Builds the Rust core the same way in the app and in the notification extension: one store in the App Group
/// container, the keychain key wrapper, Google tokens from the keychain, and this phone's calendar and contacts.
enum CoreFactory {
    /// `<App Group>/Library/Application Support/core`: the Personal profile's store.
    static var personalDataDir: URL { AppGroup.directory("core") }

    /// The active profile ([`WorkProfiles`]): Personal unless Settings switched to a work profile.
    static func activeProfile() -> AccountProfile? {
        let profile = WorkProfiles.registry().active()
        WorkProfiles.active = WorkProfiles.of(profile)
        WorkProfiles.hasWork = WorkProfiles.registry().list().contains { $0.kind == .managed }
        return profile
    }

    /// The active profile's store (the Personal one's is today's).
    static var dataDir: URL {
        guard let dir = activeProfile()?.dataDir, !dir.isEmpty else { return personalDataDir }
        let url = URL(fileURLWithPath: dir, isDirectory: true)
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    /// The active profile's data-key Keychain account.
    static var keyAccount: String { WorkProfiles.keychainAccount(keyAlias: WorkProfiles.active.keyAlias) }

    /// How long a store lock waits for the other process (the extension, or the app) before the call fails as busy.
    /// The other holds it for milliseconds at a time; longer means it is stuck. The extension, with its few seconds,
    /// gives up sooner and leaves the push to the app.
    static let appLockWaitMillis: UInt32 = 3_000
    static let extensionLockWaitMillis: UInt32 = 1_500

    /// Whether the app ever opened the store here (the notification extension never creates one).
    static var storeExists: Bool { FileManager.default.fileExists(atPath: dataDir.appendingPathComponent("dek.bin").path) }

    /// A work profile's own core while it signs in ("Add work account"): its own store and Keychain item, and no
    /// notifications until it is the active profile. It never reaches the phone's data (contacts, calendar).
    static func makeWork(_ profile: AccountProfile) throws -> ReinsCore {
        try FileManager.default.createDirectory(atPath: profile.dataDir, withIntermediateDirectories: true)
        let info = Bundle.main.infoDictionary ?? [:]
        return try ReinsCore(
            dataDir: profile.dataDir,
            keys: KeychainKeyWrapper(account: WorkProfiles.keychainAccount(keyAlias: profile.keyAlias)),
            google: GoogleTokens.of(WorkProfiles.of(profile)),
            notifier: SilentNotifier(),
            device: nil,
            telegramApiId: Int32((info["ReinsTelegramApiId"] as? String) ?? "") ?? 0,
            telegramApiHash: (info["ReinsTelegramApiHash"] as? String) ?? ""
        )
    }

    /// `keys`: the extension passes `KeychainKeyWrapper(mayReset: false)` so it can never make the core start over.
    static func make(
        notifier: Notifier,
        google: GoogleTokenProvider? = nil,
        keys: KeyWrapper? = nil
    ) throws -> ReinsCore {
        let dataDir = dataDir
        let profile = WorkProfiles.active
        // Each profile's Google sign-ins are its own; the phone's own data is the Personal profile's.
        let google = google ?? GoogleTokens.of(profile)
        let device: DeviceBridge? = profile.managed ? nil : PhoneBridge.shared
        let keys = keys ?? KeychainKeyWrapper(account: keyAccount)
        let info = Bundle.main.infoDictionary ?? [:]
        let apiId = Int32((info["ReinsTelegramApiId"] as? String) ?? "") ?? 0
        let apiHash = (info["ReinsTelegramApiHash"] as? String) ?? ""
        return try ReinsCore(
            dataDir: dataDir.path,
            keys: keys,
            google: google,
            notifier: notifier,
            device: device,
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
            // `reason` is often the server's error code ("bad_request"), so it never stands alone (as on Android).
            case let .Server(status, reason):
                if status == 403 { return "Another phone is your approval device now." }
                return "The server had a problem (\(status)). \(untrusted(reason))".trimmingCharacters(in: .whitespaces)
            case .GmailNeedsConsent: return "Gmail needs your consent again."
            case let .Gmail(reason): return reason
            case let .Service(reason): return reason
            case let .ServiceNeedsAttention(reason): return reason
            case .NotFound: return "That is no longer there."
            case let .Invalid(reason): return reason
            case let .Storage(reason): return "Storage problem: \(reason)"
            case .OtherApprovalDevice:
                return "This account already has a phone for approvals. Approve this phone from it, or enter your recovery code."
            case .ViewerOnly: return "Decisions are made on the primary phone."
            case let .RecoveryNeedsConfirmation(reason, _): return reason
            case .NotAViewer: return "Visible on the approving phone."
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

/// Posts nothing: a work profile's core while it signs in, before it is the active profile.
final class SilentNotifier: Notifier, @unchecked Sendable {
    func itemPending(item: PendingItem) {}
    func itemResolved(id: String) {}
    func autoDecided(decision: AutoDecisionView) {}
    func autopilotChanged(event: AutopilotEvent) {}
    func circleChanged(event: CircleEvent) {}
}
