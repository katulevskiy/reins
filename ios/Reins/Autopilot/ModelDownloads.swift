import Foundation
import Network
import Observation
import UIKit

/// Where the model download stands as far as the app goes (the Android app's `DownloadJob`).
enum DownloadJob: Equatable {
    /// Nothing asked for, or it finished.
    case idle
    /// Asked for, waiting for Wi-Fi (or any network).
    case waiting
    /// Downloading now.
    case running
}

/// The network as far as the download cares.
struct NetworkState: Equatable {
    var online: Bool
    /// Mobile data or a personal hotspot (iOS's "expensive"); Wi-Fi and Ethernet are not.
    var metered: Bool

    static let unmetered = NetworkState(online: true, metered: false)
    static let cellular = NetworkState(online: true, metered: true)
    static let offline = NetworkState(online: false, metered: false)
}

/// Tells the download how the network is and when it changes. A protocol so tests need no real network.
@MainActor
protocol NetworkWatching: AnyObject {
    var current: NetworkState { get }
    var onChange: ((NetworkState) -> Void)? { get set }
}

/// What keeps the app running while the model comes down, and what shows it outside the app.
@MainActor
protocol DownloadSurroundings: AnyObject {
    /// Asks iOS for time to finish when the user leaves the app; `end` gives it back.
    func beginBackground()
    func endBackground()
    /// The Live Activity (`ModelDownloadActivityAttributes`).
    func activityStarted(total: UInt64)
    func activityProgress(downloaded: UInt64, total: UInt64)
    func activityEnded(failed: Bool)
    /// The app is in front (sounds are for then; a notification is for when it is not).
    var inFront: Bool { get }
}

/// Downloads Autopilot's model in this process (about 370 MB) through the core, which checks it against the hashes
/// built into it and keeps nothing that does not match (the Android app's `ModelDownloads` and its WorkManager job).
///
/// Wi-Fi only (the default) waits for Wi-Fi and starts by itself once it is there; `startNow` is the user agreeing to
/// use mobile data this once. While it runs, the app asks iOS for background time and shows a Live Activity; a
/// dropped connection is tried again up to three times, a file that fails its check is not.
@Observable
@MainActor
final class ModelDownloads {
    private(set) var job: DownloadJob = .idle
    private(set) var downloaded: UInt64 = 0
    private(set) var total: UInt64 = 0
    /// Why the last download failed, until the next one starts.
    private(set) var failure: String?
    /// The download waits for Wi-Fi (rather than for any network).
    private(set) var waitingForWifi = false
    private(set) var network: NetworkState

    /// Called after every download that ended, with the failure (nil when the model is installed).
    @ObservationIgnored var onFinished: ((String?) async -> Void)?

    @ObservationIgnored private let core: any ReinsCoreProtocol
    @ObservationIgnored private let feedback: Feedback
    @ObservationIgnored private let watcher: NetworkWatching
    @ObservationIgnored private let surroundings: DownloadSurroundings
    @ObservationIgnored private var wifiOnly = true
    @ObservationIgnored private var attempts = 0
    @ObservationIgnored private var task: Task<Void, Never>?
    @ObservationIgnored private var lastShown = Date.distantPast

    static let maxAttempts = 3
    /// How often the bytes on screen and in the Live Activity are redrawn at most.
    static let redraw: TimeInterval = 0.25

    init(core: any ReinsCoreProtocol, feedback: Feedback, network: NetworkWatching? = nil, surroundings: DownloadSurroundings? = nil) {
        self.core = core
        self.feedback = feedback
        self.watcher = network ?? PathNetwork()
        self.surroundings = surroundings ?? SystemDownloadSurroundings()
        self.network = watcher.current
        watcher.onChange = { [weak self] state in self?.networkChanged(state) }
    }

    /// Wi-Fi only is on and the phone is online, but only over mobile data: ask before using it.
    func needsMobileDataConsent(wifiOnly: Bool) -> Bool { wifiOnly && network.online && network.metered }

    /// Downloads when the network allows (Wi-Fi only: an unmetered one), else waits for it.
    func start(wifiOnly: Bool) {
        guard job == .idle else { return }
        self.wifiOnly = wifiOnly
        attempts = 0
        failure = nil
        if suits(network) { run() } else { wait() }
    }

    /// The user agreed to use mobile data (or whatever network there is) for this download.
    func startNow() {
        guard job != .running else { return }
        wifiOnly = false
        attempts = 0
        failure = nil
        if network.online { run() } else { wait() }
    }

    /// The Wi-Fi only switch changed: a waiting download waits for the new kind of network.
    func setWifiOnly(_ on: Bool) {
        wifiOnly = on
        if job == .waiting {
            waitingForWifi = on
            if suits(network) { run() }
        }
    }

    /// Calls off a download still waiting for its network (a running one finishes in the core either way).
    func cancel() {
        guard job == .waiting else { return }
        job = .idle
        waitingForWifi = false
    }

    private func suits(_ n: NetworkState) -> Bool { n.online && (!wifiOnly || !n.metered) }

    private func wait() {
        job = .waiting
        waitingForWifi = wifiOnly
    }

    private func networkChanged(_ state: NetworkState) {
        network = state
        if job == .waiting, suits(state) { run() }
    }

    private func run() {
        job = .running
        waitingForWifi = false
        attempts += 1
        downloaded = 0
        surroundings.beginBackground()
        surroundings.activityStarted(total: total)
        let relay = ProgressRelay { [weak self] done, all in
            Task { @MainActor in self?.progress(done, all) }
        }
        let core = core
        task = Task { [weak self] in
            let result: Result<ModelStatus, Error>
            do {
                result = .success(try await core.downloadModel(progress: relay))
            } catch {
                result = .failure(error)
            }
            await self?.finished(result)
        }
    }

    private func progress(_ done: UInt64, _ all: UInt64) {
        guard job == .running else { return }
        let now = Date()
        // A few redraws a second at most; the last one always shows.
        guard now.timeIntervalSince(lastShown) >= Self.redraw || done == all else { return }
        lastShown = now
        downloaded = done
        total = all
        surroundings.activityProgress(downloaded: done, total: all)
    }

    private func finished(_ result: Result<ModelStatus, Error>) async {
        task = nil
        switch result {
        case let .success(status):
            downloaded = status.sizeBytes > 0 ? status.sizeBytes : downloaded
            total = status.sizeBytes > 0 ? status.sizeBytes : total
            job = .idle
            surroundings.activityEnded(failed: false)
            surroundings.endBackground()
            if surroundings.inFront { feedback.play(.grantCreated) }
            await onFinished?(nil)
        case let .failure(error):
            // A dropped connection is worth another go; a file that fails its check is not.
            if case CoreError.Network = error, attempts < Self.maxAttempts {
                surroundings.endBackground()
                wait()
                try? await Task.sleep(for: .seconds(2))
                if job == .waiting, suits(network) { run() }
                return
            }
            let status = await core.modelStatus()
            let message = AutopilotText.modelError(status.error ?? error.userMessage)
            failure = message
            job = .idle
            surroundings.activityEnded(failed: true)
            surroundings.endBackground()
            if surroundings.inFront { feedback.play(.error) }
            await onFinished?(message)
        }
    }
}

/// The core's progress callback, from its own threads.
private final class ProgressRelay: DownloadProgress, @unchecked Sendable {
    private let handler: @Sendable (UInt64, UInt64) -> Void

    init(_ handler: @escaping @Sendable (UInt64, UInt64) -> Void) { self.handler = handler }

    func progress(downloaded: UInt64, total: UInt64) { handler(downloaded, total) }
}

// MARK: The real network and surroundings

/// `NWPathMonitor` on its own queue, reported on the main actor.
@MainActor
final class PathNetwork: NetworkWatching {
    private(set) var current: NetworkState
    var onChange: ((NetworkState) -> Void)?
    private let monitor = NWPathMonitor()

    init() {
        current = Self.state(monitor.currentPath)
        monitor.pathUpdateHandler = { [weak self] path in
            let state = Self.state(path)
            Task { @MainActor in
                guard let self, state != self.current else { return }
                self.current = state
                self.onChange?(state)
            }
        }
        monitor.start(queue: DispatchQueue(label: "com.reins2fa.app.network"))
    }

    nonisolated private static func state(_ path: NWPath) -> NetworkState {
        NetworkState(online: path.status == .satisfied, metered: path.isExpensive)
    }
}

/// Background time from UIKit, and the Live Activity through `ModelDownloadActivity` (the Lock Screen and the
/// Dynamic Island).
@MainActor
final class SystemDownloadSurroundings: DownloadSurroundings {
    private var backgroundTask: UIBackgroundTaskIdentifier = .invalid

    var inFront: Bool { UIApplication.shared.applicationState == .active }

    func beginBackground() {
        guard backgroundTask == .invalid else { return }
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "model-download") { [weak self] in
            // Out of time: iOS suspends the app; the core's download stops with it and is tried again (or fails)
            // when the app runs again.
            MainActor.assumeIsolated { self?.endBackground() }
        }
    }

    func endBackground() {
        guard backgroundTask != .invalid else { return }
        UIApplication.shared.endBackgroundTask(backgroundTask)
        backgroundTask = .invalid
    }

    func activityStarted(total: UInt64) { ModelDownloadActivity.start(total: Int64(total)) }

    func activityProgress(downloaded: UInt64, total: UInt64) {
        ModelDownloadActivity.update(downloaded: Int64(downloaded), total: Int64(total))
    }

    func activityEnded(failed: Bool) { ModelDownloadActivity.end(failed: failed) }
}
