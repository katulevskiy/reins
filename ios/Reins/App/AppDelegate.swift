import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate {
    @MainActor lazy var host = AppHost()

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        #if DEBUG
        LaunchTiming.mark("launching")
        #endif
        // Before launching ends, though the store may still be opening: a tapped notification and the background
        // refresh arrive right after, and wait for it.
        #if DEBUG
        // `-noNotifications`: some simulators' notification service never answers, which blocks the main thread.
        if !ProcessInfo.processInfo.arguments.contains("-noNotifications") {
            NotificationRouter.shared.install(host: host)
        }
        #else
        NotificationRouter.shared.install(host: host)
        #endif
        BackgroundRefresh.register(host: host)
        application.registerForRemoteNotifications()
        #if DEBUG
        NotificationSamples.launch(host: host)
        #endif
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        host.whenReady { $0.setPushToken(deviceToken) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // No push on this device (simulator without APNs, no entitlement): the foreground long-poll still works.
    }

    /// A background push (`replaced`), or any push while the app runs.
    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        let host = host
        Task { @MainActor in
            completionHandler(await PushReceiver.handle(userInfo, host: host))
        }
    }
}
