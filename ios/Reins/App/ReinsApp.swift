import SwiftUI

@main
struct ReinsApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup {
            #if DEBUG
            let _ = LaunchTiming.mark("first body")
            #endif
            Group {
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
                } else if let error = delegate.host.startupError {
                    StartupFailureView(message: error)
                } else {
                    // The store is still opening.
                    LoadingView()
                }
            }
            #if DEBUG
            .onAppear { LaunchTiming.mark("first appear") }
            #endif
            // Links reach the model whatever shows: one opened while the store is still opening waits for it, and a
            // pairing code opened while signed out waits for the sign-in.
            .onOpenURL { url in delegate.host.open(url) }
            // A universal link (a computer's QR code, https://<server>/pair?code=...): only its code is used.
            .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
                guard let url = activity.webpageURL else { return }
                delegate.host.open(url)
            }
        }
    }
}
