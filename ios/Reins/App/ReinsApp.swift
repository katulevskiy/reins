import SwiftUI

@main
struct ReinsApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup {
            if let model = delegate.host.model {
                RootView().environment(model)
            } else {
                StartupFailureView(message: delegate.host.startupError ?? "Unknown error.")
            }
        }
    }
}
