import UIKit
import UserNotifications

/// Everything about notifications in the app process: the categories and their actions, asking for permission once the
/// user signed in, keeping delivered notifications in step with what waits, what shows while the app is in front
/// (`willPresent`), and where a tap or an action goes (`didReceive`).
///
/// Delegate callbacks use the completion-handler forms and finish on the main queue: the async variants complete off
/// the main thread, which UIKit asserts on.
@MainActor
final class NotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationRouter()

    private weak var model: AppModel?
    private var notifier: AppNotifier?
    private let center = UNUserNotificationCenter.current()

    /// At launch, before it finishes (a tap that launched the app arrives right after).
    func install(model: AppModel?, notifier: AppNotifier) {
        self.model = model
        self.notifier = notifier
        center.delegate = self
        center.setNotificationCategories(Self.categories())
        guard let model else { return }
        let grants = model.onGrantsChanged
        model.onGrantsChanged = { list in
            grants?(list)
            GrantReminders.sync(list)
        }
        let pending = model.onPendingChanged
        model.onPendingChanged = { [weak self] list in
            pending?(list)
            self?.dropStale(keeping: Set(list.map(\.id)))
        }
        let signedIn = model.onSignedIn
        // The `-demo` build asks only when told to (`-askNotifications`): the prompt would cover every screenshot.
        let ask = !model.demo
        model.onSignedIn = { [weak self] in
            signedIn?()
            if ask { Task { await self?.requestPermission() } }
        }
    }

    // MARK: Categories

    nonisolated static func categories() -> Set<UNNotificationCategory> {
        func category(_ c: NotificationCategory, _ actions: [UNNotificationAction] = []) -> UNNotificationCategory {
            UNNotificationCategory(
                identifier: c.rawValue,
                actions: actions,
                intentIdentifiers: [],
                hiddenPreviewsBodyPlaceholder: c.hiddenPlaceholder,
                options: []
            )
        }
        // Deny needs an unlocked phone but no Face ID (only approving does, as on Android) and does not open the app.
        let deny = UNNotificationAction(
            identifier: NotificationAction.deny,
            title: "Deny",
            options: [.destructive, .authenticationRequired],
            icon: UNNotificationActionIcon(systemImageName: "xmark")
        )
        // "Report" opens the entry, where "This was wrong" teaches Autopilot (an approval cannot be taken back).
        let report = UNNotificationAction(
            identifier: NotificationAction.report,
            title: "Report",
            options: [.foreground],
            icon: UNNotificationActionIcon(systemImageName: "flag")
        )
        return [
            category(.request, [deny]),
            category(.pairing),
            category(.blob),
            category(.autopilot, [report]),
            category(.grant),
            category(.status),
        ]
    }

    // MARK: Permission

    /// Asks once, after sign-in (the moment notifications make sense); later it is the Settings app's call.
    @discardableResult
    func requestPermission() async -> Bool {
        let status = await center.notificationSettings().authorizationStatus
        let granted: Bool
        if status == .notDetermined {
            granted = (try? await center.requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        } else {
            granted = [.authorized, .provisional, .ephemeral].contains(status)
        }
        if granted { UIApplication.shared.registerForRemoteNotifications() }
        return granted
    }

    // MARK: Keeping in step

    /// Takes away item notifications whose item no longer waits (a safety net under `itemResolved`). A push whose
    /// item the extension could not read yet is left alone for a minute, until the app fetched it.
    private func dropStale(keeping waiting: Set<String>) {
        let items = Set([NotificationCategory.request, .pairing, .blob].map(\.rawValue))
        center.getDeliveredNotifications { delivered in
            let cutoff = Date().addingTimeInterval(-60)
            let stale = delivered.filter { n in
                guard items.contains(n.request.content.categoryIdentifier), n.date < cutoff else { return false }
                let id = n.request.content.userInfo[NotificationKey.id] as? String ?? n.request.identifier
                return !waiting.contains(id)
            }.map(\.request.identifier)
            if !stale.isEmpty { UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: stale) }
        }
    }

    // MARK: Delivery

    /// In front, items pop up in the app with the in-app chime instead of a banner (the push still makes the app
    /// fetch the item at once); grant reminders and Autopilot's decisions say nothing the screen does not; status
    /// changes show as a quiet banner.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        let category = NotificationCategory(rawValue: notification.request.content.categoryIdentifier)
        let payload = notification.request.trigger is UNPushNotificationTrigger ? PushPayload(userInfo: notification.request.content.userInfo) : nil
        DispatchQueue.main.async {
            MainActor.assumeIsolated {
                switch category {
                case .request, .pairing, .blob:
                    if let payload, payload.itemKind != nil, let model = self.model {
                        Task {
                            try? await model.core.handlePush(kind: payload.kind, id: payload.id)
                            await model.refreshPending()
                        }
                    }
                    completionHandler([])
                case .status:
                    completionHandler([.banner, .list])
                case .autopilot, .grant, nil:
                    completionHandler([])
                }
            }
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let action = response.actionIdentifier
        let request = response.notification.request
        let info = request.content.userInfo
        let remote = request.trigger is UNPushNotificationTrigger
        let link = (info[NotificationKey.link] as? String).flatMap(URL.init(string:)).flatMap(DeepLink.init(url:))
        let payload = PushPayload(userInfo: info)
        Task { @MainActor in
            await self.respond(action: action, link: link ?? payload?.link, payload: payload, remote: remote)
            completionHandler()
        }
    }

    private func respond(action: String, link: DeepLink?, payload: PushPayload?, remote: Bool) async {
        guard let model else { return }
        switch action {
        case UNNotificationDismissActionIdentifier:
            return
        case NotificationAction.deny:
            guard let payload, payload.itemKind == .request else { return }
            await deny(payload, remote: remote, model: model)
        default:
            guard let link else { return }
            await waitForSession(model)
            // The extension may have left the item for the app to fetch.
            if remote, let payload, payload.itemKind != nil {
                try? await model.core.handlePush(kind: payload.kind, id: payload.id)
            }
            await model.handle(link)
        }
    }

    /// "Deny" from the notification: the app runs in the background for it and plays nothing.
    private func deny(_ payload: PushPayload, remote: Bool, model: AppModel) async {
        let app = UIApplication.shared
        var task = UIBackgroundTaskIdentifier.invalid
        task = app.beginBackgroundTask(withName: "deny") {
            app.endBackgroundTask(task)
            task = .invalid
        }
        defer { if task != .invalid { app.endBackgroundTask(task) } }
        await waitForSession(model)
        do {
            if remote { try? await model.core.handlePush(kind: payload.kind, id: payload.id) }
            try await model.core.deny(requestId: payload.id)
        } catch CoreError.NotFound {
            // Already answered or expired: nothing left to deny.
        } catch {
            notifier?.denyFailed(error.userMessage)
        }
        center.removeDeliveredNotifications(withIdentifiers: [payload.id])
        await model.refreshPending()
    }

    /// A tap that launched the app comes before the store is open.
    private func waitForSession(_ model: AppModel) async {
        for _ in 0..<100 {
            if case .loading = model.session {
                try? await Task.sleep(for: .milliseconds(100))
            } else {
                return
            }
        }
    }
}
