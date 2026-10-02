import Foundation

/// Builds the process's one core and model at launch, before any scene: pushes and notification actions arrive in
/// the background with no UI, and need both.
@MainActor
final class AppHost {
    private(set) var model: AppModel?
    private(set) var startupError: String?
    let notifier = AppNotifier()

    /// Launched with `-demo`: an in-memory core with sample data, no server.
    static var isDemo: Bool { ProcessInfo.processInfo.arguments.contains("-demo") }

    init() {
        let feedback: Feedback = AppFeedback.make()
        if Self.isDemo, let demo = DemoCore.make() {
            let model = AppModel(core: demo, feedback: feedback, authenticator: TrustingAuthenticator(), demo: true)
            if ProcessInfo.processInfo.arguments.contains("-noPopup") { model.autoPopup = false }
            self.model = model
            notifier.model = model
        } else {
            do {
                let core = try CoreFactory.make(notifier: notifier)
                // Autopilot's forward pass; the core loads the model on first need.
                core.setModelRuntime(runtime: OnnxModelRuntime.forApp())
                let model = AppModel(core: core, feedback: feedback, authenticator: Authenticator())
                self.model = model
                notifier.model = model
            } catch {
                startupError = error.userMessage
            }
        }
        if let model {
            let notifier = notifier
            model.onModelDownloadFinished = { failure in notifier.modelDownloadFinished(failure: failure) }
            Task {
                await model.refreshSession()
                // `-open reins://...`: open a link at launch (simctl openurl stops at a confirmation prompt).
                let args = ProcessInfo.processInfo.arguments
                if let i = args.firstIndex(of: "-open"), i + 1 < args.count, let url = URL(string: args[i + 1]),
                   let link = DeepLink(url: url) {
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
}

/// Picks the feedback engine: Core Haptics and the preloaded sound set, following the user's Sounds & haptics settings.
enum AppFeedback {
    @MainActor
    static func make() -> Feedback { FeedbackEngine() }
}
