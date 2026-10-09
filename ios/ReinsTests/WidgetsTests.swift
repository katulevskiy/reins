import XCTest
@testable import Reins

/// The logic behind widgets, controls, Live Activities and intents (Shared/Widgets/Glance.swift, the intents' links,
/// the Lockdown switch).
final class WidgetsTests: XCTestCase {
    private let now: Int64 = 1_800_000_000

    private func item(_ id: String, created: Int64, expires: Int64, kind: Snapshot.Item.Kind = .request, connection: String = "Claude", title: String? = nil) -> Snapshot.Item {
        Snapshot.Item(
            id: id, kind: kind, title: title ?? "\(connection): Search email", subtitle: "me@gmail.com", connection: connection,
            service: "gmail", createdAt: created, expiresAt: expires, suggestion: nil
        )
    }

    // MARK: Waiting

    func testWaitingIsNewestFirstAndDropsClosedWindows() {
        let items = [
            item("old", created: now - 300, expires: now + 300),
            item("gone", created: now - 100, expires: now),
            item("new", created: now - 10, expires: now + 50),
        ]
        XCTAssertEqual(Glance.waiting(items, now: now).map(\.id), ["new", "old"])
    }

    func testOperationDropsTheAIsName() {
        XCTAssertEqual(Glance.operation(item("a", created: now, expires: now + 1, title: "Claude: Send email to 2")), "Send email to 2")
        XCTAssertEqual(Glance.operation(item("b", created: now, expires: now + 1, kind: .pairing, connection: "Gemini", title: "Connect Gemini to Reins?")), "Connect Gemini to Reins?")
        XCTAssertEqual(Glance.operation(item("c", created: now, expires: now + 1, connection: "", title: "Share a file")), "Share a file")
    }

    func testBylineLeavesOutThePairingsName() {
        XCTAssertEqual(Glance.byline(item("a", created: now, expires: now + 1)), "Claude · me@gmail.com")
        XCTAssertEqual(Glance.byline(item("b", created: now, expires: now + 1, kind: .pairing, connection: "Gemini")), "me@gmail.com")
    }

    func testTheWidgetOpensTheNewestItemOrHome() {
        var s = Snapshot()
        XCTAssertEqual(Glance.waitingLink(s, now: now), DeepLink.home.url)
        s.pending = [item("req1", created: now - 5, expires: now + 40), item("req2", created: now - 50, expires: now + 400)]
        XCTAssertEqual(DeepLink(url: Glance.waitingLink(s, now: now)), .item(kind: .request, id: "req1"))
        XCTAssertEqual(DeepLink(url: Glance.waitingLink(s, now: now + 45)), .item(kind: .request, id: "req2"), "req1's window closed")
    }

    // MARK: Timelines

    func testTimelineRedrawsWhenWindowsCloseAndBypassesEnd() {
        var s = Snapshot()
        s.pending = [
            item("a", created: now - 10, expires: now + 50),
            item("b", created: now - 10, expires: now + 50),
            item("c", created: now - 10, expires: now + 20),
            item("gone", created: now - 100, expires: now - 1),
        ]
        s.bypassUntil = now + 900
        s.anyBypassUntil = now + 1_200
        XCTAssertEqual(Glance.timelineDates(s, now: now), [now, now + 20, now + 50, now + 900, now + 1_200])
        XCTAssertEqual(Glance.timelineDates(Snapshot(), now: now), [now], "nothing runs out: only the entry for now")
        XCTAssertEqual(Glance.timelineDates(s, now: now, limit: 2), [now, now + 20, now + 50])
    }

    // MARK: Autopilot

    func testABypassThatRanOutShowsTheModeItWentBackTo() {
        var s = Snapshot()
        s.autopilotMode = "bypass"
        s.baseMode = "assisted"
        s.bypassUntil = now + 60
        XCTAssertEqual(Glance.mode(s, now: now), .bypass)
        XCTAssertEqual(Glance.bypassEnd(s, now: now), now + 60)
        XCTAssertEqual(Glance.mode(s, now: now + 61), .assisted)
        XCTAssertNil(Glance.bypassEnd(s, now: now + 61))
        s.baseMode = nil
        XCTAssertEqual(Glance.mode(s, now: now + 61), .manual)
        s.autopilotMode = "nonsense"
        XCTAssertEqual(Glance.mode(s, now: now), .manual)
    }

    func testAConnectionsBypassShowsUnderTheGlobalMode() {
        var s = Snapshot()
        s.autopilotMode = "auto"
        s.anyBypassUntil = now + 300
        XCTAssertEqual(Glance.mode(s, now: now), .auto)
        XCTAssertEqual(Glance.bypassEnd(s, now: now), now + 300)
        XCTAssertTrue(Glance.connectionBypassOnly(s, now: now))
        s.bypassUntil = now + 300
        XCTAssertFalse(Glance.connectionBypassOnly(s, now: now))
    }

    func testLockdownSwitch() {
        XCTAssertEqual(IntentBridge.lockdownTarget(on: true, mode: .assisted, base: .assisted), .lockdown)
        XCTAssertNil(IntentBridge.lockdownTarget(on: true, mode: .lockdown, base: .lockdown), "already locked down")
        XCTAssertEqual(IntentBridge.lockdownTarget(on: false, mode: .lockdown, base: .auto), .auto)
        XCTAssertEqual(IntentBridge.lockdownTarget(on: false, mode: .lockdown, base: .lockdown), .manual, "as Android's End lockdown")
        XCTAssertNil(IntentBridge.lockdownTarget(on: false, mode: .bypass, base: .auto), "off when not locked down changes nothing")
    }

    @MainActor
    func testPauseAndResumeAreLockdownOnAndOff() async throws {
        let model = AppModel(core: DemoReinsCore(signedIn: true, syncCap: 0.3), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        IntentBridge.model = model
        defer { IntentBridge.model = nil }
        let nothingToResume = try await IntentBridge.endLockdown()
        XCTAssertFalse(nothingToResume, "not paused: nothing to resume")
        try await IntentBridge.setLockdown(true)
        XCTAssertEqual(model.autopilot?.mode, .lockdown)
        let resumed = try await IntentBridge.endLockdown()
        XCTAssertTrue(resumed)
        XCTAssertNotEqual(model.autopilot?.mode, .lockdown)
    }

    func testIntentsSetEveryModeButBypass() {
        for option in [AutopilotModeOption.manual, .assisted, .auto, .lockdown] {
            XCTAssertEqual(AutopilotModeOption(option.mode), option)
        }
        XCTAssertNil(AutopilotModeOption(AutopilotMode.bypass))
    }

    @MainActor
    func testAFocusSetsTheModeAndPutsTheOldOneBack() async throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "focus-tests"))
        defaults.removePersistentDomain(forName: "focus-tests")
        defer { defaults.removePersistentDomain(forName: "focus-tests") }
        let model = AppModel(core: DemoReinsCore(signedIn: true, syncCap: 0.3), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        IntentBridge.model = model
        defer { IntentBridge.model = nil }
        try await IntentBridge.setMode(.assisted)
        try await IntentBridge.applyFocus(.lockdown, defaults: defaults)
        XCTAssertEqual(model.autopilot?.mode, .lockdown)
        // A second Focus while the first is on keeps the mode from before both.
        try await IntentBridge.applyFocus(.manual, defaults: defaults)
        try await IntentBridge.applyFocus(nil, defaults: defaults)
        XCTAssertEqual(model.autopilot?.mode, .assisted)
    }

    // MARK: Activity

    func testAgo() {
        XCTAssertEqual(Glance.ago(now - 20, now: now), "now")
        XCTAssertEqual(Glance.ago(now - 300, now: now), "5 min")
        XCTAssertEqual(Glance.ago(now - 3 * 3_600, now: now), "3 h")
        XCTAssertEqual(Glance.ago(now - 2 * 86_400, now: now), "2 d")
        XCTAssertEqual(Glance.ago(now + 100, now: now), "now", "a clock that is a little behind")
    }

    // MARK: Live Activities

    func testApprovalStateShowsTheNewestAndTheCount() {
        var a = item("req1", created: now - 5, expires: now + 40, title: "Claude: Search email")
        a.connectionIcon = "claude"
        let b = item("pair1", created: now - 60, expires: now + 500, kind: .pairing, connection: "Gemini", title: "Connect Gemini to Reins?")
        let state = Glance.approvalState([b, a], now: now)
        XCTAssertEqual(state?.itemId, "req1")
        XCTAssertEqual(state?.title, "Search email")
        XCTAssertEqual(state?.count, 2)
        XCTAssertEqual(state?.connectionIcon, "claude")
        XCTAssertEqual(state?.expiresAt, Date(timeIntervalSince1970: TimeInterval(now + 40)))
        XCTAssertEqual(Glance.approvalState([b, a], now: now + 41)?.itemId, "pair1")
        XCTAssertNil(Glance.approvalState([b, a], now: now + 600), "nothing waits: the activity ends")
    }

    func testARoutineRequestCarriesApproveToTheLiveActivity() {
        var routine = item("req1", created: now - 5, expires: now + 40)
        routine.quick = true
        XCTAssertEqual(Glance.approvalState([routine], now: now)?.quick, true)
        // Asked every time (or written before the field existed): Deny and Review only.
        XCTAssertNotEqual(Glance.approvalState([item("req2", created: now - 5, expires: now + 40)], now: now)?.quick, true)
    }

    func testTheApproveIntentTargetsTheRequest() {
        XCTAssertEqual(ApproveQuickIntent(requestId: "req1").requestId, "req1")
    }

    func testBypassLengthIsTheShortestChoiceThatFits() {
        XCTAssertEqual(Glance.bypassLength(left: 10 * 60), 15 * 60)
        XCTAssertEqual(Glance.bypassLength(left: 15 * 60), 15 * 60)
        XCTAssertEqual(Glance.bypassLength(left: 16 * 60), 30 * 60)
        XCTAssertEqual(Glance.bypassLength(left: 90 * 60), 60 * 60)
    }

    func testBypassStateCoversGlobalAndConnectionBypasses() {
        let none = Glance.bypassState(global: now - 5, connections: [("Claude", now - 1), ("Cursor", nil)], previous: nil, approvedSince: { _ in 0 }, now: now)
        XCTAssertNil(none, "bypasses that ended do not count")

        let global = Glance.bypassState(global: now + 600, connections: [("Claude", now + 900)], previous: nil, approvedSince: { _ in 4 }, now: now)
        XCTAssertEqual(global?.scope, "every AI")
        XCTAssertEqual(global?.until, Date(timeIntervalSince1970: TimeInterval(now + 900)), "the last one to end")
        XCTAssertEqual(global?.startedAt, Date(timeIntervalSince1970: TimeInterval(now + 900 - 15 * 60)))
        XCTAssertEqual(global?.approvedCount, 4)

        let one = Glance.bypassState(global: nil, connections: [("Claude", now + 60), ("Cursor", nil)], previous: nil, approvedSince: { _ in 0 }, now: now)
        XCTAssertEqual(one?.scope, "Claude")
        let two = Glance.bypassState(global: nil, connections: [("Claude", now + 60), ("Cursor", now + 70)], previous: nil, approvedSince: { _ in 0 }, now: now)
        XCTAssertEqual(two?.scope, "2 AIs")
    }

    func testTheSameBypassKeepsItsStartAndARestartBeginsANewRing() {
        let first = Glance.bypassState(global: now + 900, connections: [], previous: nil, approvedSince: { _ in 0 }, now: now)
        let later = Glance.bypassState(global: now + 900, connections: [], previous: first, approvedSince: { _ in 1 }, now: now + 400)
        XCTAssertEqual(later?.startedAt, first?.startedAt, "a later refresh does not move the ring")
        let restarted = Glance.bypassState(global: now + 400 + 1_800, connections: [], previous: later, approvedSince: { _ in 0 }, now: now + 400)
        XCTAssertEqual(restarted?.startedAt, Date(timeIntervalSince1970: TimeInterval(now + 400)))
    }

    func testDownloadProgress() {
        let half = ModelDownloadActivityAttributes.ContentState(downloaded: 185_000_000, total: 370_000_000, finished: false, failed: false)
        XCTAssertEqual(Glance.downloadFraction(half), 0.5, accuracy: 0.001)
        XCTAssertEqual(Glance.downloadFraction(.init(downloaded: 5, total: 0, finished: false, failed: false)), 0)
        XCTAssertEqual(Glance.downloadFraction(.init(downloaded: 0, total: 0, finished: true, failed: false)), 1)
        XCTAssertEqual(Glance.downloadLine(.init(downloaded: 0, total: 0, finished: false, failed: true)), "The download stopped")
        XCTAssertTrue(Glance.downloadLine(half).contains(" of "))
    }

    // MARK: Intents

    func testShowWaitingOpensTheAskedItemTheNewestOrHome() {
        var s = Snapshot()
        XCTAssertEqual(ShowWaitingIntent.link(for: nil, in: s, now: now), .home)
        s.pending = [item("req1", created: now - 5, expires: now + 40), item("pair1", created: now - 60, expires: now + 500, kind: .pairing)]
        XCTAssertEqual(ShowWaitingIntent.link(for: nil, in: s, now: now), .item(kind: .request, id: "req1"))
        XCTAssertEqual(ShowWaitingIntent.link(for: "pair1", in: s, now: now), .item(kind: .pairing, id: "pair1"))
        XCTAssertEqual(ShowWaitingIntent.link(for: "gone", in: s, now: now), .item(kind: .request, id: "req1"), "an item no longer waiting falls back to the newest")
    }

    func testApprovedDuringBypassCountsOnlyBypassApprovalsSinceTheStart() {
        func entry(_ id: Int64, at: Int64, outcome: String, by: String) -> ActivityEntry {
            DemoData.entry(id, at: at, action: "read", outcome: outcome, detail: "", decidedBy: by)
        }
        let list = [
            entry(1, at: now - 100, outcome: "released", by: "bypass"),
            entry(2, at: now + 10, outcome: "released", by: "bypass"),
            entry(3, at: now + 20, outcome: "denied", by: "bypass"),
            entry(4, at: now + 30, outcome: "released", by: ""),
            entry(5, at: now + 40, outcome: "sent", by: "bypass"),
        ]
        XCTAssertEqual(LiveActivityController.approvedDuringBypass(list, since: Date(timeIntervalSince1970: TimeInterval(now))), 2)
    }

    // MARK: Snapshot

    func testOlderSnapshotsWithoutTheNewFieldsStillLoad() throws {
        let old = #"{"signedIn":true,"approvalDevice":true,"pending":[{"id":"r","kind":"request","title":"t","subtitle":"","connection":"c","service":"gmail","createdAt":1,"expiresAt":2}],"latest":[],"autopilotMode":"auto","activeGrants":0,"updatedAt":3}"#
        let s = try JSONDecoder().decode(Snapshot.self, from: Data(old.utf8))
        XCTAssertNil(s.baseMode)
        XCTAssertNil(s.anyBypassUntil)
        XCTAssertNil(s.pending.first?.connectionIcon)
    }
}
