import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate {
    @MainActor lazy var host = AppHost()

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        #if DEBUG
        LaunchTiming.mark("launching")
        #endif
        Self.styleNavigationBars()
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

extension AppDelegate {
    /// A page's large title is set in the serif, as Signal's titles are; the small one in the bar in the sans.
    static func styleNavigationBars() {
        let bar = UINavigationBar.appearance()
        if let serif = UIFont(name: RFont.serifName(false), size: 40) {
            bar.largeTitleTextAttributes = [.font: UIFontMetrics(forTextStyle: .largeTitle).scaledFont(for: serif)]
        }
        if let sans = UIFont(name: RFont.sansFamily.name(.semibold), size: 16) {
            bar.titleTextAttributes = [.font: UIFontMetrics(forTextStyle: .headline).scaledFont(for: sans)]
        }
    }
}
