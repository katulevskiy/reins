import XCTest
@testable import Reins

/// When Autopilot's model updates itself (the Android app's `ModelAutoUpdateTest`): the network and power rules, and
/// the coordinator against a core that answers as told.
@MainActor
final class ModelAutoUpdateTests: XCTestCase {
    private var core: FakeUpdateCore!
    private var network: FakeNetwork!
    private var power: FakePower!
    private var background: FakeBackground!
    private var updater: ModelAutoUpdater!
    /// Each test's own, so that the waiting state kept across launches does not leak between tests.
    private var defaults: UserDefaults!

    private static let lowData = NetworkState(online: true, metered: false, constrained: true)

    private func start(_ state: NetworkState = .unmetered, lowPower: Bool = false, wifiOnly: Bool = true) {
        core = FakeUpdateCore()
        network = FakeNetwork(state)
        power = FakePower(lowPower)
        background = FakeBackground()
        defaults = UserDefaults(suiteName: "ModelAutoUpdateTests-\(UUID().uuidString)")
        updater = ModelAutoUpdater(core: core, network: network, power: power, background: background, defaults: defaults)
        updater.wifiOnly = { wifiOnly }
    }

    private func until(_ timeout: TimeInterval = 5, _ condition: () -> Bool) async {
        let end = Date().addingTimeInterval(timeout)
        while !condition() && Date() < end { try? await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(condition(), "timed out")
    }

    /// Lets the tasks a network or power change starts run.
    private func settle() async {
        try? await Task.sleep(for: .milliseconds(200))
    }

    // MARK: The rules

    func testAnAutomaticUpdateGoesOnlyOnWifiOutsideLowDataAndLowPowerMode() {
        XCTAssertTrue(ModelUpdateGate.automatic(.unmetered, lowPower: false), "Wi-Fi")
        XCTAssertFalse(ModelUpdateGate.automatic(.cellular, lowPower: false), "mobile data")
        XCTAssertFalse(ModelUpdateGate.automatic(NetworkState(online: true, metered: true), lowPower: false), "a personal hotspot is metered too")
        XCTAssertFalse(ModelUpdateGate.automatic(.offline, lowPower: false))
        XCTAssertFalse(ModelUpdateGate.automatic(Self.lowData, lowPower: false), "Low Data Mode")
        XCTAssertFalse(ModelUpdateGate.automatic(.unmetered, lowPower: true), "Low Power Mode")
    }

    func testCheckNowFollowsWifiOnlyAndIgnoresLowDataMode() {
        XCTAssertTrue(ModelUpdateGate.checkNow(.unmetered, wifiOnly: true))
        XCTAssertFalse(ModelUpdateGate.checkNow(.cellular, wifiOnly: true), "Wi-Fi only waits for Wi-Fi")
        XCTAssertTrue(ModelUpdateGate.checkNow(.cellular, wifiOnly: false), "without Wi-Fi only, mobile data will do")
        XCTAssertFalse(ModelUpdateGate.checkNow(.offline, wifiOnly: false))
        XCTAssertTrue(ModelUpdateGate.checkNow(Self.lowData, wifiOnly: true), "the user asked: Low Data Mode does not stop it")
    }

    // MARK: Automatic

    func testNothingDueAsksTheCoreForNothingMore() async {
        start()
        core.pending = false
        await updater.check(.launch)
        XCTAssertTrue(core.calls.isEmpty)
        XCTAssertFalse(updater.waiting)
        XCTAssertTrue(background.log.isEmpty)
    }

    func testADueUpdateRunsOnWifiInTheBackgroundTime() async {
        start()
        await updater.check(.launch)
        XCTAssertEqual(core.calls, [true], "automatic")
        XCTAssertFalse(updater.waiting)
        XCTAssertFalse(updater.running)
        XCTAssertEqual(background.log, ["begin", "end"])
        XCTAssertNil(updater.checkResult, "an automatic update says nothing")
    }

    func testOnMobileDataItWaitsForWifiEvenWithoutWifiOnly() async {
        start(.cellular, wifiOnly: false)
        await updater.check(.launch)
        XCTAssertTrue(core.calls.isEmpty)
        XCTAssertTrue(updater.waiting)
        network.set(.offline)
        await settle()
        XCTAssertTrue(core.calls.isEmpty)
        network.set(.unmetered)
        await until { core.calls == [true] }
        await until { !updater.running }
        XCTAssertFalse(updater.waiting)
    }

    func testLowPowerAndLowDataModeHoldItUntilTheyEnd() async {
        start(lowPower: true)
        await updater.check(.background)
        XCTAssertTrue(core.calls.isEmpty)
        XCTAssertTrue(updater.waiting)
        power.set(false)
        await until { core.calls == [true] }

        start(Self.lowData)
        await updater.check(.background)
        XCTAssertTrue(core.calls.isEmpty)
        XCTAssertTrue(updater.waiting)
        network.set(.unmetered)
        await until { core.calls == [true] }
    }

    func testAFirstDownloadRunningOrBeingSignedOutLeavesItAlone() async {
        start()
        updater.downloadBusy = { true }
        await updater.check(.launch)
        XCTAssertTrue(core.calls.isEmpty)

        start()
        updater.canRun = { false }
        await updater.check(.launch)
        XCTAssertTrue(core.calls.isEmpty)
    }

    func testADroppedConnectionIsTriedAgainOnTheNextNetworkChange() async {
        start()
        core.answer(.failure(CoreError.Network(reason: "connection reset")), .failure(CoreError.Server(status: 503, reason: "busy")))
        await updater.check(.launch)
        XCTAssertEqual(core.calls, [true])
        XCTAssertTrue(updater.waiting, "what arrived is kept; it goes on later")
        network.set(.cellular)
        await settle()
        XCTAssertEqual(core.calls.count, 1, "not on mobile data")
        network.set(.unmetered)
        await until { core.calls == [true, true] }
        await until { !updater.running }
        XCTAssertTrue(updater.waiting, "a server that refused is tried again later too")
        network.set(.offline)
        network.set(.unmetered)
        await until { core.calls == [true, true, true] }
        await until { !updater.running }
        XCTAssertFalse(updater.waiting)
    }

    func testAModelThatFailsItsCheckIsNotTriedAgainByItself() async {
        start()
        core.answer(.failure(CoreError.Service(reason: "the new model failed its self-check; the one in use stays")))
        await updater.check(.launch)
        XCTAssertEqual(core.calls, [true])
        XCTAssertFalse(updater.waiting)
        network.set(.offline)
        network.set(.unmetered)
        await settle()
        XCTAssertEqual(core.calls.count, 1)
        XCTAssertNil(updater.checkResult, "quietly: the old model stays")
    }

    // MARK: Leaving Wi-Fi during a download

    func testLeavingWifiStopsARunningAutomaticDownloadUntilWifiIsBack() async {
        start()
        core.hold = true
        let first = Task { await updater.check(.launch) }
        await until { updater.automaticRunning }
        network.set(.cellular)
        await until { core.pauses == 1 }
        await first.value
        XCTAssertFalse(updater.running)
        XCTAssertTrue(updater.waiting, "what arrived is kept; it waits for Wi-Fi")
        XCTAssertTrue(ModelAutoUpdater.persistedWaiting(defaults))
        core.hold = false
        network.set(.unmetered)
        await until { core.calls == [true, true] }
        await until { !updater.running }
        XCTAssertFalse(updater.waiting)
        XCTAssertFalse(ModelAutoUpdater.persistedWaiting(defaults))
    }

    func testLowDataOrLowPowerModeStopsARunningAutomaticDownload() async {
        start()
        core.hold = true
        let lowData = Task { await updater.check(.launch) }
        await until { updater.automaticRunning }
        network.set(Self.lowData)
        await until { core.pauses == 1 }
        await lowData.value
        XCTAssertTrue(updater.waiting)

        start()
        core.hold = true
        let lowPower = Task { await updater.check(.launch) }
        await until { updater.automaticRunning }
        power.set(true)
        await until { core.pauses == 1 }
        await lowPower.value
        XCTAssertTrue(updater.waiting)
    }

    func testAUsersCheckNowIsNotStoppedOnMobileData() async {
        start(wifiOnly: false)
        core.hold = true
        let check = Task { await updater.checkNow() }
        await until { core.calls == [false] }
        network.set(.cellular)
        await settle()
        XCTAssertEqual(core.pauses, 0, "the user asked, and mobile data is allowed")
        core.hold = false
        await check.value
        XCTAssertEqual(updater.checkResult, .updated)
    }

    func testTheBackgroundTaskRunningOutOfTimeStopsTheDownload() async {
        start()
        core.hold = true
        let run = Task { await updater.check(.background) }
        await until { updater.automaticRunning }
        await updater.pause()
        await run.value
        XCTAssertEqual(core.pauses, 1)
        XCTAssertTrue(updater.waiting)
        // Nothing runs: nothing to stop.
        await updater.pause()
        XCTAssertEqual(core.pauses, 1)
    }

    // MARK: Waiting across launches

    func testWaitingForWifiIsKeptAcrossLaunches() async {
        start(.cellular)
        await updater.check(.background)
        XCTAssertTrue(updater.waiting)
        XCTAssertTrue(ModelAutoUpdater.persistedWaiting(defaults))
        let relaunched = ModelAutoUpdater(core: core, network: FakeNetwork(.cellular), power: FakePower(false), background: FakeBackground(),
                                          defaults: defaults)
        XCTAssertTrue(relaunched.waiting, "a background run that found mobile data is remembered")
    }

    func testTheNextBackgroundRunComesSoonerWhileWaitingAndIsNeverPushedBack() {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let day = BackgroundModelUpdate.interval, soon = BackgroundModelUpdate.retry
        XCTAssertLessThanOrEqual(soon, 6 * 60 * 60, "a few hours")
        XCTAssertEqual(BackgroundModelUpdate.nextRun(existing: nil, waiting: false, now: now), now.addingTimeInterval(day))
        XCTAssertEqual(BackgroundModelUpdate.nextRun(existing: nil, waiting: true, now: now), now.addingTimeInterval(soon))
        // Leaving the app again does not push an asked-for run back.
        XCTAssertNil(BackgroundModelUpdate.nextRun(existing: now.addingTimeInterval(60), waiting: false, now: now))
        XCTAssertNil(BackgroundModelUpdate.nextRun(existing: now.addingTimeInterval(day - 60), waiting: false, now: now))
        XCTAssertNil(BackgroundModelUpdate.nextRun(existing: .distantPast, waiting: true, now: now), "one without a start date")
        // A waiting update brings a day-away run forward.
        XCTAssertEqual(BackgroundModelUpdate.nextRun(existing: now.addingTimeInterval(day), waiting: true, now: now),
                       now.addingTimeInterval(soon))
        XCTAssertNil(BackgroundModelUpdate.nextRun(existing: now.addingTimeInterval(soon - 60), waiting: true, now: now))
    }

    // MARK: Check now

    func testCheckNowAsksTheCoreAsTheUserAndSaysWhatCameOfIt() async {
        start()
        core.pending = false
        core.answer(
            .success(.upToDate), .success(.updated), .success(.unavailable),
            .failure(CoreError.Network(reason: "connection reset")), .failure(CoreError.Server(status: 403, reason: "forbidden"))
        )
        await updater.checkNow()
        XCTAssertEqual(core.calls, [false], "the user's, even with nothing due")
        XCTAssertEqual(updater.checkResult?.text, "Up to date")
        XCTAssertFalse(updater.checking)
        await updater.checkNow()
        XCTAssertEqual(updater.checkResult?.text, "Updated")
        await updater.checkNow()
        XCTAssertEqual(updater.checkResult?.text, "Not available yet")
        await updater.checkNow()
        XCTAssertEqual(updater.checkResult?.text, "No connection: connection reset.")
        XCTAssertEqual(updater.checkResult?.failed, true)
        await updater.checkNow()
        XCTAssertEqual(updater.checkResult?.text, "The model server had a problem (403). Try again later.")
        XCTAssertEqual(core.calls, [false, false, false, false, false])
    }

    func testCheckNowOnMobileDataWaitsForWifiThenRunsByItself() async {
        start(.cellular, wifiOnly: true)
        await updater.checkNow()
        XCTAssertTrue(core.calls.isEmpty)
        XCTAssertEqual(updater.checkResult?.text, "Waiting for Wi-Fi")
        network.set(.unmetered)
        await until { core.calls == [false] }
        await until { updater.checkResult == .updated }
    }

    func testCheckNowGoesInLowPowerModeAndOnMobileDataWithoutWifiOnly() async {
        start(lowPower: true)
        await updater.checkNow()
        XCTAssertEqual(core.calls, [false])

        start(.cellular, wifiOnly: false)
        await updater.checkNow()
        XCTAssertEqual(core.calls, [false])
    }
}

// MARK: Fakes

/// Answers `updateModel` with what it was told, in order (then `.updated`), and records each call's `automatic`. With
/// `hold`, a download goes on until `hold` is cleared, or until it is paused (then it ends as the core's does, with a
/// network error).
private final class FakeUpdateCore: ModelUpdateCore, @unchecked Sendable {
    private let lock = NSLock()
    private var pendingValue = true
    private var answers: [Result<ModelUpdateOutcome, Error>] = []
    private var made: [Bool] = []
    private var holding = false
    private var paused = false
    private var pausedCount = 0

    var pending: Bool {
        get { lock.withLock { pendingValue } }
        set { lock.withLock { pendingValue = newValue } }
    }

    var hold: Bool {
        get { lock.withLock { holding } }
        set { lock.withLock { holding = newValue } }
    }

    var calls: [Bool] { lock.withLock { made } }
    var pauses: Int { lock.withLock { pausedCount } }

    func answer(_ results: Result<ModelUpdateOutcome, Error>...) {
        lock.withLock { answers += results }
    }

    func modelUpdatePending() async -> Bool { pending }

    func updateModel(automatic: Bool, progress: DownloadProgress) async throws -> ModelUpdateOutcome {
        let next: Result<ModelUpdateOutcome, Error> = lock.withLock {
            made.append(automatic)
            paused = false
            return answers.isEmpty ? .success(.updated) : answers.removeFirst()
        }
        while lock.withLock({ holding && !paused }) { try? await Task.sleep(for: .milliseconds(10)) }
        if lock.withLock({ paused }) {
            throw CoreError.Network(reason: "the download was paused; it goes on from here next time")
        }
        progress.progress(downloaded: 10, total: 10)
        return try next.get()
    }

    func pauseModelDownload() async {
        lock.withLock {
            paused = true
            pausedCount += 1
        }
    }
}

@MainActor
private final class FakeNetwork: NetworkWatching {
    private(set) var current: NetworkState
    var onChange: ((NetworkState) -> Void)?

    init(_ state: NetworkState) { current = state }

    func set(_ state: NetworkState) {
        current = state
        onChange?(state)
    }
}

@MainActor
private final class FakePower: PowerWatching {
    private(set) var lowPower: Bool
    var onChange: ((Bool) -> Void)?

    init(_ low: Bool) { lowPower = low }

    func set(_ low: Bool) {
        lowPower = low
        onChange?(low)
    }
}

@MainActor
private final class FakeBackground: BackgroundTime {
    var log: [String] = []
    func begin() { log.append("begin") }
    func end() { log.append("end") }
}
