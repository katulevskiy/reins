import XCTest
@testable import Reins

/// What the grant screens do through the model, against the demo core (the Android AppFlowTest grant cases).
@MainActor
final class GrantFlowTests: XCTestCase {
    private final class CountingAuthenticator: Authenticating {
        var answer = true
        var prompts = 0
        func confirm(_ reason: String) async -> Bool {
            prompts += 1
            return answer
        }
    }

    private final class RecordingFeedback: Feedback {
        var cues: [Cue] = []
        func haptic(_ haptic: Haptic) {}
        func cue(_ cue: Cue, step: Int) { cues.append(cue) }
        func cueUnlessRecent(_ cue: Cue) {}
    }

    private let auth = CountingAuthenticator()
    private let feedback = RecordingFeedback()

    private func model() async -> AppModel {
        let core = DemoReinsCore(signedIn: true, modelInstalled: true, syncCap: 0.3)
        let m = AppModel(core: core, feedback: feedback, authenticator: auth, demo: true)
        m.autoPopup = false
        await m.refreshPending()
        await m.refreshConnections()
        return m
    }

    func testAnEndedGrantResumesForTheChosenPeriodAfterAuthentication() async {
        let m = await model()
        XCTAssertEqual(m.grants.first { $0.id == "g3" }?.active, false)
        let error = await m.resumeGrant("g3", seconds: 604_800, standing: nil)
        XCTAssertNil(error)
        XCTAssertEqual(auth.prompts, 1, "resuming needs the same authentication as approving")
        let g = m.grants.first { $0.id == "g3" }
        XCTAssertEqual(g?.active, true)
        XCTAssertEqual((g?.expiresAt ?? 0) - (g?.createdAt ?? 0), 604_800)
        XCTAssertEqual(feedback.cues, [.done])
    }

    func testACancelledPromptResumesNothingAndSaysNothing() async {
        let m = await model()
        auth.answer = false
        let error = await m.resumeGrant("g3", seconds: 3_600, standing: nil)
        XCTAssertNil(error)
        XCTAssertEqual(m.grants.first { $0.id == "g3" }?.active, false)
        XCTAssertTrue(feedback.cues.isEmpty)
    }

    func testAnEditedResumeCarriesTheNewSendersAndUses() async throws {
        let m = await model()
        let g = try XCTUnwrap(m.grants.first { $0.id == "g3" })
        var draft = initialResumeDraft(g)
        draft.partiesText = "@bank.com, alerts@other.com"
        draft.limitUses = true
        draft.uses = "4"
        draft.period = .week
        guard case let .ok(seconds, standing) = buildResume(g, draft) else { return XCTFail() }
        XCTAssertNotNil(standing)
        let error = await m.resumeGrant("g3", seconds: seconds, standing: standing)
        XCTAssertNil(error)
        let after = m.grants.first { $0.id == "g3" }
        XCTAssertEqual(after?.maxUses, 4)
        XCTAssertEqual(after?.editableScope?.senderAddresses, ["alerts@other.com"])
    }

    func testARunningGrantIsRevokedAndAnEndedOneDeletedForGood() async {
        let m = await model()
        let revoked = await m.revokeGrant("g1")
        XCTAssertNil(revoked)
        XCTAssertEqual(m.grants.first { $0.id == "g1" }?.state, "revoked")
        let deleted = await m.deleteGrant("g1")
        XCTAssertNil(deleted)
        XCTAssertFalse(m.grants.contains { $0.id == "g1" })
        XCTAssertEqual(feedback.cues, [.delete, .delete])
        XCTAssertEqual(auth.prompts, 0, "ending access never needs authentication")
    }

    func testAGrantIsCreatedInAdvanceAfterAuthentication() async {
        let m = await model()
        let before = m.grants.count
        let form = NewGrantModel()
        form.edit {
            $0.connectionId = "c1"
            $0.account = "me@gmail.com"
            $0.partiesText = "alerts@bank.com, @statements.bank.com"
            $0.lifetime = .oneTime
        }
        await form.create(m)
        XCTAssertTrue(form.finished)
        XCTAssertNil(form.error)
        XCTAssertEqual(auth.prompts, 1)
        XCTAssertEqual(m.grants.count, before + 1)
    }

    func testAnIncompleteNewGrantExplainsWhatIsMissingAndAsksNothing() async {
        let m = await model()
        let form = NewGrantModel()
        form.edit { $0.connectionId = "c1"; $0.account = "me@gmail.com" }
        await form.create(m)
        XCTAssertFalse(form.finished)
        XCTAssertTrue(form.error?.hasPrefix("Enter which senders") == true)
        XCTAssertEqual(auth.prompts, 0)
        XCTAssertEqual(feedback.cues, [.error])
    }
}
