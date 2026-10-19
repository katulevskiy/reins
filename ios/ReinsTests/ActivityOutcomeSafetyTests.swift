import XCTest
@testable import Reins

final class ActivityOutcomeSafetyTests: XCTestCase {
    func testUncertainAndWithheldResultsOverrideEveryAuthorizationMarker() {
        for status in ["started", "unknown", "succeeded_withheld"] {
            for decider in ["", "autopilot", "overnight", "bypass", "lockdown"] {
                let entry = DemoData.entry(1, at: 0, action: "send", outcome: status, detail: "", grantId: "grant", decidedBy: decider)
                XCTAssertFalse(ActivityText.done(entry))
                XCTAssertEqual(AskTense.of(outcome: status), .refused)
                XCTAssertEqual(ActivityDays.stamp(entry).outcome, .failed)
                XCTAssertFalse(EntryOutcome.text(entry).contains("Allowed"))
                XCTAssertFalse([EntryText.Mark.approved, .auto, .grant].contains(EntryText.mark(entry)))
                XCTAssertEqual(EntryOutcome.symbol(entry), "exclamationmark.triangle")
                XCTAssertEqual(EntryOutcome.whyTitle(entry), "What Autopilot suggested")
                if status != "started" { XCTAssertTrue(EntryOutcome.text(entry).contains("do not repeat")) }
            }
        }
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
