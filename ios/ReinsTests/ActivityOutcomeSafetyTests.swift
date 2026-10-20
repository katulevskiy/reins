import XCTest
@testable import Reins

final class ActivityOutcomeSafetyTests: XCTestCase {
    func testUncertainAndWithheldResultsOverrideEveryAuthorizationMarker() {
        for status in ["started", "unknown", "succeeded_withheld"] {
            for decider in ["", "autopilot", "overnight", "bypass", "lockdown", "account_permission", "temporary_permission", "future_actor"] {
                let entry = DemoData.entry(1, at: 0, action: "send", outcome: status, detail: "", grantId: "grant", decidedBy: decider)
                XCTAssertFalse(ActivityText.done(entry))
                XCTAssertEqual(AskTense.of(outcome: status), .refused)
                XCTAssertEqual(ActivityDays.stamp(entry).outcome, .failed)
                XCTAssertFalse(EntryOutcome.text(entry).contains("Allowed"))
                XCTAssertFalse([EntryText.Mark.approved, .auto, .grant, .permission].contains(EntryText.mark(entry)))
                XCTAssertEqual(EntryOutcome.symbol(entry), "exclamationmark.triangle")
                XCTAssertEqual(EntryOutcome.whyTitle(entry), "What Autopilot suggested")
                if status != "started" { XCTAssertTrue(EntryOutcome.text(entry).contains("do not repeat")) }
            }
        }
    }
    func testExplicitPermissionsHaveTheirOwnAttributionWithoutLearnedConfidence() {
        for (actor, name) in [("account_permission", "account permission"), ("temporary_permission", "temporary permission")] {
            let entry = DemoData.entry(1, at: 0, action: "send", outcome: "sent", detail: "", decidedBy: actor, autopilot: DemoData.note())
            XCTAssertEqual(EntryText.mark(entry), .permission)
            XCTAssertEqual(EntryOutcome.text(entry), "Allowed by \(name)")
            XCTAssertEqual(EntryOutcome.symbol(entry), "key.horizontal")
            XCTAssertEqual(ActivityDays.stamp(entry).decider, .autopilot)
            XCTAssertFalse(EntryOutcome.text(entry).contains("Autopilot"))
            XCTAssertFalse(EntryOutcome.text(entry).contains("%"))
            let denied = DemoData.entry(1, at: 0, action: "send", outcome: "denied", detail: "", decidedBy: actor)
            XCTAssertEqual(EntryOutcome.text(denied), "Denied · \(name)")
            XCTAssertEqual(EntryText.mark(denied), .denied)
        }
        XCTAssertEqual(EntryMark.spoken(.permission), "Allowed by an explicit permission")
        XCTAssertEqual(ActivitySearchText.decider(.autopilot), "Automatic")
        XCTAssertEqual(ActivitySearchText.id(.autopilot), "filter:autopilot")
    }
    func testUnknownActorsDoNotClaimOwnerApprovalOrLearnedAutopilot() {
        for (status, text) in [("sent", "Allowed"), ("denied", "Denied"), ("granted", "Access given")] {
            let entry = DemoData.entry(1, at: 0, action: "send", outcome: status, detail: "", decidedBy: "future_actor", autopilot: DemoData.note())
            XCTAssertEqual(EntryOutcome.text(entry), text)
            XCTAssertEqual(ActivityDays.stamp(entry).decider, .autopilot)
        }
        let owner = DemoData.entry(1, at: 0, action: "read", outcome: "released", detail: "")
        XCTAssertEqual(EntryOutcome.text(owner), "Allowed by you")
        let learned = DemoData.entry(1, at: 0, action: "read", outcome: "released", detail: "", decidedBy: "autopilot", autopilot: DemoData.note())
        XCTAssertEqual(EntryOutcome.text(learned), "Allowed by Autopilot · 98%")
    }
    func testKnownCompletedOutcomesKeepTheirExistingTense() {
        for status in ["sent", "released", "granted"] {
            let entry = DemoData.entry(1, at: 0, action: "read", outcome: status, detail: "")
            XCTAssertTrue(ActivityText.done(entry))
            XCTAssertEqual(AskTense.of(outcome: status), .done)
        }
        XCTAssertEqual(OutcomeTag.word("unknown"), "Outcome unknown")
        XCTAssertEqual(OutcomeTag.word("succeeded_withheld"), "Completed; response withheld")
        XCTAssertEqual(ActivitySearchText.outcome(.failed), "Needs attention")
    }
}
