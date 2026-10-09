#if REINS_APP || REINS_WIDGETS
import AppIntents
import Foundation

// The App Intents behind widget and Live Activity buttons, Control Center controls, Siri and Shortcuts. They are
// compiled into the app and the widget extension. The widget extension never runs the core, so every intent that
// changes something is a `LiveActivityIntent` (the system runs those in the app's process, launching it in the
// background if needed) and does its work through `IntentBridge`; the `#else` branches only exist so the widget
// extension compiles.

/// Turns Lockdown on or off: the Control Center toggle and the Autopilot widget's button. Ending Lockdown lets
/// requests through again, so both ways need an unlocked phone.
struct SetLockdownIntent: SetValueIntent, LiveActivityIntent {
    static var title: LocalizedStringResource = "Lockdown"
    static var description = IntentDescription("Turns Lockdown on or off. While it is on, every request is denied at once.")
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication
    static var isDiscoverable = false

    @Parameter(title: "Lockdown")
    var value: Bool

    init() {}

    init(value: Bool) {
        self.value = value
    }

    func perform() async throws -> some IntentResult {
        #if REINS_APP
        try await IntentBridge.setLockdown(value)
        #endif
        return .result()
    }
}

/// "Lock down Reins": Lockdown on, from Siri, Shortcuts or the Action button.
struct LockDownIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Lock down"
    static var description = IntentDescription("Turns on Lockdown: every request is denied at once, the ones waiting now included.")
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication

    func perform() async throws -> some IntentResult & ProvidesDialog {
        #if REINS_APP
        try await IntentBridge.setLockdown(true)
        #endif
        return .result(dialog: "Reins is locked down. Every request is denied until you end Lockdown.")
    }
}

/// Ends every running bypass: the Bypass Live Activity's Stop, the Autopilot widget, the control, Siri.
struct StopBypassIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Stop the bypass"
    static var description = IntentDescription("Ends every bypass now. Requests wait for you again.")

    func perform() async throws -> some IntentResult & ProvidesDialog {
        #if REINS_APP
        let stopped = try await IntentBridge.stopBypass()
        return .result(dialog: stopped ? "The bypass is off. Requests wait for you again." : "No bypass is running.")
        #else
        return .result(dialog: "The bypass is off.")
        #endif
    }
}

/// "Deny" on the requests Live Activity. Denying needs an unlocked phone but no Face ID (as the notification's Deny).
struct DenyRequestIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Deny request"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication
    static var isDiscoverable = false

    @Parameter(title: "Request")
    var requestId: String

    init() {}

    init(requestId: String) {
        self.requestId = requestId
    }

    func perform() async throws -> some IntentResult {
        #if REINS_APP
        try await IntentBridge.deny(requestId)
        #endif
        return .result()
    }
}

/// "Approve" on the requests Live Activity: the notification's one-tap answer for a routine request (the core refuses
/// anything that is asked every time). Like the notification's Approve, it needs an unlocked phone.
struct ApproveQuickIntent: LiveActivityIntent {
    static var title: LocalizedStringResource = "Approve request"
    static var authenticationPolicy: IntentAuthenticationPolicy = .requiresAuthentication
    static var isDiscoverable = false

    @Parameter(title: "Request")
    var requestId: String

    init() {}

    init(requestId: String) {
        self.requestId = requestId
    }

    func perform() async throws -> some IntentResult {
        #if REINS_APP
        try await IntentBridge.approveQuick(requestId)
        #endif
        return .result()
    }
}

/// Opens Reins on a waiting request (the newest when none is given), or on the home screen when nothing waits:
/// "Show what is waiting in Reins", the "Waiting request" control.
struct ShowWaitingIntent: AppIntent {
    static var title: LocalizedStringResource = "Show what is waiting"
    static var description = IntentDescription("Opens Reins on the newest request that waits for you.")
    static var supportedModes: IntentModes = .foreground(.immediate)

    @Parameter(title: "Request")
    var item: WaitingItemEntity?

    init() {}

    init(item: WaitingItemEntity?) {
        self.item = item
    }

    func perform() async throws -> some IntentResult {
        let link = Self.link(for: item?.id, in: Snapshot.load(), now: Glance.now())
        #if REINS_APP
        await IntentBridge.open(link)
        return .result()
        #else
        // Normally run in the app (it opens the app); should a widget process run it, the link opens the app.
        return .result(opensIntent: OpenURLIntent(link.url))
        #endif
    }

    /// The item asked for if it still waits, else the newest waiting one, else home.
    static func link(for id: String?, in s: Snapshot, now: Int64) -> DeepLink {
        let waiting = Glance.waiting(s, now: now)
        if let id, let item = waiting.first(where: { $0.id == id }) { return .item(kind: item.kind, id: item.id) }
        if let item = waiting.first { return .item(kind: item.kind, id: item.id) }
        return .home
    }
}

// MARK: Waiting items as entities

/// A request, pairing or file that waits for an answer, as Siri and Shortcuts see it. Read from the snapshot, so
/// nothing secret: the same one-line titles the notifications show.
struct WaitingItemEntity: AppEntity {
    static var typeDisplayRepresentation: TypeDisplayRepresentation = "Waiting request"
    static var defaultQuery = WaitingItemQuery()

    var id: String
    var title: String
    var subtitle: String

    init(_ item: Snapshot.Item) {
        id = item.id
        title = item.title
        subtitle = item.subtitle
    }

    var displayRepresentation: DisplayRepresentation {
        subtitle.isEmpty ? DisplayRepresentation(title: "\(title)") : DisplayRepresentation(title: "\(title)", subtitle: "\(subtitle)")
    }
}

struct WaitingItemQuery: EntityQuery {
    func entities(for identifiers: [String]) async throws -> [WaitingItemEntity] {
        let wanted = Set(identifiers)
        return Glance.waiting(Snapshot.load(), now: Glance.now()).filter { wanted.contains($0.id) }.map(WaitingItemEntity.init)
    }

    func suggestedEntities() async throws -> [WaitingItemEntity] {
        Glance.waiting(Snapshot.load(), now: Glance.now()).map(WaitingItemEntity.init)
    }
}

// MARK: Errors

enum ReinsIntentError: Error, CustomLocalizedStringResourceConvertible {
    case signedOut
    case failed(String)

    var localizedStringResource: LocalizedStringResource {
        switch self {
        case .signedOut: "Sign in to Reins first."
        case let .failed(message): "\(message)"
        }
    }
}

#if REINS_APP
/// "Lock down Reins", "Stop the Reins bypass", "Show what is waiting in Reins".
struct ReinsShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: LockDownIntent(),
            phrases: ["Lock down \(.applicationName)", "Turn on \(.applicationName) lockdown"],
            shortTitle: "Lock down",
            systemImageName: "lock.fill"
        )
        AppShortcut(
            intent: StopBypassIntent(),
            phrases: ["Stop the \(.applicationName) bypass", "End the \(.applicationName) bypass"],
            shortTitle: "Stop the bypass",
            systemImageName: "bolt.slash.fill"
        )
        AppShortcut(
            intent: ShowWaitingIntent(),
            phrases: ["Show what is waiting in \(.applicationName)", "What is waiting in \(.applicationName)"],
            shortTitle: "What is waiting",
            systemImageName: "tray.full.fill"
        )
    }

    static var shortcutTileColor: ShortcutTileColor = .purple
}
#endif
#endif
