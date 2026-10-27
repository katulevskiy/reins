import XCTest
@testable import Reins

/// Joining as a second phone: the joining phone's steps from what each poll of the core says, when the approving
/// phone may approve, and which phones the approving phone lists as removable.
@MainActor
final class ViewerEnrollmentTests: XCTestCase {
    func testPollsMoveFromWaitingToTheCodeToTheEnd() {
        var step = ViewerJoinStep.waiting
        step = step.after(.waiting)
        XCTAssertEqual(step, .waiting)
        step = step.after(.compare(code: "482193"))
        XCTAssertEqual(step, .compare("482 193"))
        XCTAssertTrue(step.polling)
        step = step.after(.compare(code: "482 193"))
        XCTAssertEqual(step, .compare("482 193"))
        step = step.after(.enrolled)
        XCTAssertEqual(step, .enrolled)
        XCTAssertFalse(step.polling)
        XCTAssertFalse(step.ended, "an enrollment is not asked again")
    }

    func testACodeShownIsNotTakenBackByALateWaiting() {
        XCTAssertEqual(ViewerJoinStep.compare("482 193").after(.waiting), .compare("482 193"))
    }

    func testDenialAndExpiryEndAndMayBeAskedAgain() {
        for (progress, end) in [(EnrollProgress.denied, ViewerJoinStep.denied), (.expired, .expired)] {
            let step = ViewerJoinStep.compare("482 193").after(progress)
            XCTAssertEqual(step, end)
            XCTAssertTrue(step.ended)
            XCTAssertFalse(step.polling)
        }
    }

    func testALostConnectionKeepsWaitingAndAGoneRequestHasRunOut() {
        let compare = ViewerJoinStep.compare("482 193")
        XCTAssertEqual(compare.after(failure: CoreError.Network(reason: "offline")), compare)
        XCTAssertEqual(compare.after(failure: CoreError.NotFound), .expired)
        let failed = ViewerJoinStep.waiting.after(failure: CoreError.Invalid(reason: "The challenge changed."))
        XCTAssertEqual(failed, .failed("The challenge changed."))
        XCTAssertTrue(failed.ended)
    }

    func testTheApprovingPhoneApprovesOnlyOnceBothShowTheCode() {
        let waiting = EnrollRequestModel(view: EnrollView(id: "j1", deviceName: "Pixel 9", code: nil, proven: false, createdAt: 1))
        XCTAssertFalse(waiting.canApprove)
        let shown = EnrollRequestModel(view: EnrollView(id: "j1", deviceName: "Pixel 9", code: "482 193", proven: false, createdAt: 1))
        XCTAssertTrue(shown.canApprove)
    }

    func testOnlyOtherPhonesAreListedAsSecondPhones() {
        let roster = RosterSummary(
            epoch: 3, role: .primary,
            members: [
                RosterMemberView(deviceId: "me", name: "Pixel 9", primary: true, thisPhone: true),
                RosterMemberView(deviceId: "iphone", name: "iPhone", primary: false, thisPhone: false),
            ],
            ownerFingerprint: "a1b2 c3d4", warnings: []
        )
        XCTAssertEqual(SecondPhonesModel.viewers(of: roster).map(\.deviceId), ["iphone"])
    }
}
