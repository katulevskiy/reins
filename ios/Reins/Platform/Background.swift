import BackgroundTasks
import UIKit

/// A push that reaches the app process (the Android app's `PushHandler`): `replaced` comes as a background push; the
/// others reach the app only when it runs (they are alerts the notification extension handles). The core fetches what
/// the push announced and tells `AppNotifier`, which notifies or chimes.
@MainActor
enum PushReceiver {
    static func handle(_ userInfo: [AnyHashable: Any], host: AppHost) async -> UIBackgroundFetchResult {
        guard let payload = PushPayload(userInfo: userInfo), let model = host.model else { return .noData }
        if payload.kind == "replaced" {
            model.markReplaced()
            host.notifier.deviceReplaced()
        }
        do {
            try await model.core.handlePush(kind: payload.kind, id: payload.id)
            await model.refreshPending()
            return .newData
        } catch CoreError.Network {
            // Transient: the next push, refresh or the foreground poll picks it up.
            return .failed
        } catch let CoreError.Server(status, _) where status >= 500 {
            return .failed
        } catch {
            return .noData
        }
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
        let done = Once()
        let work = Task { @MainActor in
            if let model = host.model, case .signedIn = model.session, !model.deviceReplaced {
                do {
                    _ = try await model.core.sync(waitSecs: 0)
                } catch let CoreError.Server(status, _) where status == 403 {
                    model.markReplaced()
                } catch {
                    // Offline: next time.
                }
                await model.refreshPending()
            }
            if done.claim() { task.setTaskCompleted(success: !Task.isCancelled) }
        }
        task.expirationHandler = {
            work.cancel()
            if done.claim() { task.setTaskCompleted(success: false) }
        }
    }

    /// The task is completed exactly once, by whichever comes first.
    private final class Once: @unchecked Sendable {
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
