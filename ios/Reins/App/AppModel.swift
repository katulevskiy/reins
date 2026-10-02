import Foundation
import Observation
import UIKit
import WidgetKit

enum SessionState: Equatable {
    case loading
    case signedOut
    case signedIn(SessionInfo)
}

/// Whether this phone receives approval requests, kept across launches and shared with the extensions (the Android
/// app's `DeviceStatusStore`).
enum DeviceStatus {
    private static let replacedKey = "device.replaced"
    private static let approvalKey = "device.approval"
    private static let seenKey = "device.seenActivity"
    private static var d: UserDefaults { AppGroup.defaults }

    static var replaced: Bool {
        get { d.bool(forKey: replacedKey) }
        set {
            d.set(newValue, forKey: replacedKey)
            if newValue { d.set(false, forKey: approvalKey) }
        }
    }

    static var approvalDevice: Bool {
        get { d.bool(forKey: approvalKey) }
        set { d.set(newValue, forKey: approvalKey) }
    }

    static var seenActivityId: Int64 {
        get { Int64(d.integer(forKey: seenKey)) }
        set { d.set(Int(newValue), forKey: seenKey) }
    }

    static func clear() {
        [replacedKey, approvalKey, seenKey].forEach(d.removeObject(forKey:))
    }
}

/// Everything the screens share: the core, the state read from it, navigation, and the lifecycle around them (the
/// Android app's `AppContainer`, `AppState` and `AppViewModel` in one). Main-actor bound; the core's calls are async
/// and never block it.
@Observable
@MainActor
final class AppModel {
    let core: any RewardenCoreProtocol
    let feedback: Feedback
    let authenticator: Authenticating
    /// True for the `-demo` fake core (screenshots, UI tests, a look around without a server).
    let demo: Bool

    // MARK: State read from the core

    private(set) var session: SessionState = .loading
    private(set) var pending: [PendingItem] = []
    /// Everything that happened, newest first.
    private(set) var activity: [ActivityEntry] = []
    private(set) var grants: [GrantView] = []
    private(set) var accounts: [AccountView] = []
    private(set) var services: [ServiceView] = []
    private(set) var connections: [ConnectionView] = []
    private(set) var mcpServers: [McpServerView] = []
    /// Autopilot's modes, bypasses and model; nil until first read.
    private(set) var autopilot: AutopilotSettings?
    private(set) var seenActivityId: Int64 = 0
    /// The server said another phone became the approval device.
    private(set) var deviceReplaced = false
    /// This phone currently receives approval requests.
    private(set) var approvalDevice = false
    /// Why registering this phone as the approval device failed, until it succeeds.
    var registrationError: String?
    /// How the last MCP sign-in ended, shown on that server's page until it is left.
    var mcpNotice: McpNotice?

    // MARK: Navigation

    var section: AppSection = .activity
    /// Per section: the screens above its root. On a regular width the first is the detail column's root.
    var paths: [AppSection: [Route]] = [:]
    var sheet: SheetTarget?
    /// A one-line message over the content (a toast).
    var notice: String?
    /// Whether the "Expired" list on the Grants tab is open; kept while the user browses.
    var expiredOpen = false
    /// Changes for every visit to "New grant", so the form always starts empty.
    private(set) var newGrantToken = 0
    /// Items already popped up once; dismissing one leaves it in the list without popping up again.
    private var presented: Set<String> = []

    // MARK: Lifecycle

    /// The scene is active (in front and focused).
    private(set) var isActive = false
    /// New requests pop up by themselves while the app is in front. Tests and screenshots turn this off.
    var autoPopup = true
    private var syncTask: Task<Void, Never>?
    /// The APNs device token, hex, once the system gave one.
    private(set) var pushToken: String?

    /// Hooks the platform layer sets (notifications, Live Activities); the model calls them after state changes.
    var onPendingChanged: (([PendingItem]) -> Void)?
    var onAutopilotChanged: ((AutopilotSettings?) -> Void)?
    var onGrantsChanged: (([GrantView]) -> Void)?

    init(core: any RewardenCoreProtocol, feedback: Feedback, authenticator: Authenticating, demo: Bool = false) {
        self.core = core
        self.feedback = feedback
        self.authenticator = authenticator
        self.demo = demo
    }

    // MARK: Session

    func refreshSession() async {
        let info = await core.session()
        if info != nil {
            seenActivityId = DeviceStatus.seenActivityId
            approvalDevice = DeviceStatus.approvalDevice && !DeviceStatus.replaced
        }
        setSession(info.map(SessionState.signedIn) ?? .signedOut)
        if info != nil {
            await refreshPending()
            await refreshConnections()
            if !DeviceStatus.replaced { await registerDeviceQuietly() }
        }
    }

    func setSession(_ state: SessionState) {
        session = state
        if case .signedIn = state { return }
        pending = []
        activity = []
        grants = []
        accounts = []
        services = []
        connections = []
        setMcpServers([])
        mcpNotice = nil
        approvalDevice = false
        autopilot = nil
        sheet = nil
        paths = [:]
        section = .activity
        publish()
    }

    func signedIn(_ info: SessionInfo) async {
        setSession(.signedIn(info))
        await refreshSession()
    }

    func signOut() async {
        try? await core.logout()
        DeviceStatus.clear()
        approvalDevice = false
        deviceReplaced = false
        setSession(.signedOut)
    }

    // MARK: Refreshing

    /// Re-reads everything held on the phone: what waits, what happened, which permissions exist.
    func refreshPending() async {
        do {
            pending = try await core.pending()
            activity = try await core.activity(limit: 300)
            grants = try await core.grants()
            accounts = try await core.accounts()
            services = try await core.services().filter { $0.service != "sms" }
            setMcpServers(try await core.mcpServers())
            await refreshAutopilot()
            onGrantsChanged?(grants)
        } catch CoreError.NotLoggedIn {
            setSession(.signedOut)
        } catch {
            // Offline or a failing store: keep what is shown.
        }
        deviceReplaced = DeviceStatus.replaced
        onPendingChanged?(pending)
        maybePresent()
        publish()
    }

    /// Re-reads Autopilot (modes, bypasses, model) and keeps what shows it in step.
    @discardableResult
    func refreshAutopilot() async -> AutopilotSettings? {
        guard let settings = try? await core.autopilotSettings() else { return autopilot }
        autopilot = settings
        onAutopilotChanged?(settings)
        publish()
        return settings
    }

    /// Ends every bypass now: each goes back to the mode it interrupted.
    func stopBypasses() async {
        guard let settings = await refreshAutopilot() else { return }
        let now = Int64(Date().timeIntervalSince1970)
        if (settings.bypassUntil ?? 0) > now {
            try? await core.setAutopilotMode(connectionId: nil, mode: settings.baseMode, minutes: nil)
        }
        for c in settings.connections where (c.bypassUntil ?? 0) > now {
            try? await core.setAutopilotMode(connectionId: c.connectionId, mode: c.baseMode, minutes: nil)
        }
        await refreshAutopilot()
    }

    /// The AI connections (a network call; failures keep the last list).
    func refreshConnections() async {
        if let list = try? await core.connections() {
            connections = list
            publish()
        }
    }

    private func setMcpServers(_ list: [McpServerView]) {
        mcpServers = list
        McpNames.update(Dictionary(uniqueKeysWithValues: list.map {
            ($0.id, McpNames.Server(name: $0.name, tools: Dictionary($0.tools.map { ($0.name, $0.title) }, uniquingKeysWith: { a, _ in a })))
        }))
    }

    /// The user looked at the activity list up to `id`.
    func markActivitySeen(_ id: Int64) {
        guard id > seenActivityId else { return }
        seenActivityId = id
        DeviceStatus.seenActivityId = id
    }

    var unseenActivity: Int { activity.filter { $0.id > seenActivityId }.count }

    var activeGrants: Int { grants.filter(\.active).count }

    func connection(_ id: String) -> ConnectionView? { connections.first { $0.id == id } }

    // MARK: Approval device

    func setPushToken(_ token: Data) {
        let hex = token.map { String(format: "%02x", $0) }.joined()
        guard hex != pushToken else { return }
        pushToken = hex
        if case .signedIn = session, !DeviceStatus.replaced {
            Task { await registerDeviceQuietly() }
        }
    }

    /// `apns:<hex>` (production) or `apns-sandbox:<hex>` (development builds): the server sends to APNs for these.
    var registrationToken: String? {
        guard let pushToken else { return nil }
        let sandbox = (Bundle.main.object(forInfoDictionaryKey: "ReinsApsEnvironment") as? String ?? "development") != "production"
        return (sandbox ? "apns-sandbox:" : "apns:") + pushToken
    }

    /// Makes this phone the approval device. `force` is the user's explicit "use this phone"; background token
    /// refreshes never take the role back from a phone that replaced this one.
    func registerDevice(force: Bool) async throws {
        if DeviceStatus.replaced && !force { return }
        try await core.registerDevice(fcmToken: registrationToken)
        DeviceStatus.replaced = false
        DeviceStatus.approvalDevice = true
        deviceReplaced = false
        approvalDevice = true
        registrationError = nil
        publish()
    }

    private func registerDeviceQuietly() async {
        do {
            try await registerDevice(force: false)
        } catch {
            registrationError = error.userMessage
        }
    }

    /// The server says another phone is the approval device now.
    func markReplaced() {
        DeviceStatus.replaced = true
        deviceReplaced = true
        approvalDevice = false
        publish()
    }

    // MARK: Foreground

    func setActive(_ active: Bool) {
        guard active != isActive else { return }
        isActive = active
        if active {
            startSync()
            Task {
                await refreshPending()
                await refreshConnections()
            }
        } else {
            syncTask?.cancel()
            syncTask = nil
        }
    }

    /// While the app is in front, long-polls the server so requests show up within a second even without push.
    private func startSync() {
        syncTask?.cancel()
        syncTask = Task { [weak self] in
            var failures = 0
            while !Task.isCancelled {
                guard let self, case .signedIn = self.session, !self.deviceReplaced else {
                    try? await Task.sleep(for: .seconds(2))
                    continue
                }
                do {
                    _ = try await self.core.sync(waitSecs: 25)
                    // Requests that grants answered by themselves leave no prompt, only a new activity entry.
                    await self.refreshPending()
                    failures = 0
                } catch CoreError.NotLoggedIn {
                    self.setSession(.signedOut)
                } catch let CoreError.Server(status, _) where status == 403 {
                    self.markReplaced()
                } catch {
                    if Task.isCancelled { return }
                    try? await Task.sleep(for: .milliseconds(Self.backoffMillis(failures)))
                    failures += 1
                }
            }
        }
    }

    /// 1 s, 2 s, 4 s ... capped at 30 s.
    nonisolated static func backoffMillis(_ failures: Int) -> Int {
        min(30_000, 1_000 << min(max(failures, 0), 5))
    }

    // MARK: Navigation

    func select(_ target: AppSection) {
        if target == section { paths[target] = [] }
        section = target
    }

    func path(_ s: AppSection) -> [Route] { paths[s] ?? [] }

    /// From a section's root list: the detail column shows `route` (a regular width), or it is pushed (compact).
    func show(_ route: Route, in target: AppSection? = nil) {
        let s = target ?? section
        section = s
        paths[s] = [prepared(route)]
    }

    /// From a pushed screen: one more screen on top.
    func push(_ route: Route) {
        let r = prepared(route)
        var p = path(section)
        if p.last != r { p.append(r) }
        paths[section] = p
    }

    func back() {
        var p = path(section)
        if !p.isEmpty { p.removeLast() }
        paths[section] = p
    }

    func home() {
        paths[.activity] = []
        section = .activity
    }

    private func prepared(_ route: Route) -> Route {
        if case .newGrant = route {
            newGrantToken += 1
            return .newGrant(token: newGrantToken)
        }
        return route
    }

    /// Opens the sheet for `target`; it will not pop up by itself again.
    func openSheet(_ target: SheetTarget) {
        presented.insert(target.id)
        sheet = target
    }

    func closeSheet() {
        sheet = nil
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(350))
            self.maybePresent()
        }
    }

    private func maybePresent() {
        guard isActive, autoPopup, sheet == nil else { return }
        presented.formIntersection(Set(pending.map(\.id)))
        guard let next = pending.first(where: { !presented.contains($0.id) }) else { return }
        openSheet(next.sheetTarget)
    }

    /// A link from a notification, widget, control or Live Activity. Items open only if the core really has them.
    func handle(_ link: DeepLink) async {
        switch link {
        case let .item(kind, id):
            let items = (try? await core.pending()) ?? pending
            home()
            if let item = items.first(where: { $0.id == id && $0.snapshotKind == kind }) {
                openSheet(item.sheetTarget)
            } else {
                notice = "That request is no longer waiting."
            }
        case let .activity(id):
            await refreshPending()
            home()
            show(.activityDetail(id), in: .activity)
        case let .grant(id):
            await refreshPending()
            section = .grants
            paths[.grants] = grants.contains { $0.id == id } ? [.grantDetail(id)] : []
        case .autopilot:
            section = .autopilot
            paths[.autopilot] = []
        case .home:
            home()
        }
    }

    // MARK: Widgets

    /// Writes what widgets and controls show and asks WidgetKit to redraw.
    func publish() {
        var s = Snapshot()
        if case .signedIn = session { s.signedIn = true }
        s.approvalDevice = approvalDevice
        s.pending = pending.map(\.snapshotItem)
        s.latest = activity.prefix(6).map(\.snapshotEntry)
        if let a = autopilot {
            s.autopilotMode = a.mode.key
            s.bypassUntil = a.bypassUntil
        }
        s.activeGrants = activeGrants
        s.updatedAt = Int64(Date().timeIntervalSince1970)
        guard s != Snapshot.load() || s.updatedAt == 0 else { return }
        s.save()
        WidgetCenter.shared.reloadAllTimelines()
    }
}

/// How an MCP sign-in ended: `text` says how, `failed` when it did not work.
struct McpNotice: Equatable {
    var serverId: String
    var text: String
    var failed: Bool
}

extension AutopilotMode {
    /// The key widgets and links use.
    var key: String {
        switch self {
        case .manual: "manual"
        case .assisted: "assisted"
        case .auto: "auto"
        case .bypass: "bypass"
        case .lockdown: "lockdown"
        }
    }

    /// Manual < Assisted < Auto < Bypass; Lockdown is the least autonomous.
    var autonomy: Int {
        switch self {
        case .lockdown: 0
        case .manual: 1
        case .assisted: 2
        case .auto: 3
        case .bypass: 4
        }
    }
}

extension PendingItem {
    /// How long the server holds a request for an answer (the relay's TTL).
    static let answerWindow: Int64 = 600

    var expiresAt: Int64 { waitUntil ?? (createdAt + Self.answerWindow) }

    var headline: String {
        switch kind {
        case .pairing: untrusted(title)
        case .blob: "\(untrusted(connectionLabel)): Share a file"
        case .request: fullTitle(label: connectionLabel, action: action, count: Int(count), service: service, title: opTitle, op: op)
        }
    }

    var snapshotItem: Snapshot.Item {
        Snapshot.Item(
            id: id,
            kind: snapshotKind,
            title: headline,
            subtitle: untrusted(subtitle),
            connection: untrusted(connectionLabel),
            service: service,
            createdAt: createdAt,
            expiresAt: expiresAt,
            suggestion: suggestion
        )
    }
}

extension ActivityEntry {
    var headline: String {
        entryTitle(label: connectionLabel, action: action, count: Int(count), service: service, outcome: outcome, title: opTitle, op: op)
    }

    var snapshotEntry: Snapshot.Entry {
        Snapshot.Entry(id: id, title: headline, outcome: outcome.capitalized, approved: !["denied", "failed", "expired"].contains(outcome), at: at)
    }
}
