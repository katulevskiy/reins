import UIKit
import UserNotifications

final class AppDelegate: NSObject, UIApplicationDelegate {
    @MainActor lazy var host = AppHost()

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        _ = host
        application.registerForRemoteNotifications()
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        host.model?.setPushToken(deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        // No push on this device (simulator without APNs, no entitlement): the foreground long-poll still works.
    }
}
