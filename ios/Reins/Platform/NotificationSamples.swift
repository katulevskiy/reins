#if DEBUG
import ActivityKit
import UIKit
import UserNotifications

/// Debug launch arguments for checking notifications on a simulator (there is no APNs session there):
/// - `-resetNotifications`: starts clean, as UI tests need on a reused simulator: removes delivered and scheduled
///   notifications and ends the Live Activities an earlier run left.
/// - `-askNotifications`: asks for permission at launch, signed in or not.
/// - `-notifierSamples`: a few seconds after the app goes to the background, hands `AppNotifier` the newest waiting item
///   and an automatic decision, as the core would during a background wake.
/// - `-logNotifications`: logs the scheduled and delivered notifications (grant reminders) every few seconds.
@MainActor
enum NotificationSamples {
    static func launch(host: AppHost) {
        let args = ProcessInfo.processInfo.arguments
        if args.contains("-resetNotifications") {
            let center = UNUserNotificationCenter.current()
            center.removeAllDeliveredNotifications()
            center.removeAllPendingNotificationRequests()
            Task {
                for a in Activity<ApprovalActivityAttributes>.activities { await a.end(nil, dismissalPolicy: .immediate) }
                for a in Activity<BypassActivityAttributes>.activities { await a.end(nil, dismissalPolicy: .immediate) }
                for a in Activity<ModelDownloadActivityAttributes>.activities { await a.end(nil, dismissalPolicy: .immediate) }
            }
        }
        if args.contains("-askNotifications") {
            Task { await NotificationRouter.shared.requestPermission() }
        }
        if args.contains("-notifierSamples") {
            NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { post(host: host) }
            }
        }
        if args.contains("-logNotifications") {
            Task {
                while true {
                    try? await Task.sleep(for: .seconds(3))
                    let center = UNUserNotificationCenter.current()
                    let scheduled = await center.pendingNotificationRequests().map { r in
                        let at = (r.trigger as? UNTimeIntervalNotificationTrigger)?.nextTriggerDate().map { "\(Int($0.timeIntervalSinceNow))s" } ?? "-"
                        return "\(r.identifier) in \(at): \(r.content.title) / \(r.content.body)"
                    }
                    let delivered = await center.deliveredNotifications().map { "\($0.request.identifier): \($0.request.content.title)" }
                    NSLog("[notifications] scheduled: \(scheduled) delivered: \(delivered)")
                }
            }
        }
    }

    private static func post(host: AppHost) {
        let app = UIApplication.shared
        var task = UIBackgroundTaskIdentifier.invalid
        task = app.beginBackgroundTask(withName: "notifier-samples") {
            app.endBackgroundTask(task)
            task = .invalid
        }
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(3))
            if let model = await host.ready(), let item = try? await model.core.pending().first {
                host.notifier.itemPending(item: item)
                host.notifier.autoDecided(decision: AutoDecisionView(
                    requestId: "sample-auto",
                    kind: .request,
                    connectionId: item.connectionId,
                    connectionLabel: item.connectionLabel,
                    title: "Push to a branch · dkat/reins",
                    verdict: .approve,
                    decidedBy: "autopilot",
                    pApprove: 0.97,
                    confidence: 0.97,
                    activityId: 14
                ))
            }
            try? await Task.sleep(for: .seconds(4))
            if task != .invalid { app.endBackgroundTask(task) }
        }
    }
}
#endif
