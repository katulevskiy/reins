@testable import Reins
import UserNotifications
import XCTest

/// The profiles on this phone in memory, as the core's registry keeps them.
private final class FakeRegistry: ProfileRegistryProtocol, @unchecked Sendable {
    var profiles = [FakeRegistry.profile("personal", .personal, "Personal")]
    var pending: [AccountProfile] = []
    var activeId = "personal"
    var personalEmail: String?
    var removed: [String] = []

    static func profile(_ id: String, _ kind: ProfileKind, _ label: String) -> AccountProfile {
        AccountProfile(
            id: id, kind: kind, label: label, orgId: nil, email: nil, serverUrl: nil, dataDir: "/data/\(id)/core",
            keyAlias: kind == .personal ? "" : id, channelPrefix: kind == .personal ? "" : "\(id).")
    }

    func active() -> AccountProfile { profiles.first { $0.id == activeId }! }
    func beginManaged(serverUrl: String) throws -> AccountProfile {
        var p = FakeRegistry.profile("work-00000000000\(pending.count + 1)", .managed, "Work")
        p.serverUrl = serverUrl
        pending.append(p)
        return p
    }
    func confirmManaged(id: String, orgId: String, orgName: String, email: String) throws -> AccountProfile {
        if email == personalEmail { throw CoreError.Invalid(reason: "This is your personal account.") }
        guard var p = pending.first(where: { $0.id == id }) else { throw CoreError.Invalid(reason: "No such profile") }
        pending.removeAll { $0.id == id }
        p.label = orgName
        p.orgId = orgId
        p.email = email
        profiles.append(p)
        return p
    }
    func list() -> [AccountProfile] { profiles }
    func notePersonal(email: String?) throws { personalEmail = email }
    func remove(id: String) throws {
        removed.append(id)
        pending.removeAll { $0.id == id }
        profiles.removeAll { $0.id == id }
    }
    func setActive(id: String) throws {
        guard profiles.contains(where: { $0.id == id }) else { throw CoreError.Invalid(reason: "No such profile") }
        activeId = id
    }
}

/// A work account's core: its org (nil for a Personal account), and what it was asked.
private final class FakeWorkCore: WorkSignInCore, @unchecked Sendable {
    var org: OrgView?
    var begins: [(String, String)] = []
    var enrollments: [(String, EnrollInput)] = []

    func ssoBeginWork(serverUrl: String, domain: String) async throws -> SsoStart {
        begins.append((serverUrl, domain))
        return SsoStart(url: "\(serverUrl)/identity/connect/authorize?domain_hint=\(domain)", callbackScheme: "com.reins2fa.app",
                        state: "st4te", verifier: "v")
    }
    func ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome {
        SsoOutcome(session: SessionInfo(serverUrl: serverUrl, email: "dana@acme.com"), keys: .created)
    }
    func orgMembership() async throws -> OrgView? { org }
    func registerDevice(fcmToken: String?) async throws {}
    func orgChallenge() async throws -> String { "challenge-1" }
    func orgEnroll(challenge: String, input: EnrollInput) async throws { enrollments.append((challenge, input)) }
    func pushTag() async throws -> String? { "bbbbbbbbbbbbbbbb" }
}

private struct FakeKeys: DeviceKeys {
    func create(profileId: String) throws -> (publicKey: Data, hardware: Bool) { (Data([0x30, 0x59]), true) }
    func sign(profileId: String, data: Data) throws -> Data {
        XCTAssertEqual(data, Data("reins/org-enroll/v1\nchallenge-1".utf8))
        return Data([0x30, 0x44])
    }
    func attest(clientDataHash: Data) async -> (keyId: String, attestation: Data)? {
        // Bound to the key and the enrollment (`reins_proto::org::attestation_binding`).
        XCTAssertEqual(clientDataHash, WorkProfiles.attestationBinding(challenge: "challenge-1", deviceKey: Data([0x30, 0x59])))
        return ("key-id", Data("attested".utf8))
    }
}

final class WorkProfilesTests: XCTestCase {
    override func tearDown() {
        WorkProfiles.active = WorkProfiles.personal
        WorkProfiles.hasWork = false
    }

    func testPersonalKeepsTodaysKeychainItemAndWorkProfilesGetTheirOwn() {
        let acme = WorkProfiles.of(FakeRegistry.profile("work-1", .managed, "Acme"))
        XCTAssertEqual(WorkProfiles.keychainAccount(keyAlias: WorkProfiles.personal.keyAlias), "core-dek-wrap-v1")
        XCTAssertEqual(WorkProfiles.keychainAccount(keyAlias: acme.keyAlias), "core-dek-wrap-v1.work-1")
        XCTAssertNotEqual(WorkProfiles.orgKeyTag(profileId: "work-1"), WorkProfiles.orgKeyTag(profileId: "work-2"))
    }

    func testThreadsArePrefixedPerProfile() {
        XCTAssertEqual(NotificationCategory.request.thread, "requests")
        WorkProfiles.active = WorkProfiles.of(FakeRegistry.profile("work-1", .managed, "Acme"))
        XCTAssertEqual(NotificationCategory.request.thread, "work-1.requests")
        XCTAssertEqual(NotificationCategory.grant.thread, "work-1.grants")
    }

    func testTheEnrollmentMessageIsTheProtocols() {
        XCTAssertEqual(WorkProfiles.enrollmentMessage(challenge: "abc"), Data("reins/org-enroll/v1\nabc".utf8))
    }

    func testAttestationsAreBoundToTheEnrolledKey() {
        // The vector `reins_proto::org` pins.
        let binding = WorkProfiles.attestationBinding(challenge: "challenge-1", deviceKey: Data([0x30, 0x59]))
        XCTAssertEqual(binding.map { String(format: "%02x", $0) }.joined(),
                       "1dc1c8bb30a580b9a299e73e68abe5ee7907fd2b53d3d7af42ac4e530aee873e")
        XCTAssertNotEqual(binding, WorkProfiles.attestationBinding(challenge: "challenge-1", deviceKey: Data([0x30, 0x5a])))
    }

    func testOrgLabelsAreShownAsUntrustedNames() {
        var p = FakeRegistry.profile("work-1", .managed, "Acme")
        p.label = "Acme\u{202E}lanosreP"
        XCTAssertFalse(WorkProfiles.of(p).label.contains("\u{202E}"))
    }
}

final class ProfileBadgeTests: XCTestCase {
    func testWorkCardsCarryTheOrgName() {
        XCTAssertEqual(WorkProfiles.badge(WorkProfiles.of(FakeRegistry.profile("work-1", .managed, "Acme"))), "Acme")
    }

    func testPersonalCardsCarryNone() {
        XCTAssertNil(WorkProfiles.badge(WorkProfiles.personal))
    }
}

final class OrgPostureTests: XCTestCase {
    func testJailbreakSignsAreReported() {
        XCTAssertEqual(OrgPosture.jailbroken(exists: { _ in false }, canWriteOutside: { false }), false)
        XCTAssertEqual(OrgPosture.jailbroken(exists: { $0 == "/Applications/Cydia.app" }, canWriteOutside: { false }), true)
        XCTAssertEqual(OrgPosture.jailbroken(exists: { _ in false }, canWriteOutside: { true }), true)
    }

    func testPostureReportsPasscodeOSAndAppVersion() {
        let p = OrgPosture.collect(screenLock: true)
        XCTAssertTrue(p.screenLock)
        XCTAssertEqual(p.osName, "iOS")
        XCTAssertFalse(p.osVersion.isEmpty)
        XCTAssertFalse(p.appVersion.isEmpty)
    }
}

@MainActor
final class AddWorkAccountTests: XCTestCase {
    private func flow(_ core: FakeWorkCore, _ registry: FakeRegistry) -> AddWorkAccount {
        AddWorkAccount(
            server: "https://reins.example", registry: registry, makeCore: { _ in core },
            browse: { url, _ in
                XCTAssertTrue(url.absoluteString.contains("domain_hint=acme.com"))
                return URL(string: "com.reins2fa.app://sso-callback?code=c&state=st4te")!
            },
            keys: FakeKeys(),
            posture: { PostureInput(screenLock: true, osName: "iOS", osVersion: "26.1", rooted: false, appVersion: "1.0") }
        )
    }

    func testAWorkAccountIsAddedAndThePhoneEnrolled() async {
        let core = FakeWorkCore()
        core.org = OrgView(orgId: "o1", name: "Acme", tier: "business", roles: ["Approver"], enrolled: false, githubOrgs: [])
        let registry = FakeRegistry()
        let add = flow(core, registry)
        await add.run(domain: " acme.com ")
        guard case let .added(label, _, enrolled) = add.step else { return XCTFail("\(add.step)") }
        XCTAssertEqual(label, "Acme")
        XCTAssertTrue(enrolled)
        XCTAssertEqual(core.begins.first?.1, "acme.com")
        XCTAssertEqual(core.enrollments.first?.0, "challenge-1")
        XCTAssertEqual(core.enrollments.first?.1.attestations.first?.kind, .appAttest)
        XCTAssertTrue(registry.list().contains { $0.label == "Acme" && $0.kind == .managed })
    }

    func testAnAccountWithoutAnOrgIsRefused() async {
        let registry = FakeRegistry()
        let add = flow(FakeWorkCore(), registry)
        await add.run(domain: "acme.com")
        XCTAssertEqual(add.error, "This account isn't managed by an organization.")
        XCTAssertEqual(add.step, .domain)
        XCTAssertEqual(registry.list().count, 1)
        XCTAssertEqual(registry.removed.count, 1)
    }

    func testThePersonalAccountIsNeverAddedAsWork() async {
        let core = FakeWorkCore()
        core.org = OrgView(orgId: "o1", name: "Acme", tier: "team", roles: [], enrolled: false, githubOrgs: [])
        let registry = FakeRegistry()
        registry.personalEmail = "dana@acme.com"
        let add = flow(core, registry)
        await add.run(domain: "acme.com")
        XCTAssertNotNil(add.error)
        XCTAssertEqual(registry.list().count, 1)
        XCTAssertTrue(core.enrollments.isEmpty)
    }
}

/// Switching profiles moves the app in place: the registry's active profile changes and the rebuild runs, and nothing
/// terminates the process.
@MainActor
final class ProfileSwitchInPlaceTests: XCTestCase {
    private final class Registry: ProfileRegistryProtocol, @unchecked Sendable {
        var activeId = "personal"
        func active() -> AccountProfile { fatalError("unused") }
        func beginManaged(serverUrl: String) throws -> AccountProfile { fatalError("unused") }
        func confirmManaged(id: String, orgId: String, orgName: String, email: String) throws -> AccountProfile { fatalError("unused") }
        func list() -> [AccountProfile] { [] }
        func notePersonal(email: String?) throws {}
        func remove(id: String) throws {}
        func setActive(id: String) throws { activeId = id }
    }

    func testSwitchingRebuildsInPlace() {
        let saved = ProfileSwitch.relaunch
        defer { ProfileSwitch.relaunch = saved }
        var rebuilt = 0
        ProfileSwitch.relaunch = { rebuilt += 1 }
        let registry = Registry()
        ProfileSwitch.switchTo("work-1", registry: registry)
        XCTAssertEqual(registry.activeId, "work-1")
        XCTAssertEqual(rebuilt, 1, "the switch rebuilds the app; the test process is still running, so nothing exited")
    }

    func testTheHostSwitchesToANewModelWithoutQuitting() async throws {
        let host = try XCTUnwrap(AppHost.current, "the test host app made one")
        let before = host.model
        await host.switchProfile()
        XCTAssertFalse(host.switching)
        XCTAssertNotNil(host.model, "the profile's model is open again")
        XCTAssertFalse(host.model === before, "a fresh model, not the old one")
    }
}

/// Which profile a notification, an action on it, or a push belongs to: never acted on by another profile's core.
final class WorkProfileRoutingTests: XCTestCase {
    private let work = WorkProfiles.Active(id: "work-1", label: "Acme", managed: true, threadPrefix: "work-1.", keyAlias: "work-1")
    private let tags = ["personal": "aaaaaaaaaaaaaaaa", "work-1": "bbbbbbbbbbbbbbbb"]

    func testPushTagsAreSixteenLowercaseHexDigits() {
        XCTAssertEqual(PushPayload.tag(["p": "0123456789abcdef"]), "0123456789abcdef")
        XCTAssertNil(PushPayload.tag(["p": "0123"]))
        XCTAssertNil(PushPayload.tag(["p": "0123456789ABCDEF"]))
        XCTAssertNil(PushPayload.tag([:]))
    }

    func testAPushGoesToTheProfileWhoseTagItCarries() {
        let personal = WorkProfiles.personal
        let forWork: [AnyHashable: Any] = ["t": "req", "id": "r1", "p": "bbbbbbbbbbbbbbbb"]
        XCTAssertFalse(WorkProfiles.belongsHere(forWork, remote: true, active: personal, hasWork: true, tags: tags))
        XCTAssertEqual(WorkProfiles.owner(of: forWork, remote: true, active: personal, hasWork: true, tags: tags), "work-1")
        XCTAssertTrue(WorkProfiles.belongsHere(forWork, remote: true, active: work, hasWork: true, tags: tags))
        // A tag nobody here knows is no profile's.
        let unknown: [AnyHashable: Any] = ["t": "req", "id": "r1", "p": "cccccccccccccccc"]
        XCTAssertNil(WorkProfiles.owner(of: unknown, remote: true, active: personal, hasWork: true, tags: tags))
        // No tag (an older server), or no work profile: the running profile's, as before.
        XCTAssertTrue(WorkProfiles.belongsHere(["t": "req", "id": "r1"], remote: true, active: personal, hasWork: true, tags: tags))
        XCTAssertTrue(WorkProfiles.belongsHere(unknown, remote: true, active: personal, hasWork: false, tags: [:]))
    }

    func testANotificationPostedHereNamesItsProfile() {
        let content = UNMutableNotificationContent()
        content.userInfo = ["t": "req", "id": "r1"]
        let stamped = WorkProfiles.stamp(content, profile: work)
        XCTAssertEqual(stamped.userInfo[WorkProfiles.notificationKey] as? String, "work-1")
        XCTAssertFalse(WorkProfiles.belongsHere(stamped.userInfo, remote: false, active: WorkProfiles.personal, hasWork: true, tags: tags))
        XCTAssertTrue(WorkProfiles.belongsHere(stamped.userInfo, remote: false, active: work, hasWork: true, tags: tags))
        // One posted before profiles is the Personal profile's.
        XCTAssertFalse(WorkProfiles.belongsHere(content.userInfo, remote: false, active: work, hasWork: true, tags: tags))
    }

    func testGrantRemindersAreKeptPerProfile() {
        let now: Int64 = 1_700_000_000
        let grant = GrantView(
            id: "g1", connectionId: "c1", connectionLabel: "Claude", action: "read", summary: "Read mail",
            expiresAt: now + 600, maxUses: nil, uses: 0, createdAt: now - 3_000, lastUsedAt: nil, origin: "approval",
            service: "gmail", account: "me@gmail.com", lines: [], active: true, state: "active", allMail: false,
            editableScope: nil
        )
        let personal = GrantReminders.plan([grant], now: now)
        let ours = GrantReminders.plan([grant], now: now, profile: work)
        XCTAssertNotEqual(personal[0].identifier, ours[0].identifier)
        XCTAssertNotEqual(GrantReminders.scheduledKey(WorkProfiles.personal), GrantReminders.scheduledKey(work))
        let c = GrantReminders.content(ours[0], now: now, settings: FeedbackSettings(), profile: work)
        XCTAssertEqual(c.userInfo[WorkProfiles.notificationKey] as? String, "work-1")
        XCTAssertTrue(c.threadIdentifier.hasPrefix("work-1."))
    }
}
