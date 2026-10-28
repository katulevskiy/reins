import UserNotifications
import XCTest
@testable import Reins

// The email send hold (Decisions v2, 08 R10): an approved Gmail send waits out its hold ("Sending in 23 s" · Undo), a
// silent `hold` push wakes the app to send it, and one the phone woke too late for asks "Still send?".

/// The hold in words: the countdown, the buttons per state, and the setting's choices.
final class SendHoldTextTests: XCTestCase {
    private func hold(_ state: HoldState, releaseAt: Int64 = 1_000) -> ActivityHold {
        ActivityHold(id: "h1", releaseAt: releaseAt, state: state)
    }

    func testTheCountdownSaysHowManySecondsAreLeftThenSending() {
        XCTAssertEqual(HoldText.countdown(releaseAt: 1_023, now: 1_000), "Sending in 23 s")
        XCTAssertEqual(HoldText.countdown(releaseAt: 1_001, now: 1_000), "Sending in 1 s")
        XCTAssertEqual(HoldText.countdown(releaseAt: 1_000, now: 1_000), "Sending", "at 0 it goes")
        XCTAssertEqual(HoldText.countdown(releaseAt: 990, now: 1_000), "Sending", "never a negative count")
    }

    func testTheStatusLineFollowsTheHoldsState() {
        XCTAssertEqual(HoldText.status(hold(.waiting, releaseAt: 1_030), now: 1_000), "Sending in 30 s")
        XCTAssertEqual(HoldText.status(hold(.sending, releaseAt: 1_030), now: 1_000), "Sending")
        XCTAssertEqual(HoldText.status(hold(.late), now: 9_000), "Still send?")
    }

    func testEachStateOffersItsOwnButtons() {
        XCTAssertEqual(HoldText.actions(.waiting), [.undo])
        XCTAssertEqual(HoldText.actions(.sending), [], "nothing to stop once it goes")
        XCTAssertEqual(HoldText.actions(.late), [.send, .discard])
    }

    func testTheButtonsUseRealSymbols() {
        XCTAssertEqual(HoldAction.undo.title, "Undo")
        XCTAssertEqual(HoldAction.undo.symbol, "arrow.uturn.backward")
        XCTAssertEqual(HoldAction.send.title, "Send")
        XCTAssertEqual(HoldAction.send.symbol, "paperplane")
        XCTAssertEqual(HoldAction.discard.title, "Discard")
        XCTAssertEqual(HoldAction.discard.symbol, "xmark")
        XCTAssertEqual(Set(HoldAction.allCases.map(\.identifier)).count, HoldAction.allCases.count)
    }

    func testOnlyAWaitingHoldWhoseTimeHasComeIsDue() {
        XCTAssertFalse(HoldText.due(hold(.waiting, releaseAt: 1_001), now: 1_000))
        XCTAssertTrue(HoldText.due(hold(.waiting, releaseAt: 1_000), now: 1_000))
        XCTAssertTrue(HoldText.due(hold(.waiting, releaseAt: 900), now: 1_000))
        XCTAssertFalse(HoldText.due(hold(.sending, releaseAt: 900), now: 1_000), "already going")
        XCTAssertFalse(HoldText.due(hold(.late, releaseAt: 900), now: 1_000), "late asks the owner instead")
    }

    func testTheHeldSendsAreTheEntriesWithAHoldNewestFirst() {
        var older = DemoData.entry(1, at: 100, action: "send", outcome: "not_sent", detail: "")
        older.hold = hold(.late)
        var newer = DemoData.entry(2, at: 200, action: "send", outcome: "held", detail: "")
        newer.hold = hold(.waiting)
        let gone = DemoData.entry(3, at: 300, action: "send", outcome: "held", detail: "")
        let sent = DemoData.entry(4, at: 400, action: "send", outcome: "sent", detail: "")
        XCTAssertEqual(HoldText.held([older, gone, newer, sent]).map(\.id), [2, 1])
    }

    func testWhatTheHeldSendsNeedNow() {
        var due = DemoData.entry(1, at: 0, action: "send", outcome: "held", detail: "")
        due.hold = ActivityHold(id: "due", releaseAt: 1_000, state: .waiting)
        var later = DemoData.entry(2, at: 0, action: "send", outcome: "held", detail: "")
        later.hold = ActivityHold(id: "later", releaseAt: 2_000, state: .waiting)
        var going = DemoData.entry(3, at: 0, action: "send", outcome: "held", detail: "")
        going.hold = ActivityHold(id: "going", releaseAt: 900, state: .sending)
        let work = HoldWork.of([due, later, going], now: 1_000)
        XCTAssertEqual(work.due, ["due"])
        XCTAssertEqual(work.sending, ["going"])
        XCTAssertFalse(work.idle)
        XCTAssertTrue(HoldWork.of([later], now: 1_000).idle, "nothing to do until it comes due")
    }

    func testTheSettingsChoices() {
        XCTAssertEqual(HoldText.choices(30), [0, 10, 30, 60, 120])
        XCTAssertEqual(HoldText.choices(45), [0, 10, 30, 45, 60, 120], "a value set elsewhere still shows")
        XCTAssertEqual(HoldText.choiceLabel(0), "Off")
        XCTAssertEqual(HoldText.choiceLabel(10), "10 s")
        XCTAssertEqual(HoldText.choiceLabel(120), "120 s")
        XCTAssertEqual(HoldText.settingTitle, "Send hold")
    }

    func testTheLateNotificationIsAShortAsk() {
        XCTAssertEqual(HoldText.lateTitle, "Still send?")
        XCTAssertEqual(HoldText.lateBody(1), "An email was not sent in time.")
        XCTAssertEqual(HoldText.lateBody(3), "3 emails were not sent in time.")
    }
}

/// A held send's entries in the river: "held" never reads as sent, whoever approved it.
final class SendHoldOutcomeTests: XCTestCase {
    private func entry(_ outcome: String, decidedBy: String = "", hold: HoldState? = nil) -> ActivityEntry {
        var e = DemoData.entry(1, at: 0, action: "send", outcome: outcome, detail: "", decidedBy: decidedBy, autopilot: decidedBy.isEmpty ? nil : DemoData.note())
        e.hold = hold.map { ActivityHold(id: "h1", releaseAt: 10, state: $0) }
        return e
    }

    func testOutcomeWords() {
        XCTAssertEqual(HoldText.outcome("held", hold: ActivityHold(id: "h1", releaseAt: 10, state: .waiting)), "Sending")
        XCTAssertEqual(HoldText.outcome("held", hold: nil), "Held", "its send has an entry of its own by now")
        XCTAssertEqual(HoldText.outcome("undone", hold: nil), "Undone")
        XCTAssertEqual(HoldText.outcome("not_sent", hold: nil), "Not sent")
        XCTAssertNil(HoldText.outcome("sent", hold: nil))
        XCTAssertEqual(OutcomeTag.word("held"), "Sending")
        XCTAssertEqual(OutcomeTag.word("undone"), "Undone")
        XCTAssertEqual(OutcomeTag.word("not_sent"), "Not sent")
    }

    func testAHeldSendHasNotGoneWhoeverApprovedIt() {
        for decider in ["", "autopilot", "overnight", "bypass", "account_permission"] {
            for state in [HoldState.waiting, .sending] {
                let e = entry("held", decidedBy: decider, hold: state)
                XCTAssertFalse(ActivityText.done(e), "\(decider): not in the past")
                XCTAssertEqual(AskTense.of(outcome: e.outcome), .refused, "never \"Sent …\"")
                XCTAssertEqual(EntryText.mark(e), .held(sending: true))
                XCTAssertEqual(EntryOutcome.text(e), "Sending")
                XCTAssertFalse(EntryOutcome.text(e).contains("Sent"))
                XCTAssertEqual(EntryOutcome.symbol(e), "paperplane")
                XCTAssertEqual(EntryRowText.outcome(e), .held(word: "Sending"))
            }
        }
        let gone = entry("held", decidedBy: "autopilot")
        XCTAssertEqual(EntryText.mark(gone), .held(sending: false))
        XCTAssertEqual(EntryOutcome.text(gone), "Held")
        XCTAssertEqual(EntryRowText.outcome(gone), .held(word: "Held"))
    }

    func testUndoneAndNotSentNeverClaimTheSend() {
        let undone = entry("undone", decidedBy: "autopilot")
        XCTAssertFalse(ActivityText.done(undone))
        XCTAssertEqual(EntryText.mark(undone), .undone)
        XCTAssertEqual(EntryOutcome.text(undone), "Undone")
        XCTAssertEqual(EntryOutcome.symbol(undone), "arrow.uturn.backward")
        XCTAssertEqual(EntryRowText.outcome(undone), .undone)
        XCTAssertEqual(ActivityDays.stamp(undone).outcome, .denied, "filed as the core files it")

        let late = entry("not_sent", hold: .late)
        XCTAssertFalse(ActivityText.done(late))
        XCTAssertEqual(EntryText.mark(late), .notSent)
        XCTAssertEqual(EntryOutcome.text(late), "Not sent")
        XCTAssertEqual(EntryRowText.outcome(late), .notSent)
        XCTAssertEqual(ActivityDays.stamp(late).outcome, .failed)
        XCTAssertEqual(EntryMark.spoken(.notSent), "Not sent")
        XCTAssertEqual(Outcome.notSent.spoken, "Not sent")
    }

    func testTheReleasesOwnEntryStillReadsSent() {
        let sent = entry("sent")
        XCTAssertTrue(ActivityText.done(sent))
        XCTAssertEqual(EntryOutcome.text(sent), "Sent")
    }
}

/// A `hold` push only wakes the app: it is accepted, silent, opens nothing, and the extension shows nothing for it.
final class SendHoldPushTests: XCTestCase {
    func testAHoldPushIsAcceptedAndSilent() throws {
        let push = try XCTUnwrap(PushPayload(userInfo: ["t": "hold", "id": "a1b2-c3"]))
        XCTAssertEqual(push.kind, PushPayload.hold)
        XCTAssertTrue(push.silent)
        XCTAssertNil(push.itemKind, "nothing to fetch and park")
        XCTAssertNil(push.circle)
        XCTAssertNil(push.link, "no notification to tap")
    }

    func testAMalformedHoldPushIsDropped() {
        XCTAssertNil(PushPayload(kind: "hold", id: "../x"))
        XCTAssertNil(PushPayload(kind: "hold", id: ""))
        XCTAssertNil(PushPayload(kind: "hold", id: nil))
    }

    func testOtherPushesAreNotSilentHolds() {
        for (kind, id) in [("req", "r1"), ("pair", "p1"), ("blob", "b1"), ("replaced", ""), ("circle", "news:n1")] {
            XCTAssertEqual(PushPayload(kind: kind, id: id)?.silent, false, kind)
        }
    }

    func testTheExtensionShowsNothingForIt() {
        let c = NotificationContent.nothing()
        XCTAssertTrue(c.title.isEmpty)
        XCTAssertTrue(c.subtitle.isEmpty)
        XCTAssertTrue(c.body.isEmpty)
        XCTAssertNil(c.sound)
        XCTAssertEqual(c.interruptionLevel, .passive)
    }

    func testAFailedActionSaysWhy() {
        XCTAssertEqual(SendHolds.message(.undo, CoreError.NotFound), "Too late to undo: it is being sent.")
        XCTAssertEqual(SendHolds.message(.send, CoreError.NotFound), "That email is no longer waiting.")
    }
}

/// The hold against the `-demo` core: the setting, the release, Undo, and "Still send?".
@MainActor
final class SendHoldFlowTests: XCTestCase {
    private final class Owner: Authenticating {
        var asked: [String] = []
        func confirm(_ reason: String) async -> Bool {
            asked.append(reason)
            return true
        }
    }

    private var owner: Owner!
    private var core: DemoReinsCore!
    private var model: AppModel!

    private func start() async {
        owner = Owner()
        core = DemoReinsCore(signedIn: true, syncCap: 0.3, holds: true)
        model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshPending()
    }

    func testTheHoldIsASettingWithoutTheOwnersCheck() async throws {
        await start()
        let ap = AutopilotModel(app: model)
        await ap.refresh()
        XCTAssertEqual(ap.sendHoldSecs, 30, "30 s by default")
        await ap.setSendHold(0)
        XCTAssertEqual(ap.sendHoldSecs, 0, "off")
        let read = try await core.sendHoldSecs()
        XCTAssertEqual(read, 0)
        await ap.setSendHold(120)
        XCTAssertEqual(ap.sendHoldSecs, 120)
        // Past 120 s the core refuses, and what it holds shows again.
        await ap.setSendHold(300)
        XCTAssertEqual(ap.sendHoldSecs, 120)
        XCTAssertNotNil(ap.error)
        XCTAssertTrue(owner.asked.isEmpty, "the level of a send is the same either way: no owner check")
    }

    func testTheDemoShowsAWaitingAndALateSend() async {
        await start()
        let held = HoldText.held(model.activity)
        XCTAssertEqual(held.map { $0.hold?.state }, [.waiting, .late])
        XCTAssertEqual(held.first?.outcome, "held")
        XCTAssertEqual(held.last?.outcome, "not_sent")
    }

    func testUndoStopsAWaitingSendWithoutTheOwnersCheck() async throws {
        await start()
        let waiting = try XCTUnwrap(HoldText.held(model.activity).first { $0.hold?.state == .waiting })
        let message = await SendHolds.perform(.undo, holdId: waiting.hold!.id, model: model)
        XCTAssertNil(message)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertFalse(model.activity.contains { $0.hold?.id == "hold-1" }, "nothing left to undo")
        XCTAssertEqual(model.activity.first?.outcome, "undone")
        let again = await SendHolds.perform(.undo, holdId: "hold-1", model: model)
        XCTAssertNotNil(again, "already undone")
    }

    func testStillSendSendsAfterTheOwnersCheck() async throws {
        await start()
        let message = await SendHolds.perform(.send, holdId: "hold-2", model: model)
        XCTAssertNil(message)
        XCTAssertEqual(owner.asked, ["Send the email"], "it goes as the owner's approval")
        XCTAssertFalse(model.activity.contains { $0.hold?.id == "hold-2" })
        XCTAssertEqual(model.activity.first?.outcome, "sent")
    }

    func testDiscardDropsAStillSendWithoutTheOwnersCheck() async {
        await start()
        let message = await SendHolds.perform(.discard, holdId: "hold-2", model: model)
        XCTAssertNil(message)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertFalse(model.activity.contains { $0.hold?.id == "hold-2" })
        XCTAssertFalse(model.activity.contains { $0.outcome == "sent" && $0.id > 1_101 }, "never sent")
    }

    func testReleaseSendsOnlyWhatIsDue() async throws {
        await start()
        let early = try await SendHolds.release(model, notifier: nil)
        XCTAssertEqual(early.sent, 0, "the countdown has 25 s left")
        XCTAssertTrue(model.activity.contains { $0.hold?.id == "hold-1" })
    }
}
