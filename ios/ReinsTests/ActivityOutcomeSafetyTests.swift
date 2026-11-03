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
                XCTAssertEqual(EntryOutcome.symbol(entry), status == "started" ? "ellipsis" : "exclamationmark.triangle")
                XCTAssertEqual(EntryOutcome.whyTitle(entry), "What Autopilot suggested")
                if status == "started" {
                    XCTAssertEqual(EntryOutcome.text(entry), "In progress")
                } else {
                    XCTAssertTrue(EntryOutcome.text(entry).contains("do not repeat"))
                }
            }
        }
    }
    func testAFoldedExecutionTellsOnItsDecisionRowOnlyWhenSomethingIsWrong() {
        func decided(_ state: ExecutionState) -> ActivityEntry {
            DemoData.entry(
                1, at: 0, action: "write", outcome: "sent", detail: "", service: "desktop", op: "ssh", decidedBy: "bypass",
                execution: ActivityExecution(state: state, records: [0], startedAt: 0)
            )
        }
        XCTAssertEqual(EntryText.mark(decided(.done)), .bypass, "fine: no extra tag")
        XCTAssertEqual(EntryOutcome.text(decided(.done)), "Allowed during a bypass")
        XCTAssertTrue(ActivityText.done(decided(.done)))
        XCTAssertEqual(ActivityDays.stamp(decided(.done)).outcome, .approved)
        let unknown = decided(.unknown)
        XCTAssertEqual(EntryText.mark(unknown), .unknown)
        XCTAssertEqual(EntryOutcome.text(unknown), "Outcome unknown · do not repeat")
        XCTAssertEqual(ActivityDays.stamp(unknown).outcome, .failed)
        XCTAssertFalse(ActivityText.done(unknown), "never says it ran")
        XCTAssertEqual(EntryText.mark(decided(.withheld)), .withheld)
        XCTAssertEqual(EntryText.mark(decided(.pending)), .bypass)
        let running = DemoData.entry(
            2, at: 0, action: "execute", outcome: "started", detail: "",
            execution: ActivityExecution(state: .pending, records: [2], startedAt: 0)
        )
        XCTAssertEqual(EntryRowText.outcome(running), .pending)
        XCTAssertEqual(EntryMark.spoken(.pending), "In progress")
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
