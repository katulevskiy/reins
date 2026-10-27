import UserNotifications

/// Rewrites a push ("Reins", "Something is waiting for you") into what actually waits, like the Android app's
/// notifications: it opens the same store as the app (App Group container, shared keychain), lets the core fetch the
/// item the push names, and words the notification from what the core reports. The automatic deciders run first: a
/// bypass, Lockdown, Overnight or Autopilot decision comes out as its quiet notification, and only what then waits for
/// the owner is an ask (`PushOutcome`). Sounds follow the in-app switches.
///
/// Anything that cannot be done in time, or safely, delivers the generic text with the right sound and no action
/// (it may be something a mode answers): the app in front (it handles the push itself), no store or no session, a
/// locked keychain, a network failure, the app holding the store throughout. When Autopilot needs its model (which only
/// the app can run; this process has a few MB of memory), the item is parked without Autopilot's pass and the app judges
/// it when it next runs: the notification says so, and is never an ask for something Autopilot may then decide.
final class NotificationService: UNNotificationServiceExtension {
    private let lock = NSLock()
    private var deliver: ((UNNotificationContent) -> Void)?
    private var fallback: UNNotificationContent?
    private var work: Task<Void, Never>?

    /// The system allows about 30 s; the generic text goes out well before.
    private static let deadline: Duration = .seconds(22)
    /// While the app holds the store (it is often handling this very push, woken by it), tried again this often, up to
    /// this many times: the item is then read as the app left it (decided, or asked by the app itself) instead of
    /// falling back to the generic text.
    private static let busyPause: Duration = .seconds(1)
    private static let busyTries = 12

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let settings = FeedbackSettings.load()
        let payload = PushPayload(userInfo: request.content.userInfo)
        let generic = NotificationContent.generic(request.content, payload: payload, settings: settings)
        lock.withLock {
            deliver = contentHandler
            fallback = generic
        }
        // The active profile (it names the store opened below), and whether the push is its own: a push for another
        // profile on this phone keeps its generic text and never reaches this profile's core.
        _ = CoreFactory.activeProfile()
        let ours = WorkProfiles.belongsHere(request.content.userInfo, remote: true)
        guard ours, let payload, payload.itemKind != nil, !AppPresence.inFront(), CoreFactory.storeExists else {
            finish(generic)
            return
        }
        work = Task.detached(priority: .userInitiated) { [weak self] in
            let content = await Self.rich(payload: payload, original: request.content, settings: settings) ?? generic
            self?.finish(content)
        }
        Task.detached { [weak self] in
            try? await Task.sleep(for: Self.deadline)
            self?.finish(generic)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        work?.cancel()
        finish(lock.withLock { fallback } ?? UNMutableNotificationContent())
    }

    /// Hands the content over once; later calls do nothing.
    private func finish(_ content: UNNotificationContent) {
        let handler: ((UNNotificationContent) -> Void)? = lock.withLock {
            defer { deliver = nil }
            return deliver
        }
        handler?(content)
    }

    /// What the core says about the pushed item; nil for the generic text.
    ///
    /// The store is shared with the app, under a file lock this process holds only with an expiring activity (iOS ends a
    /// process suspended with it held). The lock is not waited on for long at a time: when the app holds it (often
    /// because the same push woke it, and it is deciding the item), this tries again after a pause, a few times, then
    /// leaves the item to the app and shows the generic text, without an action.
    private static func rich(payload: PushPayload, original: UNNotificationContent, settings: FeedbackSettings) async -> UNNotificationContent? {
        McpNames.loadSaved()
        StoreLocking.install(ExpiringActivityKeepAwake(), waitMillis: CoreFactory.extensionLockWaitMillis)
        for attempt in 0..<busyTries {
            switch await richOnce(payload: payload, original: original, settings: settings) {
            case let .done(content):
                return content
            case .busy where attempt < busyTries - 1 && !Task.isCancelled:
                try? await Task.sleep(for: busyPause)
            case .busy, .failed:
                return nil
            }
        }
        return nil
    }

    private enum Attempt {
        case done(UNNotificationContent)
        /// The store could not be had now (the app holds it): worth one more try.
        case busy
        case failed
    }

    private static func richOnce(payload: PushPayload, original: UNNotificationContent, settings: FeedbackSettings) async -> Attempt {
        let capture = CapturingNotifier()
        // `mayReset: false`: whatever goes wrong with the keychain here, the core never starts over from this process.
        let core: ReinsCore
        do {
            core = try CoreFactory.make(notifier: capture, keys: KeychainKeyWrapper(mayReset: false, account: CoreFactory.keyAccount))
        } catch CoreError.Storage {
            return .busy
        } catch {
            return .failed
        }
        let session: SessionInfo?
        do {
            session = try await core.session()
        } catch {
            return .busy
        }
        guard session != nil else { return .failed }
        // Manual, Auto and Overnight judge with the model, which only the app runs: park the item unjudged (handling it
        // fully here would settle it as "could not judge" for good) and leave the judging to the app's next pass.
        // Bypass and Lockdown decide either way: the deferring call still runs them.
        let deferred = (try? await core.autopilotSettings()).map(ExtensionPolicy.needsModel) ?? true
        do {
            if deferred {
                try await core.handlePushDeferringAutopilot(kind: payload.kind, id: payload.id)
            } else {
                try await core.handlePush(kind: payload.kind, id: payload.id)
            }
        } catch CoreError.Storage {
            // Nothing is half done: what was fetched and parked is known, and the next try skips it.
            return .busy
        } catch {
            return .failed
        }
        // Only the owner's list makes an ask: what a mode decided is said as decided, and what Autopilot has still to
        // decide (only the app can) is said as that, without an action.
        let decision = capture.decision(payload.id)
        var item = capture.item(payload.id)
        if decision == nil, item == nil { item = (try? await core.pending())?.first { $0.id == payload.id } }
        let deciding = decision == nil && item == nil ? (try? await core.deciding())?.first { $0.id == payload.id } : nil
        let content = NotificationContent.outcome(
            PushOutcome.of(decision: decision, waiting: item, deciding: deciding), original: original, settings: settings
        )
        // Widgets show the new item too; this process may be suspended as soon as the content is handed over.
        if !Task.isCancelled { await SnapshotWriter.update(from: core) }
        return .done(WorkProfiles.stamp(content, profile: WorkProfiles.active))
    }
}

/// The core's `Notifier` for one push: keeps what it reported so the notification can say it.
private final class CapturingNotifier: Notifier, @unchecked Sendable {
    private let lock = NSLock()
    private var items: [String: PendingItem] = [:]
    private var decisions: [String: AutoDecisionView] = [:]

    func itemPending(item: PendingItem) { lock.withLock { items[item.id] = item } }

    func itemResolved(id: String) { lock.withLock { items[id] = nil } }

    func autoDecided(decision: AutoDecisionView) { lock.withLock { decisions[decision.requestId] = decision } }

    func autopilotChanged(event: AutopilotEvent) {}

    func item(_ id: String) -> PendingItem? { lock.withLock { items[id] } }

    func decision(_ id: String) -> AutoDecisionView? { lock.withLock { decisions[id] } }
}
