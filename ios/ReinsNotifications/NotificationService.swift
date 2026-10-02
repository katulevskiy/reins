import UserNotifications

/// Rewrites a push ("Reins", "Something is waiting for you") into what actually waits, like the Android app's
/// notifications: it opens the same store as the app (App Group container, shared keychain), lets the core fetch the
/// item the push names, and words the notification from what the core reports. Autopilot's decisions come out as its
/// quiet notification. Sounds follow the in-app switches.
///
/// Anything that cannot be done in time, or safely, delivers the generic text with the right category and sound:
/// the app in front (it handles the push itself), no store or no session, a locked keychain, a network failure.
/// When Autopilot needs its model (which only the app can run; this process has a few MB of memory), the item is
/// parked without Autopilot's pass and the app judges it when it next runs (the push also wakes it in the background).
final class NotificationService: UNNotificationServiceExtension {
    private let lock = NSLock()
    private var deliver: ((UNNotificationContent) -> Void)?
    private var fallback: UNNotificationContent?
    private var work: Task<Void, Never>?

    /// The system allows about 30 s; the generic text goes out well before.
    private static let deadline: Duration = .seconds(22)

    override func didReceive(_ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void) {
        let settings = FeedbackSettings.load()
        let payload = PushPayload(userInfo: request.content.userInfo)
        let generic = NotificationContent.generic(request.content, payload: payload, settings: settings)
        lock.withLock {
            deliver = contentHandler
            fallback = generic
        }
        guard let payload, payload.itemKind != nil, !AppPresence.inFront(), CoreFactory.storeExists else {
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
    private static func rich(payload: PushPayload, original: UNNotificationContent, settings: FeedbackSettings) async -> UNNotificationContent? {
        McpNames.loadSaved()
        let capture = CapturingNotifier()
        // `mayReset: false`: whatever goes wrong with the keychain here, the core never starts over from this process.
        guard let core = try? CoreFactory.make(notifier: capture, keys: KeychainKeyWrapper(mayReset: false)) else { return nil }
        guard await core.session() != nil else { return nil }
        // Assisted and Auto judge with the model, which only the app runs: park the item unjudged (handling it fully here
        // would settle it as "could not judge" for good) and leave the judging to the app's next pass.
        let deferred = (try? await core.autopilotSettings()).map(ExtensionPolicy.needsModel) ?? true
        do {
            if deferred {
                try await core.handlePushDeferringAutopilot(kind: payload.kind, id: payload.id)
            } else {
                try await core.handlePush(kind: payload.kind, id: payload.id)
            }
        } catch {
            return nil
        }
        var item = capture.item(payload.id)
        if item == nil { item = (try? await core.pending())?.first { $0.id == payload.id } }
        let content: UNNotificationContent
        if let decision = capture.decision(payload.id) {
            content = NotificationContent.decision(decision, settings: settings)
        } else if let item {
            content = NotificationContent.pending(item, settings: settings)
        } else {
            // Answered before it could be shown (a grant, another device, its time ran out).
            content = NotificationContent.handled(original)
        }
        // Widgets show the new item too; this process may be suspended as soon as the content is handed over.
        if !Task.isCancelled { await SnapshotWriter.update(from: core) }
        return content
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
