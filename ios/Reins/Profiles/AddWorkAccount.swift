import CryptoKit
import DeviceCheck
import Foundation
import UIKit

/// What a work profile's core does while it signs in and enrolls (the real `ReinsCore`; a fake in tests).
protocol WorkSignInCore: AnyObject, Sendable {
    func ssoBeginWork(serverUrl: String, domain: String) async throws -> SsoStart
    func ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome
    func orgMembership() async throws -> OrgView?
    func registerDevice(fcmToken: String?) async throws
    func orgChallenge() async throws -> String
    func orgEnroll(challenge: String, input: EnrollInput) async throws
}

extension ReinsCore: WorkSignInCore {}

/// A work profile's org device key and what vouches for it.
protocol DeviceKeys: Sendable {
    /// Makes (replacing) the key for `profileId`; `hardware`: the Secure Enclave holds it. Returns the
    /// SubjectPublicKeyInfo DER.
    func create(profileId: String) throws -> (publicKey: Data, hardware: Bool)
    /// ECDSA P-256 SHA-256 (DER) of `data`.
    func sign(profileId: String, data: Data) throws -> Data
    /// App Attest of `challenge` (CBOR), nil when the platform can't.
    func attest(challenge: Data) async -> Data?
}

/// The Secure Enclave when the phone has one (else a software P-256 key), kept in the Keychain; App Attest.
final class EnclaveDeviceKeys: DeviceKeys, @unchecked Sendable {
    private var keys: [String: SecureEnclave.P256.Signing.PrivateKey] = [:]
    private var soft: [String: P256.Signing.PrivateKey] = [:]
    private let lock = NSLock()

    func create(profileId: String) throws -> (publicKey: Data, hardware: Bool) {
        lock.lock(); defer { lock.unlock() }
        if SecureEnclave.isAvailable {
            let key = try SecureEnclave.P256.Signing.PrivateKey()
            keys[profileId] = key
            KeyItem.save(key.dataRepresentation, tag: WorkProfiles.orgKeyTag(profileId: profileId))
            return (key.publicKey.derRepresentation, true)
        }
        let key = P256.Signing.PrivateKey()
        soft[profileId] = key
        KeyItem.save(key.rawRepresentation, tag: WorkProfiles.orgKeyTag(profileId: profileId))
        return (key.publicKey.derRepresentation, false)
    }

    func sign(profileId: String, data: Data) throws -> Data {
        lock.lock(); defer { lock.unlock() }
        if let key = keys[profileId] { return try key.signature(for: data).derRepresentation }
        if let key = soft[profileId] { return try key.signature(for: data).derRepresentation }
        throw ForeignError.Failed(reason: "no device key")
    }

    func attest(challenge: Data) async -> Data? {
        let service = DCAppAttestService.shared
        guard service.isSupported else { return nil }
        do {
            let keyId = try await service.generateKey()
            return try await service.attestKey(keyId, clientDataHash: Data(SHA256.hash(data: challenge)))
        } catch {
            return nil
        }
    }
}

/// The org device key's private part, kept in this device's Keychain (never synced).
private enum KeyItem {
    static func save(_ data: Data, tag: String) {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.reins2fa.app.org-device",
            kSecAttrAccount as String: tag,
            kSecUseDataProtectionKeychain as String: true,
        ]
        SecItemDelete(query as CFDictionary)
        query[kSecValueData as String] = data
        query[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(query as CFDictionary, nil)
    }
}

/// What the phone says about itself (06 §6): recorded by the org, gating comes later.
enum OrgPosture {
    /// Signs of a jailbreak: the usual files, or writing outside the sandbox. `nil`: could not tell.
    static func jailbroken(exists: (String) -> Bool, canWriteOutside: () -> Bool) -> Bool? {
        let paths = ["/Applications/Cydia.app", "/Library/MobileSubstrate/MobileSubstrate.dylib", "/bin/bash",
                     "/usr/sbin/sshd", "/etc/apt", "/private/var/lib/apt/", "/var/jb"]
        return paths.contains(where: exists) || canWriteOutside()
    }

    static func collect(screenLock: Bool = ScreenLock.isSet()) -> PostureInput {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        let jailbroken = jailbroken(exists: { FileManager.default.fileExists(atPath: $0) }) {
            let probe = "/private/reins-jb-\(UUID().uuidString)"
            guard (try? "x".write(toFile: probe, atomically: true, encoding: .utf8)) != nil else { return false }
            try? FileManager.default.removeItem(atPath: probe)
            return true
        }
        return PostureInput(
            screenLock: screenLock,
            osName: "iOS",
            osVersion: "\(v.majorVersion).\(v.minorVersion).\(v.patchVersion)",
            rooted: jailbroken,
            appVersion: (Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String) ?? "0"
        )
    }
}

/// "Add work account" (05 §9, 06 §2): the work account signs in on its own core, in its own reserved profile,
/// through its organization's SSO (the domain hint); an account that isn't an org's member is refused and its
/// profile removed. Then this phone is enrolled to the org: a Secure Enclave key, App Attest, and the posture.
@MainActor
@Observable
final class AddWorkAccount {
    enum Step: Equatable {
        case domain
        case working
        case added(label: String, id: String, enrolled: Bool)
    }

    private(set) var step: Step = .domain
    private(set) var error: String?
    let registry: ProfileRegistryProtocol
    private let server: String
    private let makeCore: (AccountProfile) throws -> WorkSignInCore
    private let browse: (URL, String) async throws -> URL
    private let keys: DeviceKeys
    private let posture: () -> PostureInput

    init(
        server: String,
        registry: ProfileRegistryProtocol = WorkProfiles.registry(),
        makeCore: @escaping (AccountProfile) throws -> WorkSignInCore,
        browse: @escaping (URL, String) async throws -> URL = { try await WebAuth.run($0, callbackScheme: $1) },
        keys: DeviceKeys = EnclaveDeviceKeys(),
        posture: @escaping () -> PostureInput = { OrgPosture.collect() }
    ) {
        self.server = server
        self.registry = registry
        self.makeCore = makeCore
        self.browse = browse
        self.keys = keys
        self.posture = posture
    }

    func run(domain: String) async {
        guard step != .working else { return }
        step = .working
        error = nil
        var reserved: AccountProfile?
        do {
            let profile = try registry.beginManaged(serverUrl: server)
            reserved = profile
            let core = try makeCore(profile)
            let start = try await core.ssoBeginWork(serverUrl: server, domain: domain.trimmingCharacters(in: .whitespaces))
            guard let url = URL(string: start.url) else { throw ForeignError.Failed(reason: "The sign-in page didn't open.") }
            let callback = try await browse(url, start.callbackScheme)
            let outcome = try await core.ssoFinish(serverUrl: server, callbackUrl: callback.absoluteString, state: start.state, verifier: start.verifier)
            guard let org = try await core.orgMembership() else {
                try? registry.remove(id: profile.id)
                step = .domain
                error = "This account isn't managed by an organization."
                return
            }
            let added = try registry.confirmManaged(id: profile.id, orgId: org.orgId, orgName: org.name, email: outcome.session.email)
            reserved = nil
            let enrolled = await enroll(core: core, profileId: added.id)
            step = .added(label: added.label, id: added.id, enrolled: enrolled)
        } catch {
            if let reserved { try? registry.remove(id: reserved.id) }
            step = .domain
            self.error = error.userMessage
        }
    }

    private func enroll(core: WorkSignInCore, profileId: String) async -> Bool {
        do {
            try await core.registerDevice(fcmToken: nil)
            let challenge = try await core.orgChallenge()
            let key = try keys.create(profileId: profileId)
            let signature = try keys.sign(profileId: profileId, data: WorkProfiles.enrollmentMessage(challenge: challenge))
            let attestation = await keys.attest(challenge: Data(challenge.utf8))
            let input = EnrollInput(
                deviceKey: key.publicKey.base64EncodedString(),
                signature: signature.base64EncodedString(),
                hardware: key.hardware,
                attestations: [attestation.map { AttestationInput(kind: .appAttest, data: $0.base64EncodedString()) }
                    ?? AttestationInput(kind: .none, data: "")],
                posture: posture()
            )
            try await core.orgEnroll(challenge: challenge, input: input)
            return true
        } catch {
            return false
        }
    }
}
