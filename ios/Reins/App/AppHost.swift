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

    /// The host of this process, for switching profiles in place (`ProfileSwitch`).
    static weak var current: AppHost?

    /// The profile the window shows: the root view is keyed on it, so a switch starts the UI afresh.
    private(set) var profileId = ""
    /// A profile switch is under way: the window says so until the other profile's core is open.
    private(set) var switching = false
    @ObservationIgnored private let feedback: Feedback
    @ObservationIgnored private var keepAwake: AppKeepAwake?

    init() {
        #if DEBUG
        // UI tests: start from the default sounds and haptics, whatever an earlier (interrupted) run left saved.
        if ProcessInfo.processInfo.arguments.contains("-resetFeedback") { FeedbackSettings().save() }
        #endif
        // The wallpaper's earlier keys, read once: what they kept becomes the theme's choice, looking as it did.
        WallpaperChoice.migrate()
        feedback = AppFeedback.make()
        Self.current = self
        // The demo is in memory: the first frame already has its model.
        if !openDemo() { Task { await openActiveProfile() } }
    }

    /// `-demo`: settles the in-memory core's model at once; false when this is not the demo.
    private func openDemo() -> Bool {
        guard Self.isDemo, let demo = DemoCore.make() else { return false }
        let model = AppModel(core: demo, feedback: feedback, authenticator: TrustingAuthenticator(), demo: true)
        if ProcessInfo.processInfo.arguments.contains("-noPopup") { model.autoPopup = false }
        settle(model)
        return true
    }

    /// Opens the active profile's core off the main thread (the demo's at once) and settles the model on it.
    private func openActiveProfile() async {
        profileId = CoreFactory.activeProfile()?.id ?? ""
        if openDemo() { return }
        let notifier = notifier
        // Everything this notifier posts from now on names this profile.
        notifier.profile = WorkProfiles.active
        // Before the core opens the store: the app holds a background task while it holds the store's lock.
        let keepAwake = keepAwake ?? AppKeepAwake()
        self.keepAwake = keepAwake
        let opened = await Task.detached(priority: .userInitiated) {
            Result {
                StoreLocking.install(keepAwake, waitMillis: CoreFactory.appLockWaitMillis)
                return try CoreFactory.make(notifier: notifier)
            }
        }.value
        switch opened {
        case let .success(core):
            // Autopilot's forward pass; the core loads the model on first need.
            let runtime = modelRuntime ?? OnnxModelRuntime.forApp()
            core.setModelRuntime(runtime: runtime)
            modelRuntime = runtime
            settle(AppModel(core: core, feedback: feedback, authenticator: Authenticator()))
            // This profile's push tag, so a push for another profile on this phone never reaches this core.
            let profile = WorkProfiles.active.id
            Task.detached { if let tag = try? await core.pushTag() { WorkProfiles.rememberPushTag(tag, profileId: profile) } }
        case let .failure(error):
            startupError = error.userMessage
            settle(nil)
        }
    }

    /// Moves the whole app to the newly active profile without quitting (iOS never relaunches an app that exits, and
    /// App Review treats a quit as a crash): the current account stops (its long poll, Live Activities, notification
    /// sweep), the window shows "Switching…", the active profile's core opens and the UI starts again on it.
    func switchProfile() async {
        guard !switching else { return }
        switching = true
        defer { switching = false }
        if let old = model {
            old.shutDownForSwitch()
            LiveActivityController.shared.uninstall()
        }
        for observer in backgroundObservers { NotificationCenter.default.removeObserver(observer) }
        backgroundObservers = []
        IntentBridge.model = nil
        notifier.model = nil
        model = nil
        startupError = nil
        arrival.reopen()
        // The old model (and its core, which holds the old profile's store) goes once nothing holds it: let the
        // main actor run what was queued for it before the new one opens.
        await Task.yield()
        await openActiveProfile()
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
