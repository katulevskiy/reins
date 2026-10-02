import SwiftUI

@main
struct ReinsApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup {
            if let model = delegate.host.model {
                #if DEBUG
                if WidgetGallery.requested {
                    WidgetGallery().environment(model)
                } else {
                    RootView().environment(model)
                }
                #else
                RootView().environment(model)
                #endif
            } else {
                StartupFailureView(message: delegate.host.startupError ?? "Unknown error.")
            }
        }
    }
}
