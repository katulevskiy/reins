import UserNotifications
import XCTest
@testable import Reins

/// Only what waits for the owner is an ask: a bypass, Lockdown, Overnight or Autopilot decides first, and what it decides
/// never pops up, never stays actionable in a notification and closes if it was open (the Android app's `AskGateTest`).
@MainActor
final class AskGateTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func item(_ id: String, _ kind: PendingKind = .request) -> PendingItem {
        PendingItem(
            kind: kind, id: id, title: "Send email", subtitle: "me@gmail.com", createdAt: now, connectionId: "c1",
            connectionLabel: "Claude", action: "send", count: 1, service: "gmail", account: "me@gmail.com", waitUntil: nil,
            op: "", opTitle: "", suggestion: nil
        )
    }

    private func decision(_ id: String, by: String = "bypass") -> AutoDecisionView {
        AutoDecisionView(
            requestId: id, kind: .request, connectionId: "c1", connectionLabel: "Claude", title: "Send email",
            verdict: .approve, decidedBy: by, pApprove: 0, confidence: 0, activityId: 7
        )
    }

    // MARK: The gate

    func testWhatAModeDecidedNeverPopsUpEvenFromAListReadBeforeTheDecision() {
        let read = [item("r1"), item("r2")]
        let decided = AskGate.remember("r1", in: [])
        XCTAssertEqual(AskGate.waiting(read, decided: decided).map(\.id), ["r2"])
        XCTAssertEqual(AskGate.next(AskGate.waiting(read, decided: decided), presented: [])?.id, "r2")
        XCTAssertNil(AskGate.next([], presented: []), "nothing waits for the owner: nothing pops up")
        XCTAssertEqual(AskGate.next(read, presented: ["r1"])?.id, "r2")
    }

    func testAnAskOpenOnARequestAModeDecidedCloses() {
        XCTAssertTrue(AskGate.closes(.approval("r1"), decided: "r1"))
        XCTAssertTrue(AskGate.closes(.upload("b1"), decided: "b1"), "an upload a bypass approved too")
        XCTAssertFalse(AskGate.closes(.approval("r2"), decided: "r1"))
        XCTAssertFalse(AskGate.closes(nil, decided: "r1"))
    }

    func testOnlyTheLastDecisionsAreRemembered() {
        var decided: [String] = []
        for i in 0..<(AskGate.remembered + 5) { decided = AskGate.remember("r\(i)", in: decided) }
        XCTAssertEqual(decided.count, AskGate.remembered)
        XCTAssertEqual(decided.last, "r\(AskGate.remembered + 4)")
        XCTAssertFalse(decided.contains("r0"))
        XCTAssertEqual(AskGate.remember("r9", in: ["r9", "r8"]), ["r8", "r9"], "once each")
    }

    func testTheModelClosesTheAskAModeDecidedAndKeepsItOutOfTheList() async throws {
        let model = AppModel(core: DemoReinsCore(signedIn: true, syncCap: 0.3), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        await model.refreshPending()
        let asked = try XCTUnwrap(model.pending.first { $0.kind == .request })
        model.openSheet(asked.sheetTarget)
        model.decidedAutomatically(asked.id)
        XCTAssertNil(model.sheet, "the ask closed by itself")
        XCTAssertFalse(model.pending.contains { $0.id == asked.id })
        // The demo core still lists it, as a read made a moment before the decision would.
        await model.refreshPending()
        XCTAssertFalse(model.pending.contains { $0.id == asked.id }, "it does not come back")
        // Another ask open stays open.
        let other = try XCTUnwrap(model.pending.first { $0.kind == .request })
        model.openSheet(other.sheetTarget)
        model.decidedAutomatically("someone-else")
        XCTAssertEqual(model.sheet, other.sheetTarget)
    }

    // MARK: What the extension shows

    func testTheExtensionAsksOnlyForWhatWaitsForTheOwner() {
        let original = UNMutableNotificationContent()
        original.title = "Reins"
        original.body = "Something is waiting for you"
        let s = FeedbackSettings()
        // A decision wins over everything: it is final.
        guard case .decided = PushOutcome.of(decision: decision("r1"), waiting: item("r1"), deciding: item("r1")) else {
            return XCTFail("a decision is said as decided")
        }
        let decided = NotificationContent.outcome(.of(decision: decision("r1"), waiting: nil, deciding: nil), original: original, settings: s)
        XCTAssertEqual(decided.categoryIdentifier, "autopilot")
        XCTAssertEqual(decided.body, "Approved by Bypass")
        // What waits for the owner is the ask.
        let asked = NotificationContent.outcome(.of(decision: nil, waiting: item("r2"), deciding: nil), original: original, settings: s)
        XCTAssertEqual(asked.categoryIdentifier, "request")
        // What Autopilot has still to decide is never an ask.
        let deciding = NotificationContent.outcome(.of(decision: nil, waiting: nil, deciding: item("r3")), original: original, settings: s)
        XCTAssertEqual(deciding.categoryIdentifier, "deciding")
        XCTAssertTrue(deciding.title.hasPrefix("Claude: "), deciding.title)
        XCTAssertEqual(deciding.body, "Open Reins so Autopilot can decide")
        XCTAssertEqual(deciding.userInfo[NotificationKey.id] as? String, "r3")
        XCTAssertEqual(deciding.userInfo[NotificationKey.link] as? String, "reins://home", "opens Reins, not the request")
        // Gone already: a quiet line.
        let gone = NotificationContent.outcome(.of(decision: nil, waiting: nil, deciding: nil), original: original, settings: s)
        XCTAssertEqual(gone.title, "Already handled")
        XCTAssertNil(gone.sound)
    }

    func testNeitherAnUnreadPushNorOneAutopilotDecidesCanBeAnswered() {
        let push = UNMutableNotificationContent()
        push.userInfo = ["t": "req", "id": "r1"]
        let unread = NotificationContent.generic(push, payload: PushPayload(userInfo: push.userInfo), settings: FeedbackSettings())
        XCTAssertEqual(unread.categoryIdentifier, "arriving", "it may be one a bypass answers: no Deny")
        XCTAssertEqual(unread.userInfo[NotificationKey.link] as? String, "reins://item?kind=request&id=r1", "a tap still opens it")
        let categories = NotificationRouter.categories()
        for id in ["arriving", "deciding"] {
            let c = categories.first { $0.identifier == id }
            XCTAssertNotNil(c, id)
            XCTAssertEqual(c?.actions.count, 0, id)
        }
    }

    func testOvernightLeavesItsJudgingToTheApp() {
        let s = AutopilotSettings(
            mode: .overnight, baseMode: .manual, bypassUntil: nil, bypassSince: nil, defaultProfileId: "p", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 1, downloadedBytes: 1, error: nil, runtimeReady: true),
            connections: [ConnectionAutopilot(connectionId: "c1", baseMode: .overnight, bypassUntil: nil, bypassSince: nil, mode: .overnight, profileId: "p")]
        )
        XCTAssertTrue(ExtensionPolicy.needsModel(s), "never loaded in the extension, never judged as could not judge")
    }

    // MARK: Withdrawing

    func testADecisionWithdrawsEveryNotificationAboutItsRequestButItsOwn() {
        let delivered: [(identifier: String, item: String?)] = [
            ("r1", "r1"), // the app's ask
            ("push-abc", "r1"), // the extension's text, filed under the push's identifier
            ("auto:r1", nil), // the decision itself names no item
            ("r2", "r2"),
        ]
        XCTAssertEqual(NotificationSweep.about("r1", delivered: delivered), ["r1", "push-abc"])
        XCTAssertEqual(NotificationSweep.about("r1", delivered: delivered, keeping: "r1"), ["push-abc"], "the ask itself stays")
    }

    func testTheSweepKeepsWhatWaitsAndWhatAutopilotIsStillDeciding() {
        let at = Date(timeIntervalSince1970: TimeInterval(now))
        func stale(_ category: NotificationCategory, ago: TimeInterval, waiting: Set<String> = []) -> Bool {
            NotificationSweep.isStale(category: category.rawValue, id: "r1", date: at.addingTimeInterval(-ago), waiting: waiting, now: at)
        }
        XCTAssertTrue(stale(.request, ago: 120), "an ask for something that no longer waits")
        XCTAssertFalse(stale(.request, ago: 120, waiting: ["r1"]))
        XCTAssertFalse(stale(.arriving, ago: 30), "left a minute for the app to fetch it")
        XCTAssertTrue(stale(.arriving, ago: 90))
        XCTAssertFalse(stale(.deciding, ago: 120), "Autopilot is still to decide: never in the owner's list")
        XCTAssertTrue(stale(.deciding, ago: 700), "the server let it go")
        XCTAssertTrue(stale(.autopilot, ago: 0), "an automatic decision is in Activity: swept as soon as the app runs")
        XCTAssertFalse(stale(.join, ago: 9_999))
    }
}
