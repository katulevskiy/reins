@testable import Reins
import XCTest

/// A work profile's core for the reviews: its org, what waits, and the answers it was sent.
private final class FakeOrgCore: OrgReviewsCore, @unchecked Sendable {
    var org: OrgView? = OrgView(orgId: "o1", name: "Acme", tier: "business", roles: ["Security admin"], enrolled: true, githubOrgs: [])
    var reviews: [OrgReviewView] = []
    var decisions: [(id: String, approve: Bool, at: Int64, signature: String)] = []
    var listed = 0

    func orgMembership() async throws -> OrgView? { org }
    func orgReviews() async throws -> [OrgReviewView] {
        listed += 1
        return reviews
    }
    func orgDecide(id: String, approve: Bool, at: Int64, signature: String) async throws {
        decisions.append((id, approve, at, signature))
    }
}

/// A device key that records what it signed.
private final class RecordingKeys: DeviceKeys, @unchecked Sendable {
    var signed: [(profileId: String, data: Data)] = []
    func create(profileId: String) throws -> (publicKey: Data, hardware: Bool) { (Data([0x30, 0x59]), true) }
    func sign(profileId: String, data: Data) throws -> Data {
        signed.append((profileId, data))
        return Data([0x30, 0x44])
    }
    func attest(clientDataHash: Data) async -> (keyId: String, attestation: Data)? { nil }
}

@MainActor
final class OrgReviewsTests: XCTestCase {
    private let work = WorkProfiles.Active(id: "work-1", label: "Acme", managed: true, threadPrefix: "work-1.", keyAlias: "work-1")

    private func review(_ id: String, kind: OrgReviewKind = .request, own: Bool = false) -> OrgReviewView {
        OrgReviewView(
            id: id, kind: kind, orgName: "Acme", requester: "dana@acme.com", connection: "Claude", title: "Send an email",
            lines: ["Sends an email to ext@other.com"], requires: "Security", level: "L2", zone: "external",
            digest: String(repeating: "a", count: 64), snapshot: "snap", expiresAt: 1_700_000_900, own: own
        )
    }

    func testApproveSignsTheReviewMessage() async {
        let core = FakeOrgCore()
        let r = review("or_1")
        core.reviews = [r, review("ch_1", kind: .change, own: true)]
        let keys = RecordingKeys()
        let model = OrgReviewsModel(core: core, profile: work, keys: keys, now: { 1_700_000_100 })
        await model.load()
        XCTAssertEqual(model.reviews?.map(\.id), ["or_1", "ch_1"])

        await model.answer(r, approve: true)
        // The enrolled key of this profile signed exactly the core's review message, and that signature was sent.
        XCTAssertEqual(keys.signed.count, 1)
        XCTAssertEqual(keys.signed[0].profileId, "work-1")
        XCTAssertEqual(keys.signed[0].data, orgReviewMessage(orgId: "o1", review: r, approve: true, at: 1_700_000_100))
        XCTAssertEqual(core.decisions.count, 1)
        XCTAssertEqual(core.decisions[0].id, "or_1")
        XCTAssertTrue(core.decisions[0].approve)
        XCTAssertEqual(core.decisions[0].at, 1_700_000_100)
        XCTAssertEqual(core.decisions[0].signature, Data([0x30, 0x44]).base64EncodedString())
        XCTAssertEqual(model.reviews?.map(\.id), ["ch_1"])

        // Your own change is confirmed: a different message (Propose) from an approval of someone else's.
        let own = review("ch_1", kind: .change, own: true)
        var other = own
        other.own = false
        XCTAssertNotEqual(
            orgReviewMessage(orgId: "o1", review: own, approve: true, at: 5),
            orgReviewMessage(orgId: "o1", review: other, approve: true, at: 5)
        )
    }

    func testPersonalProfileShowsNoReviews() async {
        let core = FakeOrgCore()
        core.reviews = [review("or_1")]
        let keys = RecordingKeys()
        let model = OrgReviewsModel(core: core, profile: WorkProfiles.personal, keys: keys)
        await model.load()
        XCTAssertEqual(model.reviews?.count, 0)
        // The Personal core is asked nothing, and nothing is signed.
        XCTAssertEqual(core.listed, 0)
        await model.answer(review("or_1"), approve: true)
        XCTAssertTrue(keys.signed.isEmpty)
        XCTAssertTrue(core.decisions.isEmpty)
    }

    func testAnOrgReviewPushOpensTheReviews() {
        let push = PushPayload(kind: "orgreview", id: "or_1")
        XCTAssertEqual(push?.link, .orgReviews)
        XCTAssertNil(push?.itemKind)
        XCTAssertEqual(DeepLink(url: DeepLink.orgReviews.url), .orgReviews)
    }
}
