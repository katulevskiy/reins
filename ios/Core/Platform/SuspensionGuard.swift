import Foundation
import os

/// Something that keeps this process from being suspended while held: a UIApplication background task in the app, an
/// expiring activity in the notification extension (which has no UIApplication).
protocol KeepAwake: Sendable {
    /// Starts holding. `expiring` is called, on any thread, when the system is about to suspend the process anyway;
    /// the holder must then end the token promptly. Nil when it cannot be had (the process is being suspended).
    func begin(expiring: @escaping @Sendable () -> Void) -> KeepAwakeToken?
}

/// One hold of a `KeepAwake`; `end` may be called any number of times and ends it once.
protocol KeepAwakeToken: AnyObject, Sendable {
    func end()
}

/// The core's `StoreLockGuard`. The store lives in the App Group container, shared by the app and the notification
/// extension, and is written under a file lock there; iOS ends a process that is suspended while it holds such a lock
/// (`0xdead10cc`). So while the core holds it, this holds a `KeepAwake`; when that cannot be had, or the system is
/// already taking the process's time back, the lock is refused and the core's call fails instead.
///
/// One hold covers a burst of locks: it ends `grace` seconds after the last one is given back. When the system says time
/// is up, no new lock is taken, and the hold ends as soon as the locks in progress (milliseconds each) are given back:
/// never while one is held, or iOS would suspend the app holding it. `expire` waits for them at most `drain` seconds;
/// one still held then ends the hold from `unlocked`. A section that never ends gets the app ended for its overdue
/// background task instead, the lesser evil, and is logged.
final class SuspensionGuard: StoreLockGuard, @unchecked Sendable {
    private let keepAwake: KeepAwake
    private let grace: TimeInterval
    private let drain: TimeInterval
    private let ends = DispatchQueue(label: "com.reins2fa.suspension-guard")
    private static let log = Logger(subsystem: "com.reins2fa.app", category: "store-lock")

    private let condition = NSCondition()
    // Guarded by `condition`.
    private var held = 0
    private var token: KeepAwakeToken?
    private var beginning = false
    private var expiring = false
    /// Bumped by every expiry: a hold begun while one ran is ended at once.
    private var expiries = 0
    /// Bumped by every lock and unlock: an end scheduled before them is stale.
    private var generation = 0

    init(keepAwake: KeepAwake, grace: TimeInterval = 2, drain: TimeInterval = 1) {
        self.keepAwake = keepAwake
        self.grace = grace
        self.drain = drain
    }

    /// Locks held now (for tests).
    var holding: Int { condition.withLock { held } }

    /// Whether a hold is open now (for tests).
    var awake: Bool { condition.withLock { token != nil } }

    func locking() -> Bool {
        condition.lock()
        while true {
            if expiring {
                condition.unlock()
                return false
            }
            if token != nil {
                held += 1
                generation += 1
                condition.unlock()
                return true
            }
            if !beginning { break }
            condition.wait()
        }
        beginning = true
        let before = expiries
        condition.unlock()

        // Outside the lock: an expiring callback may come on another thread while this waits.
        let started = keepAwake.begin { [weak self] in self?.expire() }

        condition.lock()
        beginning = false
        condition.broadcast()
        guard let started else {
            condition.unlock()
            return false
        }
        if expiring || expiries != before {
            condition.unlock()
            started.end()
            return false
        }
        token = started
        held += 1
        generation += 1
        condition.unlock()
        return true
    }

    func unlocked() {
        condition.lock()
        held = max(0, held - 1)
        generation += 1
        let idle = held == 0
        let current = generation
        var endingNow: KeepAwakeToken?
        if idle {
            condition.broadcast()
            if expiring {
                // Time ran out while this was held: the hold ends now that nothing is.
                endingNow = token
                token = nil
                expiring = false
            }
        }
        condition.unlock()
        if let endingNow {
            endingNow.end()
            return
        }
        guard idle else { return }
        ends.asyncAfter(deadline: .now() + grace) { [weak self] in self?.endIfIdle(current) }
    }

    /// The system is taking the time back: no new locks; the hold ends once the ones in progress are given back.
    func expire() {
        condition.lock()
        expiring = true
        expiries += 1
        let deadline = Date().addingTimeInterval(drain)
        while held > 0, condition.wait(until: deadline) {}
        guard held == 0 else {
            let stuck = held
            condition.unlock()
            // Never ended with the lock held: the last `unlocked` ends it.
            Self.log.error("background time is up with \(stuck, privacy: .public) store lock(s) held; the task ends when they are given back")
            return
        }
        let ending = token
        token = nil
        generation += 1
        expiring = false
        condition.broadcast()
        condition.unlock()
        ending?.end()
    }

    private func endIfIdle(_ scheduled: Int) {
        condition.lock()
        guard scheduled == generation, held == 0, let ending = token else {
            condition.unlock()
            return
        }
        token = nil
        condition.unlock()
        ending.end()
    }
}

/// The notification extension's `KeepAwake`: `ProcessInfo.performExpiringActivity`, whose block is kept running (and
/// the process with it) until the token ends.
struct ExpiringActivityKeepAwake: KeepAwake {
    /// How long `begin` waits for the system to start the activity.
    var timeout: TimeInterval = 1

    func begin(expiring: @escaping @Sendable () -> Void) -> KeepAwakeToken? {
        let token = ActivityToken()
        let answered = DispatchSemaphore(value: 0)
        ProcessInfo.processInfo.performExpiringActivity(withReason: "Reins saves its data") { expired in
            if expired {
                // Never started: refused. Started: time is up; the guard ends the token, which lets the first call go.
                if token.decide(false) { answered.signal() } else if !token.isEnded { expiring() }
                return
            }
            guard token.decide(true) else { return }
            answered.signal()
            token.hold()
        }
        if answered.wait(timeout: .now() + timeout) == .timedOut {
            _ = token.decide(false)
        }
        return token.granted ? token : nil
    }

    private final class ActivityToken: KeepAwakeToken, @unchecked Sendable {
        private let lock = NSLock()
        private let released = DispatchSemaphore(value: 0)
        private var decided: Bool?
        private var ended = false

        /// Records whether the activity started; false when it was already decided.
        func decide(_ started: Bool) -> Bool {
            lock.withLock {
                guard decided == nil else { return false }
                decided = started
                return true
            }
        }

        var granted: Bool { lock.withLock { decided == true } }

        var isEnded: Bool { lock.withLock { ended } }

        /// Keeps the activity's block (and so the activity) running until `end`.
        func hold() { released.wait() }

        func end() {
            let first = lock.withLock {
                defer { ended = true }
                return !ended
            }
            if first { released.signal() }
        }
    }
}

/// Installs the process's `SuspensionGuard` in the core, once, before the first `ReinsCore`.
enum StoreLocking {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var installed = false

    /// `waitMillis`: how long a lock may wait for the other process before the call fails as busy.
    static func install(_ keepAwake: KeepAwake, waitMillis: UInt32) {
        lock.withLock {
            guard !installed else { return }
            installed = true
            installStoreLockGuard(guard: SuspensionGuard(keepAwake: keepAwake), waitMs: waitMillis)
        }
    }
}
