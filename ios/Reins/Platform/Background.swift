import BackgroundTasks
import UIKit

/// A push that reaches the app process (the Android app's `PushHandler`): `replaced` and `hold` come as background
/// pushes; the others reach the app only when it runs (they are alerts the notification extension handles). The core fetches what
/// the push announced and tells `AppNotifier`, which notifies or chimes.
@MainActor
enum PushReceiver {
    /// Handles the push and calls `completion` exactly once: with what the work found, or `.failed` when the budget
    /// runs out or iOS takes the background time back first.
    static func receive(_ userInfo: [AnyHashable: Any], host: AppHost, completion: @escaping (UIBackgroundFetchResult) -> Void) {
        let app = UIApplication.shared
        let task = BackgroundTaskToken { app.endBackgroundTask($0) }
        let budget = Budgeted(limit: BackgroundBudget.limit, fallback: UIBackgroundFetchResult.failed, work: {
            await handle(userInfo, host: host)
        }, finish: { result in
            completion(result)
            task.end()
        })
        task.started(app.beginBackgroundTask(withName: "push") {
            budget.expire()
            task.end()
        })
    }

    static func handle(_ userInfo: [AnyHashable: Any], host: AppHost) async -> UIBackgroundFetchResult {
        guard let payload = PushPayload(userInfo: userInfo), let model = await host.ready() else { return .noData }
        // A push for another profile on this phone: not this core's (no fetch, no "replaced"). Its notification says
        // only that something waits, and a tap opens that profile.
        guard WorkProfiles.belongsHere(userInfo, remote: true) else { return .noData }
        // A held email is due (08 R10): sent from here, silently; only a "Still send?" notifies.
        if payload.silent { return await SendHolds.receive(model, notifier: host.notifier) }
        if payload.kind == "replaced" {
            model.markReplaced()
            host.notifier.deviceReplaced()
        }
        do {
            try await model.core.handlePush(kind: payload.kind, id: payload.id)
        } catch CoreError.Network {
            // Transient: the next push, refresh or the foreground poll picks it up.
            return .failed
        } catch let CoreError.Server(status, _) where status >= 500 {
            return .failed
        } catch {
            return .noData
        }
        if Task.isCancelled { return .newData }
        // In the background this reads only what waits (`AppModel.inBackground`); the rest when the app is opened.
        await model.refreshPending()
        if payload.circle != nil { await model.refreshCircles() }
        return .newData
    }
}

/// `com.reins2fa.app.refresh`: now and then while the app is in the background, a short sync (what push may have
/// missed, Autopilot's pass, grants that ran out) so the notifications, widgets and grant reminders stay right.
@MainActor
enum BackgroundRefresh {
    static let identifier = "com.reins2fa.app.refresh"
    /// iOS decides when; this is the earliest it may.
    static let interval: TimeInterval = 15 * 60

    /// Must run before the app finishes launching.
    static func register(host: AppHost) {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            MainActor.assumeIsolated {
                guard let task = task as? BGAppRefreshTask else {
                    task.setTaskCompleted(success: false)
                    return
                }
                run(task, host: host)
            }
        }
        NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { schedule() }
        }
    }

    static func schedule() {
        let request = BGAppRefreshTaskRequest(identifier: identifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        try? BGTaskScheduler.shared.submit(request)
    }

    private static func run(_ task: BGAppRefreshTask, host: AppHost) {
        schedule()
        let budget = Budgeted(limit: BackgroundBudget.limit, fallback: false, work: {
            await refresh(host: host)
        }, finish: { success in
            task.setTaskCompleted(success: success)
        })
        task.expirationHandler = {
            // Called on a queue of its own: the task is completed on the main queue, where it was registered (and
            // where `budget` completes it exactly once), and the work in flight is cancelled.
            DispatchQueue.main.async {
                MainActor.assumeIsolated { budget.expire() }
            }
        }
    }

    /// The refresh itself; true when it ran to the end.
    private static func refresh(host: AppHost) async -> Bool {
        // The refresh may be why the app launched: the store may still be opening.
        guard let model = await host.ready(), case .signedIn = model.session, !model.deviceReplaced else { return true }
        // Held sends that came due go first, so one woken for too late says "Still send?" (the sync releases them too,
        // silently).
        _ = try? await SendHolds.release(model, notifier: host.notifier)
        do {
            _ = try await model.core.sync(waitSecs: 0)
        } catch let CoreError.Server(status, _) where status == 403 {
            model.markReplaced()
        } catch {
            // Offline: next time.
        }
        if !Task.isCancelled { await model.refreshPending() }
        return !Task.isCancelled
    }
}

/// `com.reins2fa.app.model-update`: about once a day, on a network, Autopilot's model updates itself if a newer one
/// is due (`ModelAutoUpdater`, which still waits for Wi-Fi outside Low Data and Low Power Mode). A processing task, so
/// iOS gives it minutes rather than seconds; one cut short goes on from what arrived next time.
@MainActor
enum BackgroundModelUpdate {
    static let identifier = "com.reins2fa.app.model-update"
    /// iOS decides when; this is the earliest it may.
    static let interval: TimeInterval = 24 * 60 * 60
    /// How long a run waits for the session to be read when it is why the app launched.
    static let sessionWait: Duration = .seconds(10)

    /// Must run before the app finishes launching.
    static func register(host: AppHost) {
        BGTaskScheduler.shared.register(forTaskWithIdentifier: identifier, using: .main) { task in
            MainActor.assumeIsolated {
                guard let task = task as? BGProcessingTask else {
                    task.setTaskCompleted(success: false)
                    return
                }
                run(task, host: host)
            }
        }
        NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
            MainActor.assumeIsolated { schedule() }
        }
    }

    /// While an update waits (the last run found mobile data, Low Data or Low Power Mode, or was cut off), the next run
    /// may come this soon.
    static let retry: TimeInterval = 3 * 60 * 60

    /// When to ask for the next run, or nil to keep the one already asked for (`existing`: its earliest start, or
    /// `.distantPast` for one without). A waiting update wants one within `retry`, else one a day; a request already
    /// there is never pushed back, only brought forward.
    nonisolated static func nextRun(existing: Date?, waiting: Bool, now: Date) -> Date? {
        let wanted = now.addingTimeInterval(waiting ? retry : interval)
        guard let existing else { return wanted }
        return wanted < existing ? wanted : nil
    }

    /// Asks iOS for the next run if none is asked for, or for a sooner one while an update waits. Requires any network
    /// (iOS knows no "unmetered" here): the run itself checks for Wi-Fi.
    static func schedule(waiting: Bool = ModelAutoUpdater.persistedWaiting()) {
        BGTaskScheduler.shared.getPendingTaskRequests { requests in
            let mine = requests.first { $0.identifier == identifier }
            let existing = mine.map { $0.earliestBeginDate ?? .distantPast }
            Task { @MainActor in
                guard let at = nextRun(existing: existing, waiting: waiting, now: Date()) else { return }
                let request = BGProcessingTaskRequest(identifier: identifier)
                request.requiresNetworkConnectivity = true
                request.requiresExternalPower = false
                request.earliestBeginDate = at
                try? BGTaskScheduler.shared.submit(request)
            }
        }
    }

    private static func run(_ task: BGProcessingTask, host: AppHost) {
        // This run's request is used up: the next is asked for now, in case this one is cut short, and again once it
        // knows whether the update still waits.
        schedule()
        let done = Once()
        let work = Task { @MainActor in
            // The task may be why the app launched: the store may still be opening, the session still being read.
            if let model = await host.ready() {
                let end = ContinuousClock.now + sessionWait
                while model.session == .loading, ContinuousClock.now < end, !Task.isCancelled {
                    try? await Task.sleep(for: .milliseconds(100))
                }
                if case .signedIn = model.session, !Task.isCancelled {
                    await model.modelAutoUpdate.check(.background)
                    schedule(waiting: model.modelAutoUpdate.waiting)
                }
            }
            if done.claim() { task.setTaskCompleted(success: !Task.isCancelled) }
        }
        task.expirationHandler = {
            // Out of time: the download stops (the core keeps what arrived) and the next run comes sooner.
            Task { @MainActor in await host.model?.modelAutoUpdate.pause() }
            work.cancel()
            if done.claim() { task.setTaskCompleted(success: false) }
        }
    }

    /// The task is completed exactly once, by whichever comes first: the run or its expiration handler (which iOS
    /// calls on a queue of its own).
    final class Once: @unchecked Sendable {
        private let lock = NSLock()
        private var claimed = false

        func claim() -> Bool {
            lock.withLock {
                defer { claimed = true }
                return !claimed
            }
        }
    }
}
