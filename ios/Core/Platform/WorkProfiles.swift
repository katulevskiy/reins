import CryptoKit
import Foundation
import UserNotifications

/// The profiles on this phone (Decisions v2 O1, 06 §2): the Personal account and a Managed work profile per
/// organization. Each profile runs its own core on its own data directory, its data key wrapped by its own Keychain
/// item, with its own notification threads; the app shows one profile at a time and switches in Settings. Shared by
/// the app and the notification extension, so both open the active profile's store.
enum WorkProfiles {
    /// The Personal profile's data-key Keychain account (today's: nothing moves for anyone without a work profile).
    static let dekAccount = "core-dek-wrap-v1"

    /// What the running app shows.
    struct Active: Equatable, Sendable {
        let id: String
        let label: String
        let managed: Bool
        /// Put in front of notification thread identifiers ("" for Personal).
        let threadPrefix: String
        /// Appended to the data-key Keychain account ("" for Personal).
        let keyAlias: String
    }

    static let personal = Active(id: "personal", label: "Personal", managed: false, threadPrefix: "", keyAlias: "")

    /// Set when the core is made; Personal until then.
    nonisolated(unsafe) static var active: Active = personal

    /// Whether this phone has a work profile (then pairing says which profile an agent joins).
    nonisolated(unsafe) static var hasWork = false

    static func of(_ p: AccountProfile) -> Active {
        Active(id: p.id, label: label(p), managed: p.kind == .managed, threadPrefix: p.channelPrefix, keyAlias: p.keyAlias)
    }

    /// A profile's label as shown anywhere: the org typed it, so it goes through `untrustedName` (the core already
    /// stripped control, bidi and invisible characters and kept it to 40).
    static func label(_ p: AccountProfile) -> String { untrustedName(p.label, limit: 40) }

    /// What an attestation of this enrollment vouches for (`reins_proto::org::attestation_binding`): SHA-256 of the
    /// enrollment message followed by the enrolled key (SPKI DER). App Attest's client data hash.
    static func attestationBinding(challenge: String, deviceKey: Data) -> Data {
        Data(SHA256.hash(data: enrollmentMessage(challenge: challenge) + deviceKey))
    }

    /// The Keychain account of a work profile's App Attest key id (kept for later assertions).
    static func appAttestKeyTag(profileId: String) -> String { "com.reins2fa.app.app-attest.\(profileId)" }

    /// The Keychain account that wraps a profile's data key.
    static func keychainAccount(keyAlias: String) -> String {
        keyAlias.isEmpty ? dekAccount : "\(dekAccount).\(keyAlias)"
    }

    /// The Keychain tag of a work profile's org device key (made at enrollment).
    static func orgKeyTag(profileId: String) -> String { "com.reins2fa.app.org-device.\(profileId)" }

    /// The badge on cards and rows: the org's name in a work profile, none in the Personal one.
    static func badge(_ active: Active = active) -> String? { active.managed ? active.label : nil }

    /// A notification thread of the active profile: its own, so a work profile's never mix with the Personal ones.
    static func thread(_ base: String, prefix: String = active.threadPrefix) -> String { prefix + base }

    /// What the device key signs to enroll (`reins_proto::org::enrollment_message`).
    static func enrollmentMessage(challenge: String) -> Data {
        Data("reins/org-enroll/v1\n".utf8) + Data(challenge.utf8)
    }

    // MARK: Which profile a notification is for

    /// The `userInfo` key naming the profile a notification posted on this phone belongs to.
    static let notificationKey = "profile"

    private static let pushTagsKey = "workProfiles.pushTags"

    /// Each profile's push tag (`reins_proto::pairing::push_profile_tag`), in the App Group so the notification
    /// extension reads them too. Not secret: the server sends it in the clear.
    static func rememberPushTag(_ tag: String, profileId: String, defaults: UserDefaults = AppGroup.defaults) {
        var tags = defaults.dictionary(forKey: pushTagsKey) as? [String: String] ?? [:]
        guard tags[profileId] != tag else { return }
        tags[profileId] = tag
        defaults.set(tags, forKey: pushTagsKey)
    }

    static func forgetPushTag(profileId: String, defaults: UserDefaults = AppGroup.defaults) {
        var tags = defaults.dictionary(forKey: pushTagsKey) as? [String: String] ?? [:]
        tags[profileId] = nil
        defaults.set(tags, forKey: pushTagsKey)
    }

    static func pushTags(defaults: UserDefaults = AppGroup.defaults) -> [String: String] {
        defaults.dictionary(forKey: pushTagsKey) as? [String: String] ?? [:]
    }

    /// The profile a notification belongs to: the one it names (posted here), or for a push, the profile whose tag it
    /// carries. `nil` when it can't be placed: a push whose tag no profile here knows.
    ///
    /// A push without a tag (an older server), on a phone without a work profile, is the active profile's, as before
    /// profiles; a local notification without a name was posted before profiles, by the Personal profile.
    static func owner(
        of userInfo: [AnyHashable: Any],
        remote: Bool,
        active: Active = active,
        hasWork: Bool = hasWork,
        tags: [String: String] = pushTags()
    ) -> String? {
        if let named = userInfo[notificationKey] as? String { return named }
        guard remote else { return personal.id }
        guard let tag = PushPayload.tag(userInfo), hasWork else { return active.id }
        if tags[active.id] == tag { return active.id }
        return tags.first { $0.value == tag && $0.key != active.id }?.key
    }

    /// `content`, naming `profile` (`notificationKey`): an action on it is only ever taken by that profile's core.
    static func stamp(_ content: UNNotificationContent, profile: Active) -> UNNotificationContent {
        let c = (content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        c.userInfo[notificationKey] = profile.id
        return c
    }

    /// Whether a notification may be acted on by the active profile's core.
    static func belongsHere(_ userInfo: [AnyHashable: Any], remote: Bool, active: Active = active, hasWork: Bool = hasWork, tags: [String: String] = pushTags()) -> Bool {
        owner(of: userInfo, remote: remote, active: active, hasWork: hasWork, tags: tags) == active.id
    }

    /// The folder that holds the Personal `core` directory and the registry.
    static var baseDir: URL { CoreFactory.personalDataDir.deletingLastPathComponent() }

    /// The phone's profile registry.
    static func registry() -> ProfileRegistry { ProfileRegistry(baseDir: baseDir.path) }
}
