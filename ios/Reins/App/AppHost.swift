import Foundation
import Observation
import UIKit

/// Builds the process's one core and model at launch, before any scene: pushes and notification actions arrive in
/// the background with no UI, and need both. The core opens off the main thread (SQLite, the data key through the
/// keychain, the sealed store), so the first frame does not wait for it: the window shows the loading state until
/// then, and what arrives first (a push, a notification action, a background refresh, a link) waits for `ready()`.
@MainActor
@Observable
final class AppHost {
    private(set) var model: AppModel?
    private(set) var startupError: String?
    @ObservationIgnored let notifier = AppNotifier()
    @ObservationIgnored private let arrival = Arrival<AppModel>()
    @ObservationIgnored private var modelRuntime: OnnxModelRuntime?
    @ObservationIgnored private var backgroundObservers: [NSObjectProtocol] = []

    /// Launched with `-demo`: an in-memory core with sample data, no server.
    static var isDemo: Bool { ProcessInfo.processInfo.arguments.contains("-demo") }

    init() {
        #if DEBUG
        // UI tests: start from the default sounds and haptics, whatever an earlier (interrupted) run left saved.
        if ProcessInfo.processInfo.arguments.contains("-resetFeedback") { FeedbackSettings().save() }
        #endif
        // The wallpaper's earlier keys, read once: what they kept becomes the theme's choice, looking as it did.
        WallpaperChoice.migrate()
        let feedback: Feedback = AppFeedback.make()
        if Self.isDemo, let demo = DemoCore.make() {
            // In memory: nothing to wait for.
            let model = AppModel(core: demo, feedback: feedback, authenticator: TrustingAuthenticator(), demo: true)
            if ProcessInfo.processInfo.arguments.contains("-noPopup") { model.autoPopup = false }
            settle(model)
        } else {
            let notifier = notifier
            // Before the core opens the store: the app holds a background task while it holds the store's lock.
            let keepAwake = AppKeepAwake()
            Task {
                let opened = await Task.detached(priority: .userInitiated) {
                    Result {
                        StoreLocking.install(keepAwake, waitMillis: CoreFactory.appLockWaitMillis)
                        return try CoreFactory.make(notifier: notifier)
                    }
                }.value
                switch opened {
                case let .success(core):
                    // Autopilot's forward pass; the core loads the model on first need.
                    let runtime = OnnxModelRuntime.forApp()
                    core.setModelRuntime(runtime: runtime)
                    modelRuntime = runtime
                    settle(AppModel(core: core, feedback: feedback, authenticator: Authenticator()))
                case let .failure(error):
                    startupError = error.userMessage
                    settle(nil)
                }
            }
        }
    }

    /// The model once the core is open; nil when it could not open.
    func ready() async -> AppModel? {
        await arrival.wait()
    }

    /// Runs `body` with the model as soon as the core is open (now, if it is), before the session is first read.
    func whenReady(_ body: @escaping (AppModel) -> Void) {
        arrival.whenSettled(body)
    }

    /// A link opened while the store may still be opening: it waits for the model rather than being dropped.
    func open(_ url: URL) {
        guard let link = DeepLink.opened(url) else { return }
        Task {
            guard let model = await ready() else { return }
            await model.handle(link)
        }
    }

    private func settle(_ model: AppModel?) {
        self.model = model
        #if DEBUG
        LaunchTiming.mark("core open")
        #endif
        if let model {
            notifier.model = model
            let notifier = notifier
            model.onModelDownloadFinished = { failure in notifier.modelDownloadFinished(failure: failure) }
            // Widget buttons, controls and Siri reach the model through these (an intent may be why we launched).
            IntentBridge.model = model
            LiveActivityController.shared.install(model: model)
        }
        if let model { followBackground(model) }
        arrival.settle(model)
        guard let model else { return }
        Task {
            await model.refreshSession()
            // A newer model on Wi-Fi, by itself (it waits for Wi-Fi otherwise); nothing below waits for it.
            Task { await model.modelAutoUpdate.check(.launch) }
            // `-open reins://...`: open a link at launch (simctl openurl stops at a confirmation prompt).
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-open"), i + 1 < args.count, let url = URL(string: args[i + 1]),
               let link = DeepLink.opened(url) {
                await model.handle(link)
            }
            #if DEBUG
            // `-show <place>`: open a tab or a page at launch (DebugShow).
            if let i = args.firstIndex(of: "-show"), i + 1 < args.count {
                DebugShow.open(args[i + 1], in: model)
            }
            #endif
        }
    }
}

extension AppHost {
    /// Keeps the model, the core and the ONNX runtime told whether the app is in the background, from launch (a push or
    /// a background refresh may be why it launched) on.
    fileprivate func followBackground(_ model: AppModel) {
        let runtime = modelRuntime
        let apply: @MainActor @Sendable (Bool) -> Void = { background in
            model.setInBackground(background)
            runtime?.setBackground(background)
        }
        apply(UIApplication.shared.applicationState == .background)
        let center = NotificationCenter.default
        backgroundObservers = [
            center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { apply(true) }
            },
            center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) { _ in
                MainActor.assumeIsolated { apply(false) }
            },
        ]
    }
}

/// Picks the feedback engine: Core Haptics and the preloaded sound set, following the user's Sounds & haptics settings.
enum AppFeedback {
    @MainActor
    static func make() -> Feedback { FeedbackEngine() }
}
