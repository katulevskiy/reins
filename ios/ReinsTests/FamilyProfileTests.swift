@testable import Reins
import XCTest

/// The profiles on this phone in memory (a family's joins like a work profile, marked as a family).
private final class FamilyRegistry: ProfileRegistryProtocol, @unchecked Sendable {
    var profiles = [FamilyRegistry.profile("personal", .personal, "Personal")]
    var pending: [AccountProfile] = []
    var activeId = "personal"
    var removed: [String] = []

    static func profile(_ id: String, _ kind: ProfileKind, _ label: String, family: Bool = false) -> AccountProfile {
        AccountProfile(
            id: id, kind: kind, label: label, orgId: nil, email: nil, serverUrl: nil, dataDir: "/data/\(id)/core",
            keyAlias: kind == .personal ? "" : id, channelPrefix: kind == .personal ? "" : "\(id).", family: family)
    }

    func active() -> AccountProfile { profiles.first { $0.id == activeId }! }
    func beginManaged(serverUrl: String) throws -> AccountProfile {
        var p = FamilyRegistry.profile("work-00000000000\(pending.count + 1)", .managed, "Work")
        p.serverUrl = serverUrl
        pending.append(p)
        return p
    }
    func confirmManaged(id: String, orgId: String, orgName: String, email: String) throws -> AccountProfile {
        guard var p = pending.first(where: { $0.id == id }) else { throw CoreError.Invalid(reason: "No such profile") }
        pending.removeAll { $0.id == id }
        p.label = orgName
        p.orgId = orgId
        p.email = email
        profiles.append(p)
        return p
    }
    func confirmFamily(id: String, orgId: String, orgName: String, email: String) throws -> AccountProfile {
        var p = try confirmManaged(id: id, orgId: orgId, orgName: orgName, email: email)
        p.family = true
        profiles = profiles.map { $0.id == p.id ? p : $0 }
        return p
    }
    func list() -> [AccountProfile] { profiles }
    func notePersonal(email: String?) throws {}
    func remove(id: String) throws {
        removed.append(id)
        pending.removeAll { $0.id == id }
        profiles.removeAll { $0.id == id }
    }
    func setActive(id: String) throws { activeId = id }
}

private func lees(minor: Bool, guardian: Bool = false, leaves: [FamilyLeaveAsk] = []) -> FamilyView {
    FamilyView(
        orgId: "f1", name: "The Lees", minor: minor, guardian: guardian, guardians: ["mom@example.com"], noticeDays: 3,
        leave: nil, minorProposed: false,
        sees: ["Requests from your agents in this profile", "What guardians decided", "Not your Personal profile"],
        leaves: leaves)
}

/// A family profile's core: the family it joins (nil: the server refused the account), and what it was asked.
private final class FakeFamilyCore: FamilySignInCore, FamilyCore, @unchecked Sendable {
    var family: FamilyView?
    var begins: [String] = []
    var joins: [String] = []
    var consents: [String] = []

    func ssoBegin(serverUrl: String) async throws -> SsoStart {
        begins.append(serverUrl)
        return SsoStart(url: "\(serverUrl)/identity/connect/authorize?state=st4te", callbackScheme: "com.reins2fa.app",
                        state: "st4te", verifier: "v")
    }
    func ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome {
        SsoOutcome(session: SessionInfo(serverUrl: serverUrl, email: "sam.lee@gmail.com"), keys: .created)
    }
    func familyJoin(link: String) async throws -> FamilyView {
        joins.append(link)
        guard let family else { throw CoreError.Invalid(reason: "Use a new address for your family account.") }
        return family
    }
    func registerDevice(fcmToken: String?) async throws {}
    func pushTag() async throws -> String? { nil }

    func familyView() async throws -> FamilyView? { family }
    func familyRequestLeave() async throws -> FamilyLeaveInfo {
        guard let f = family else { throw CoreError.NotFound }
        let leave = FamilyLeaveInfo(requestedAt: 100, effectiveAt: f.minor ? nil : 100 + Int64(f.noticeDays) * 86_400,
                                    needsConsent: f.minor)
        family?.leave = leave
        return leave
    }
    func familyCancelLeave() async throws { family?.leave = nil }
    func familyConsentLeave(member: String) async throws {
        consents.append(member)
        family?.leaves.removeAll { $0.member == member }
    }
    func familyAnswerMinor(accept: Bool) async throws {
        // A copy: reading `family` while writing through `family?` overlaps (an exclusivity error).
        guard var f = family else { return }
        f.minor = f.minor || accept
        f.minorProposed = false
        family = f
    }
}

@MainActor
final class FamilyProfileTests: XCTestCase {
    private let link = "reins-family://reins.example/" + String(repeating: "A", count: 43)

    override func tearDown() {
        WorkProfiles.active = WorkProfiles.personal
    }

    private func flow(_ core: FakeFamilyCore, _ registry: FamilyRegistry) -> AddFamilyAccount {
        AddFamilyAccount(
            server: "https://reins.example", registry: registry, makeCore: { _ in core },
            browse: { url, _ in
                // An ordinary sign-in on the family profile's own core: no domain hint, a new account.
                XCTAssertFalse(url.absoluteString.contains("domain_hint"))
                return URL(string: "com.reins2fa.app://sso-callback?code=c&state=st4te")!
            })
    }

    func testJoinWithInvite() async {
        let core = FakeFamilyCore()
        core.family = lees(minor: true)
        let registry = FamilyRegistry()
        let f = flow(core, registry)
        await f.run(link: link)
        guard case let .added(label, id) = f.step else { return XCTFail("not added: \(String(describing: f.error))") }
        XCTAssertEqual(label, "The Lees")
        XCTAssertEqual(core.joins, [link])
        XCTAssertTrue(registry.list().contains { $0.id == id && $0.family && $0.kind == .managed })
    }

    func testAnAccountThatIsNotNewIsRefusedAndItsProfileRemoved() async {
        let core = FakeFamilyCore()
        let registry = FamilyRegistry()
        let f = flow(core, registry)
        await f.run(link: link)
        XCTAssertEqual(f.error, "Use a new address for your family account.")
        XCTAssertEqual(registry.list().count, 1)
        XCTAssertEqual(registry.removed.count, 1)
        let other = flow(core, registry)
        await other.run(link: "https://example.com")
        XCTAssertEqual(other.error, "This isn't a family invitation.")
        XCTAssertTrue(core.begins.count == 1, "nothing starts for a link that isn't an invitation")
    }

    func testMinorLeaveWaits() async {
        let core = FakeFamilyCore()
        core.family = lees(minor: true)
        let m = FamilyModel(core: core)
        await m.leave()
        XCTAssertEqual(m.view?.leave?.needsConsent, true)
        XCTAssertNil(m.view?.leave?.effectiveAt)
        // An adult's counts down the notice instead.
        core.family = lees(minor: false)
        await m.leave()
        let adult = m.view?.leave
        XCTAssertEqual(adult?.needsConsent, false)
        XCTAssertEqual(FamilyModel.leavesIn(adult?.effectiveAt ?? 0, now: 100), "Leaves in 3 days")
        // A guardian allows a minor's request.
        core.family = lees(minor: false, guardian: true,
                           leaves: [FamilyLeaveAsk(member: "u-sam", email: "sam@example.com", minor: true, requestedAt: 1, effectiveAt: nil)])
        await m.allow("u-sam")
        XCTAssertEqual(core.consents, ["u-sam"])
        XCTAssertEqual(m.view?.leaves.isEmpty, true)
    }

    func testFamilyBadge() {
        let family = WorkProfiles.of(FamilyRegistry.profile("work-f1", .managed, "The Lees", family: true))
        let work = WorkProfiles.of(FamilyRegistry.profile("work-a1", .managed, "Acme"))
        XCTAssertTrue(family.family)
        XCTAssertFalse(work.family)
        XCTAssertEqual(WorkProfiles.badge(family), "The Lees")
        XCTAssertNil(WorkProfiles.badge(WorkProfiles.personal))
        // A family's pushes open its page and say what waits, nothing more.
        let leave = PushPayload(kind: PushPayload.orgReview, id: "family:leave")
        XCTAssertEqual(leave?.link, .family)
        XCTAssertEqual(leave?.familyText, "A request to leave waits for you.")
        XCTAssertEqual(PushPayload(kind: PushPayload.orgReview, id: "r1")?.link, .orgReviews)
        XCTAssertNil(PushPayload(kind: PushPayload.orgReview, id: "r1")?.familyText)
        XCTAssertEqual(DeepLink(url: DeepLink.family.url), .family)
        XCTAssertEqual(PushPayload(kind: PushPayload.orgReview, id: "family:minor")?.familyText, "Your family asks you something.")
        XCTAssertNil(PushPayload(kind: PushPayload.orgReview, id: "family:other"), "only the family asks the server sends")
        XCTAssertNil(PushPayload(kind: "req", id: "family:leave"), "a family ask is an org review's")
    }
}
