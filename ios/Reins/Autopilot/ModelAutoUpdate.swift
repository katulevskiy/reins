import Foundation
import Observation
import UIKit

/// When a model update may download (pure, so the rules are tested on their own).
enum ModelUpdateGate {
    /// An automatic update: an unmetered network (Wi-Fi or Ethernet, not mobile data or a hotspot), not Low Data Mode,
    /// not Low Power Mode. "Download on Wi-Fi only" being off does not loosen it: on mobile data it waits.
    static func automatic(_ network: NetworkState, lowPower: Bool) -> Bool {
        network.online && !network.metered && !network.constrained && !lowPower
    }

    /// "Check now", the user's own: any network unless "Download on Wi-Fi only" holds it to an unmetered one. Low
    /// Power and Low Data Mode do not stop what the user asked for.
    static func checkNow(_ network: NetworkState, wifiOnly: Bool) -> Bool {
        network.online && (!network.metered || !wifiOnly)
    }
}

/// Low Power Mode and when it changes. A protocol so tests need no device.
@MainActor
protocol PowerWatching: AnyObject {
    var lowPower: Bool { get }
    var onChange: ((Bool) -> Void)? { get set }
}

/// Time to finish when the user leaves the app (`UIApplication.beginBackgroundTask`); `end` gives it back.
@MainActor
protocol BackgroundTime: AnyObject {
    func begin()
    func end()
}

/// The two calls an update needs from the core. A protocol so tests can answer them.
protocol ModelUpdateCore: Sendable {
    func modelUpdatePending() async -> Bool
    func updateModel(automatic: Bool, progress: DownloadProgress) async throws -> ModelUpdateOutcome
}

/// The real core behind `ModelUpdateCore`.
struct CoreModelUpdates: ModelUpdateCore {
    let core: any ReinsCoreProtocol

    func modelUpdatePending() async -> Bool { await core.modelUpdatePending() }

    func updateModel(automatic: Bool, progress: DownloadProgress) async throws -> ModelUpdateOutcome {
        try await core.updateModel(automatic: automatic, progress: progress)
    }
}

/// What "Check now" came to, in one line under it.
enum ModelCheckResult: Equatable {
    case upToDate
    case updated
    case unavailable
    case waitingForWifi
    case failed(String)

    var text: String {
        switch self {
        case .upToDate: "Up to date"
        case .updated: "Updated"
        case .unavailable: "Not available yet"
        case .waitingForWifi: "Waiting for Wi-Fi"
        case let .failed(message): message
        }
    }

    var failed: Bool {
        if case .failed = self { return true }
        return false
    }
}

/// Keeps Autopilot's model up to date by itself (the Android app's `ModelAutoUpdate`). The core knows whether a newer
/// model this build trusts is published, downloads it (going on from an interrupted download), checks every file
/// against the hashes built into it, runs it once and only then puts it in use; on any failure the model in use stays.
/// This decides when: at launch and in the background task, on an unmetered network outside Low Data and Low Power
/// Mode, else as soon as they allow it while the app runs. Never a notification, sound, haptic or Live Activity: the
/// only trace is the model's row ("v2 · Updated today").
@Observable
@MainActor
final class ModelAutoUpdater {
    /// Why a check runs (for reading the code; every one does the same).
    enum Reason { case launch, background, network, power, switchedOn }

    /// An update (automatic or "Check now") is downloading or being put in use.
    private(set) var running = false
    private(set) var downloaded: UInt64 = 0
    private(set) var total: UInt64 = 0
    /// An automatic update waits for Wi-Fi (or for Low Data or Low Power Mode to end).
    private(set) var waiting = false
    /// "Check now" runs (waiting for Wi-Fi does not count).
    private(set) var checking = false
    /// What the last "Check now" came to; nil before the first.
    private(set) var checkResult: ModelCheckResult?
    private(set) var network: NetworkState
    private(set) var lowPower: Bool

    /// Whether an automatic update may run at all (signed in); the app sets it.
    @ObservationIgnored var canRun: () -> Bool = { true }
    /// The first download runs (`ModelDownloads`): an automatic update leaves it alone.
    @ObservationIgnored var downloadBusy: () -> Bool = { false }
    /// "Download on Wi-Fi only", for "Check now".
    @ObservationIgnored var wifiOnly: () -> Bool = { true }
    /// Called after every update that ran, whatever it came to (the app re-reads Autopilot's settings).
    @ObservationIgnored var onFinished: (() async -> Void)?

    @ObservationIgnored private let core: any ModelUpdateCore
    @ObservationIgnored private let watcher: NetworkWatching
    @ObservationIgnored private let power: PowerWatching
    @ObservationIgnored private let background: BackgroundTime
    /// The check running now; the next one waits for it.
    @ObservationIgnored private var work: Task<Void, Never>?
    /// Checks started and not yet over.
    @ObservationIgnored private var queued = 0
    /// "Check now" waits for a network it may use.
    @ObservationIgnored private var checkWaiting = false
    @ObservationIgnored private var lastShown = Date.distantPast

    init(core: any ModelUpdateCore, network: NetworkWatching? = nil, power: PowerWatching? = nil, background: BackgroundTime? = nil) {
        self.core = core
        self.watcher = network ?? PathNetwork()
        self.power = power ?? SystemPower()
        self.background = background ?? SystemBackgroundTime()
        self.network = watcher.current
        self.lowPower = self.power.lowPower
        watcher.onChange = { [weak self] state in self?.networkChanged(state) }
        self.power.onChange = { [weak self] low in self?.powerChanged(low) }
    }

    /// Updates the model if a newer one is due and the network and power allow, else remembers to when they do. A
    /// check already running is waited for instead.
    func check(_ reason: Reason) async {
        if let running = work {
            await running.value
            return
        }
        await enqueue { await self.automatic() }
    }

    /// The user's "Check now": updates now if the network allows (Wi-Fi only: an unmetered one), else waits for it and
    /// runs by itself then.
    func checkNow() async {
        guard !checking else { return }
        checkResult = nil
        guard ModelUpdateGate.checkNow(network, wifiOnly: wifiOnly()) else {
            checkWaiting = true
            checkResult = .waitingForWifi
            return
        }
        await startUserCheck()
    }

    /// Runs "Check now" after whatever runs (an automatic update it would otherwise collide with).
    private func startUserCheck() async {
        checkWaiting = false
        checking = true
        await enqueue { await self.userCheck() }
    }

    /// Runs `body` after the checks before it. `work` is cleared as the last one ends, in the same step, so a check that
    /// comes right after runs rather than waiting for one already over.
    private func enqueue(_ body: @escaping @MainActor () async -> Void) async {
        let previous = work
        queued += 1
        let task = Task { @MainActor in
            await previous?.value
            await body()
            self.queued -= 1
            if self.queued == 0 { self.work = nil }
        }
        work = task
        await task.value
    }

    private func automatic() async {
        guard canRun(), await core.modelUpdatePending() else {
            waiting = false
            return
        }
        guard !downloadBusy() else { return }
        guard ModelUpdateGate.automatic(network, lowPower: lowPower) else {
            waiting = true
            return
        }
        waiting = false
        do {
            _ = try await update(automatic: true)
        } catch CoreError.Network {
            // What arrived is kept: the next network change or launch goes on from there.
            waiting = true
        } catch CoreError.Server {
            waiting = true
        } catch {
            // Failed its check or its self-check (the core keeps the model in use and does not try that one again by
            // itself), or a download already runs: nothing to say.
        }
        await onFinished?()
    }

    private func userCheck() async {
        defer { checking = false }
        do {
            switch try await update(automatic: false) {
            case .updated: checkResult = .updated
            case .unavailable, .rejected: checkResult = .unavailable
            case .upToDate, .off, .notInstalled: checkResult = .upToDate
            }
            waiting = false
        } catch {
            checkResult = .failed(Self.message(error))
        }
        await onFinished?()
    }

    /// Why "Check now" did not work, in one line. The model server's refusal is not the account server's (whose 403
    /// means another phone took over).
    static func message(_ error: Error) -> String {
        if case let CoreError.Server(status, _) = error { return "The model server had a problem (\(status)). Try again later." }
        return AutopilotText.modelError(error.userMessage)
    }

    private func update(automatic: Bool) async throws -> ModelUpdateOutcome {
        running = true
        downloaded = 0
        total = 0
        background.begin()
        defer {
            running = false
            background.end()
        }
        let relay = ProgressRelay { [weak self] done, all in
            Task { @MainActor in self?.progress(done, all) }
        }
        return try await core.updateModel(automatic: automatic, progress: relay)
    }

    private func progress(_ done: UInt64, _ all: UInt64) {
        guard running else { return }
        let now = Date()
        guard now.timeIntervalSince(lastShown) >= ModelDownloads.redraw || done == all else { return }
        lastShown = now
        downloaded = done
        total = all
    }

    private func networkChanged(_ state: NetworkState) {
        network = state
        resume()
    }

    private func powerChanged(_ low: Bool) {
        lowPower = low
        resume()
    }

    /// What waited for the network or the power goes now that they allow it.
    private func resume() {
        if checkWaiting, !checking, ModelUpdateGate.checkNow(network, wifiOnly: wifiOnly()) {
            checkWaiting = false
            checking = true
            Task { await self.enqueue { await self.userCheck() } }
        }
        if waiting, ModelUpdateGate.automatic(network, lowPower: lowPower) {
            Task { await self.check(.network) }
        }
    }
}

// MARK: The real power state and background time

/// `ProcessInfo.isLowPowerModeEnabled` and its change notification.
@MainActor
final class SystemPower: PowerWatching {
    private(set) var lowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
    var onChange: ((Bool) -> Void)?
    private var observer: NSObjectProtocol?

    init() {
        observer = NotificationCenter.default.addObserver(forName: .NSProcessInfoPowerStateDidChange, object: nil, queue: .main) { [weak self] _ in
            let low = ProcessInfo.processInfo.isLowPowerModeEnabled
            MainActor.assumeIsolated {
                guard let self, low != self.lowPower else { return }
                self.lowPower = low
                self.onChange?(low)
            }
        }
    }
}

/// Background time from UIKit, for an update the user leaves the app during.
@MainActor
final class SystemBackgroundTime: BackgroundTime {
    private var task: UIBackgroundTaskIdentifier = .invalid

    func begin() {
        guard task == .invalid else { return }
        task = UIApplication.shared.beginBackgroundTask(withName: "model-update") { [weak self] in
            // Out of time: iOS suspends the app; the core keeps what arrived and goes on from there next time.
            MainActor.assumeIsolated { self?.end() }
        }
    }

    func end() {
        guard task != .invalid else { return }
        UIApplication.shared.endBackgroundTask(task)
        task = .invalid
    }
}
