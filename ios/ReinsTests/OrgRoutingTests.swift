@testable import Reins
import XCTest

/// A work profile's core for routing and web passkeys: what waits, and the confirmations it was sent.
private final class RoutingCore: OrgReviewsCore, @unchecked Sendable {
    var org: OrgView? = OrgView(orgId: "o1", name: "Acme", tier: "business", roles: ["Engineer"], enrolled: true, githubOrgs: [])
    var reviews: [OrgReviewView] = []
    var passkeys: [OrgPasskeyView] = []
    var confirmations: [(id: String, approve: Bool, at: Int64, signature: String)] = []

    func orgMembership() async throws -> OrgView? { org }
    func orgReviews() async throws -> [OrgReviewView] { reviews }
    func orgDecide(id: String, approve: Bool, at: Int64, signature: String) async throws {}
    func orgPasskeys() async throws -> [OrgPasskeyView] { passkeys }
    func orgConfirmPasskey(id: String, approve: Bool, at: Int64, signature: String) async throws {
        confirmations.append((id, approve, at, signature))
    }
}

/// A device key that records what it signed.
private final class RoutingKeys: DeviceKeys, @unchecked Sendable {
    var signed: [(profileId: String, data: Data)] = []
    func create(profileId: String) throws -> (publicKey: Data, hardware: Bool) { (Data([0x30, 0x59]), true) }
    func sign(profileId: String, data: Data) throws -> Data {
        signed.append((profileId, data))
        return Data([0x30, 0x44])
    }
    func attest(clientDataHash: Data) async -> (keyId: String, attestation: Data)? { nil }
}

/// Org routing on the review cards (O3): who else must decide, how far it got and where an escalation took it; and a
/// web passkey the member added in the browser, confirmed on this phone with the key enrolled to the org.
@MainActor
final class OrgRoutingTests: XCTestCase {
    private let work = WorkProfiles.Active(id: "work-1", label: "Acme", managed: true, threadPrefix: "work-1.", keyAlias: "work-1")
    private let keyHash = "ab12cd34" + String(repeating: "0", count: 56)

    private func review(
        _ id: String, needs: [String] = [], progress: String = "", step: String? = nil, urgent: Bool = false, reason: String? = nil
    ) -> OrgReviewView {
        OrgReviewView(
            id: id, kind: .request, orgName: "Acme", requester: "dana@acme.com", connection: "Claude", title: "Write to GitHub",
            lines: ["Writes to acme/prod-db"], requires: "On-call", level: "L2", zone: "prod",
            digest: String(repeating: "a", count: 64), snapshot: "snap", expiresAt: 1_700_000_900, own: false,
            needs: needs, progress: progress, step: step, urgent: urgent, reason: reason
        )
    }

    func testNeedsLine() {
        XCTAssertEqual(OrgRouting.needsLine(review("or_1", needs: ["On-call: payments"], progress: "1 of 2")), "Needs On-call: payments · 1 of 2")
        XCTAssertEqual(
            OrgRouting.needsLine(review("or_2", needs: ["On-call: payments", "2 of\nFinance", "Code owners"])),
            "Needs On-call: payments, 2 of Finance, Code owners"
        )
        XCTAssertEqual(OrgRouting.needsLine(review("or_3", progress: "1 of 2")), "1 of 2")
        // One approval decides: no line.
        XCTAssertNil(OrgRouting.needsLine(review("or_4")))
        XCTAssertNil(OrgRouting.needsLine(review("or_5", needs: [" "])))
        XCTAssertEqual(OrgRouting.reason(review("or_6", reason: "Rotate the\nleaked key")), "Rotate the leaked key")
        XCTAssertNil(OrgRouting.reason(review("or_7", reason: "")))
    }

    func testEscalatedLine() {
        let escalated = review("or_1", needs: ["Security admin"], step: "Escalated to Security admin", urgent: true)
        XCTAssertEqual(OrgRouting.stepLine(escalated), "Escalated to Security admin")
        XCTAssertTrue(escalated.urgent)
        XCTAssertNil(OrgRouting.stepLine(review("or_2")))
        XCTAssertNil(OrgRouting.stepLine(review("or_3", step: "  ")))
    }

    func testPasskeyConfirm() async {
        let core = RoutingCore()
        let mine = OrgPasskeyView(credentialId: "cred_A-1", createdAt: 1_700_000_000, confirmed: false, spkiHash: keyHash, fingerprint: "ab12-cd34")
        let other = OrgPasskeyView(
            credentialId: "cred_B", createdAt: 1_700_000_001, confirmed: false, spkiHash: String(repeating: "f", count: 64), fingerprint: "ffff-ffff"
        )
        let done = OrgPasskeyView(credentialId: "cred_C", createdAt: 1, confirmed: true, spkiHash: keyHash, fingerprint: "ab12-cd34")
        core.passkeys = [mine, other, done]
        let keys = RoutingKeys()
        let model = OrgReviewsModel(core: core, profile: work, keys: keys, now: { 1_700_000_050 })
        await model.load()
        // Only the ones that wait for this phone.
        XCTAssertEqual(model.passkeys.map(\.credentialId), ["cred_A-1", "cred_B"])

        await model.confirmPasskey(mine, approve: true)
        // The enrolled key of this profile signed exactly the core's message over the key hash and registration time.
        XCTAssertEqual(keys.signed.count, 1)
        XCTAssertEqual(keys.signed[0].profileId, "work-1")
        XCTAssertEqual(
            keys.signed[0].data,
            orgPasskeyConfirmMessage(orgId: "o1", credentialId: "cred_A-1", spkiHash: keyHash, registeredAt: 1_700_000_000, at: 1_700_000_050)
        )
        XCTAssertEqual(core.confirmations.count, 1)
        XCTAssertEqual(core.confirmations[0].id, "cred_A-1")
        XCTAssertTrue(core.confirmations[0].approve)
        XCTAssertEqual(core.confirmations[0].at, 1_700_000_050)
        XCTAssertEqual(core.confirmations[0].signature, Data([0x30, 0x44]).base64EncodedString())
        XCTAssertEqual(model.passkeys.map(\.credentialId), ["cred_B"])

        // Not me signs the same message for the other key, and sends a refusal.
        await model.confirmPasskey(other, approve: false)
        XCTAssertEqual(core.confirmations.last?.id, "cred_B")
        XCTAssertEqual(core.confirmations.last?.approve, false)
        XCTAssertTrue(model.passkeys.isEmpty)

        // The Personal profile signs nothing.
        let personal = OrgReviewsModel(core: core, profile: WorkProfiles.personal, keys: keys)
        await personal.confirmPasskey(mine, approve: true)
        XCTAssertEqual(keys.signed.count, 2)

        // Its push opens the reviews, with a credential id longer than other ids.
        let push = PushPayload(kind: "orgpasskey", id: String(repeating: "A", count: 342))
        XCTAssertEqual(push?.link, .orgReviews)
        XCTAssertNil(PushPayload(kind: "orgreview", id: String(repeating: "A", count: 342)))
    }
}
