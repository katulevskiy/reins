import XCTest
@testable import Reins

/// Activity in Next: what the Mode sheet's rows do, who asks on a waiting card (and when one tap may approve), how a
/// row writes who did what, and the day's counts.
final class ActivityNextTests: XCTestCase {
    private func settings(_ mode: AutopilotMode, bypassUntil: Int64? = nil, overnightUntil: Int64? = nil) -> AutopilotSettings {
        AutopilotSettings(
            mode: mode, baseMode: .manual, bypassUntil: bypassUntil, bypassSince: nil, overnightUntil: overnightUntil, overnightSince: nil,
            defaultProfileId: "personal", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 0, downloadedBytes: 0, error: nil, runtimeReady: true, risk: true, update: false),
            connections: []
        )
    }

    private func item(_ label: String, kind: PendingKind = .request) -> PendingItem {
        PendingItem(
            kind: kind, id: "r1", title: "", subtitle: "", createdAt: 0, connectionId: "c1", connectionLabel: label,
            action: "push", count: 1, service: "github", account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil, label: nil
        )
    }

    // MARK: The Mode sheet

    func testTheModeSheetDoesWhatTheChipsMenuDid() {
        let manual = settings(.manual)
        XCTAssertEqual(ModeSheetAction.of(.manual, manual), .none, "the mode in force")
        XCTAssertEqual(ModeSheetAction.of(.auto, manual), .set(.auto))
        XCTAssertEqual(ModeSheetAction.of(.lockdown, manual), .set(.lockdown), "locking down is a plain tap")
        XCTAssertEqual(ModeSheetAction.of(.overnight, manual), .askOvernight, "its own held sheet")
        XCTAssertEqual(ModeSheetAction.of(.bypass, manual), .askBypass, "its own held sheet, then the owner's check")
    }

    func testRunningModesStopFromTheirOwnRow() {
        let now = Int64(Date().timeIntervalSince1970)
        XCTAssertEqual(ModeSheetAction.of(.bypass, settings(.bypass, bypassUntil: now + 600)), .stopBypass)
        XCTAssertEqual(ModeSheetAction.of(.overnight, settings(.overnight, overnightUntil: now + 600)), .stopOvernight)
        XCTAssertEqual(ModeSheetAction.of(.lockdown, settings(.lockdown)), .endLockdown, "ending a lockdown asks the owner")
        // Leaving a lockdown for Manual or Auto goes through AutopilotModel.setMode, which asks the owner too.
        XCTAssertEqual(ModeSheetAction.of(.manual, settings(.lockdown)), .set(.manual))
    }

    func testTheModeSheetKeepsTheMenusIdentifiersAndLengths() {
        XCTAssertEqual(ModeKind.allCases.map(ModeSheetAction.identifier), ["quick:MANUAL", "quick:AUTO", "quick:OVERNIGHT", "quick:LOCKDOWN", "quick:BYPASS"])
        XCTAssertEqual(ModeSheetAction.length(.bypass, settings(.manual)), "1h")
        XCTAssertEqual(ModeSheetAction.length(.overnight, settings(.manual)), "8h")
        XCTAssertNil(ModeSheetAction.length(.auto, settings(.manual)))
        XCTAssertNil(ModeSheetAction.length(.bypass, settings(.bypass, bypassUntil: 1)), "running: the check shows instead")
    }

    // MARK: Who asks on a waiting card (security review, rule 4)

    func testAWaitingCardNamesWhoAsksQuotedOnOneShortLine() {
        XCTAssertEqual(WaitCardIdentity.who(item("Claude Code")), "“Claude Code”")
        let long = WaitCardIdentity.who(item(String(repeating: "Reins approved this ", count: 5)))!
        XCTAssertLessThanOrEqual(long.count, WaitCardIdentity.limit + 3, "capped (the quotes and an ellipsis aside)")
        let multiline = WaitCardIdentity.who(item("Claude\nApproved by Reins"))!
        XCTAssertFalse(multiline.contains("\n"), "one line")
        XCTAssertNil(WaitCardIdentity.who(item("Claude", kind: .pairing)), "a pairing quotes its name in its mono line instead")
    }

    func testOneTapAllowOnlyWhileWhoAsksShows() {
        XCTAssertEqual(WaitCardIdentity.pick(item("Claude"), .allow(97)), .allow(97))
        XCTAssertNil(WaitCardIdentity.pick(item(""), .allow(97)), "no name to show: Review only, no one-tap Allow and no swipe to allow")
        XCTAssertNil(WaitCardIdentity.pick(item("\u{200B}\u{202E}"), .allow(97)), "a name that cleans to nothing counts as none")
        XCTAssertEqual(WaitCardIdentity.pick(item(""), .deny(91)), .deny(91), "denying needs no identity")
        XCTAssertEqual(WaitCardIdentity.pick(item(""), .unsure), .unsure)
    }

    // MARK: Rows

    func testARowSaysWhoDidItApartFromWhatWasDone() {
        let at: Int64 = 1_700_000_000
        let done = DemoData.entry(1, at: at, action: "read", outcome: "released", detail: "")
        let who = EntryRowText.who(done)
        XCTAssertNotNil(who)
        let runs = EntryRowText.runs(done, who: who)
        XCTAssertFalse(runs.map(\.text).joined().contains(who!), "the name is a run of its own, never inside Reins' sentence")
        XCTAssertTrue(runs.first!.text.first!.isLowercase, "the sentence goes on after the name")
        let denied = DemoData.entry(2, at: at, action: "read", outcome: "denied", detail: "")
        XCTAssertTrue(EntryRowText.runs(denied, who: "Claude").first!.text.hasPrefix("tried to "))
        XCTAssertTrue(EntryRowText.runs(denied, who: nil).first!.text.hasPrefix("Tried to "))
    }

    func testARowsOutcomeTagNeverClaimsAutopilotForAPermission() {
        let at: Int64 = 1_700_000_000
        XCTAssertEqual(EntryRowText.outcome(DemoData.entry(1, at: at, action: "read", outcome: "released", detail: "")), .approved)
        XCTAssertEqual(EntryRowText.outcome(DemoData.entry(2, at: at, action: "read", outcome: "denied", detail: "", decidedBy: "autopilot")), .autoDenied)
        XCTAssertEqual(EntryRowText.outcome(DemoData.entry(3, at: at, action: "send", outcome: "sent", detail: "", decidedBy: "account_permission")), .grant)
        XCTAssertEqual(EntryRowText.outcome(DemoData.entry(4, at: at, action: "send", outcome: "sent", detail: "", decidedBy: "bypass")), .bypass)
        if case .attention = EntryRowText.outcome(DemoData.entry(5, at: at, action: "send", outcome: "unknown", detail: "")) {} else {
            XCTFail("an unknown outcome keeps its warning")
        }
    }

    func testTheRowsTimeIsShortToday() {
        let now: Int64 = 1_700_000_000
        XCTAssertEqual(EntryRowText.time(now - 30, now: now), "now")
        XCTAssertEqual(EntryRowText.time(now - 300, now: now), "5m")
    }

    func testTheDaysCountsSplitAutopilotFromYours() {
        let stamps = [
            ActivityStamp(id: 1, at: 1_700_000_000, outcome: .approved, decider: .you),
            ActivityStamp(id: 2, at: 1_700_000_010, outcome: .approved, decider: .autopilot),
            ActivityStamp(id: 3, at: 1_700_000_020, outcome: .denied, decider: .autopilot),
        ]
        let day = ActivityDays.group(stamps, calendar: .current)[0]
        XCTAssertEqual(day.approved, 1)
        XCTAssertEqual(day.auto, 1)
        XCTAssertEqual(day.denied, 1, "a denial counts as one, whoever denied it")
    }
}
