import UIKit

/// One UIApplication background task, ended exactly once: by its expiration handler or by its owner, whichever comes
/// first (ending one twice, or never, gets the app ended). `started` takes the identifier `beginBackgroundTask` gave;
/// an `end` that came before it ends the task as soon as it is known.
final class BackgroundTaskToken: KeepAwakeToken, @unchecked Sendable {
    private let lock = NSLock()
    private let endTask: @Sendable (UIBackgroundTaskIdentifier) -> Void
    // Guarded by `lock`.
    private var id: UIBackgroundTaskIdentifier = .invalid
    private var ended = false

    init(end: @escaping @Sendable (UIBackgroundTaskIdentifier) -> Void) {
        endTask = end
    }

    /// The task the app began, ended by `UIApplication.shared`.
    @MainActor
    static func begin(_ name: String, expiring: (@MainActor @Sendable () -> Void)? = nil) -> BackgroundTaskToken {
        let app = UIApplication.shared
        let token = BackgroundTaskToken { app.endBackgroundTask($0) }
        token.started(app.beginBackgroundTask(withName: name) {
            expiring?()
            token.end()
        })
        return token
    }

    var identifier: UIBackgroundTaskIdentifier { lock.withLock { id } }

    func started(_ identifier: UIBackgroundTaskIdentifier) {
        let endNow: Bool = lock.withLock {
            if ended { return identifier != .invalid }
            id = identifier
            return false
        }
        if endNow { endTask(identifier) }
    }

    func end() {
        let ending: UIBackgroundTaskIdentifier? = lock.withLock {
            guard !ended else { return nil }
            ended = true
            defer { id = .invalid }
            return id == .invalid ? nil : id
        }
        if let ending { endTask(ending) }
    }
}

/// The app's `KeepAwake` for the store lock: a UIApplication background task, begun from whichever thread the core
/// locks on (`beginBackgroundTask` and `endBackgroundTask` are safe from any thread). In the background with less than
/// `minimum` seconds left, none is begun: the lock is refused rather than taken just before iOS suspends the app.
struct AppKeepAwake: KeepAwake {
    let app: UIApplication
    let minimum: TimeInterval

    @MainActor
    init(app: UIApplication = .shared, minimum: TimeInterval = 3) {
        self.app = app
        self.minimum = minimum
    }

    func begin(expiring: @escaping @Sendable () -> Void) -> KeepAwakeToken? {
        // In front this is the largest finite value.
        guard app.backgroundTimeRemaining >= minimum else { return nil }
        let app = app
        let token = BackgroundTaskToken { app.endBackgroundTask($0) }
        let id = app.beginBackgroundTask(withName: "Reins saves its data") { expiring() }
        guard id != .invalid else { return nil }
        token.started(id)
        return token
    }
}

/// Runs `work` and hands `finish` exactly one result: the work's, or `fallback` once `limit` passed or `expire()` was
/// called (the work is then cancelled; what it started in the core goes on under the store lock's own background
/// tasks). For a background wake, whose completion handler must be called before iOS's time is up.
@MainActor
final class Budgeted<Result: Sendable> {
    private let fallback: Result
    private let finish: @MainActor (Result) -> Void
    private var done = false
    private var work: Task<Void, Never>?
    private var timer: Task<Void, Never>?

    init(
        limit: Duration,
        fallback: Result,
        work: @escaping @MainActor () async -> Result,
        finish: @escaping @MainActor (Result) -> Void
    ) {
        self.fallback = fallback
        self.finish = finish
        self.work = Task { @MainActor in
            let result = await work()
            self.complete(result)
        }
        timer = Task { @MainActor in
            try? await Task.sleep(for: limit)
            if !Task.isCancelled { self.expire() }
        }
    }

    /// Whether `finish` was called.
    var finished: Bool { done }

    /// Time is up: `finish` gets the fallback now (unless it already had a result) and the work is cancelled.
    func expire() {
        work?.cancel()
        complete(fallback)
    }

    private func complete(_ result: Result) {
        guard !done else { return }
        done = true
        timer?.cancel()
        finish(result)
    }
}

/// How long a background wake (a push, a background refresh) may run before its completion is called: iOS allows
/// about 30 s, and ends the app (`0x8badf00d`) when the handler comes later.
enum BackgroundBudget {
    static let limit: Duration = .seconds(20)
}
