import Foundation
import Observation

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

    /// Launched with `-demo`: an in-memory core with sample data, no server.
    static var isDemo: Bool { ProcessInfo.processInfo.arguments.contains("-demo") }

    init() {
        let feedback: Feedback = AppFeedback.make()
        if Self.isDemo, let demo = DemoCore.make() {
            // In memory: nothing to wait for.
            let model = AppModel(core: demo, feedback: feedback, authenticator: TrustingAuthenticator(), demo: true)
            if ProcessInfo.processInfo.arguments.contains("-noPopup") { model.autoPopup = false }
            settle(model)
        } else {
            let notifier = notifier
            Task {
                let opened = await Task.detached(priority: .userInitiated) {
                    Result { try CoreFactory.make(notifier: notifier) }
                }.value
                switch opened {
                case let .success(core):
                    // Autopilot's forward pass; the core loads the model on first need.
                    core.setModelRuntime(runtime: OnnxModelRuntime.forApp())
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
        arrival.settle(model)
        guard let model else { return }
        Task {
            await model.refreshSession()
            // `-open reins://...`: open a link at launch (simctl openurl stops at a confirmation prompt).
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-open"), i + 1 < args.count, let url = URL(string: args[i + 1]),
               let link = DeepLink.opened(url) {
                await model.handle(link)
            }
            #if DEBUG
            // `-show sounds`: open Settings > Sounds & haptics at launch (screenshots, manual checks).
            if let i = args.firstIndex(of: "-show"), i + 1 < args.count, args[i + 1] == "sounds" {
                model.show(.sounds, in: .settings)
            }
            #endif
        }
    }
}

/// Picks the feedback engine: Core Haptics and the preloaded sound set, following the user's Sounds & haptics settings.
enum AppFeedback {
    @MainActor
    static func make() -> Feedback { FeedbackEngine() }
}
