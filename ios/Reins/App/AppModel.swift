import Foundation
import Observation
import UIKit
import WidgetKit

enum SessionState: Equatable {
    case loading
    case signedOut
    case signedIn(SessionInfo)
    /// Signed in through "Continue" to an account whose keys this phone cannot open yet (another phone, or the recovery
    /// code, has them): the Unlock screen shows, and this phone does not take the approval role until it can.
    case keysLocked(SessionInfo)
}

/// The account a passwordless sign-in left locked on this phone (server and email), kept across launches so a restart
/// comes back to the Unlock screen instead of registering the phone.
enum KeysLock {
    private static let key = "account.keysLocked"

    static func set(_ info: SessionInfo) { AppGroup.defaults.set("\(info.serverUrl)|\(info.email)", forKey: key) }

    static func matches(_ info: SessionInfo) -> Bool {
        AppGroup.defaults.string(forKey: key) == "\(info.serverUrl)|\(info.email)"
    }

    static func clear() { AppGroup.defaults.removeObject(forKey: key) }
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
    /// The account was made without a master password and this phone keeps its secret: Settings offers the recovery
    /// code.
    private(set) var recoveryCodeAvailable = false
    /// How the last MCP sign-in ended, shown on that server's page until it is left.
    var mcpNotice: McpNotice?
    /// After a fresh sign-in (or a new account) on the sign-in screen: the steps that connect a computer and an AI
    /// show instead of the app until "Done".
    private(set) var onboarding = false
    /// A pairing code from a link opened before this phone could redeem it (signed out, still starting, not yet the
    /// approval device); redeemed as soon as it can be.
    private(set) var waitingPairCode: String?

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
    /// A model download ended: nil when the model is installed, else why it failed.
    var onModelDownloadFinished: ((String?) -> Void)?
    /// The session is there (at launch or after signing in): the moment to ask for notifications.
    var onSignedIn: (() -> Void)?

    /// Autopilot's model coming down: the job, its bytes, the network it waits for (the screen, the Live Activity).
    let modelDownloads: ModelDownloads

    init(core: any RewardenCoreProtocol, feedback: Feedback, authenticator: Authenticating, demo: Bool = false,
         modelDownloads: ModelDownloads? = nil) {
        self.core = core
        self.feedback = feedback
        self.authenticator = authenticator
        self.demo = demo
        self.modelDownloads = modelDownloads ?? ModelDownloads(core: core, feedback: feedback)
        self.modelDownloads.onFinished = { [weak self] failure in
            await self?.refreshAutopilot()
            self?.onModelDownloadFinished?(failure)
        }
    }

    // MARK: Session

    func refreshSession() async {
        let info = await core.session()
        // The demo core keeps nothing across launches, so neither does its lock.
        if let info, !demo, KeysLock.matches(info) {
            setSession(.keysLocked(info))
            return
        }
        if info != nil {
            seenActivityId = DeviceStatus.seenActivityId
            approvalDevice = DeviceStatus.approvalDevice && !DeviceStatus.replaced
        }
        setSession(info.map(SessionState.signedIn) ?? .signedOut)
        if info != nil {
            onSignedIn?()
            await refreshPending()
            await refreshConnections()
            recoveryCodeAvailable = (try? await core.accountRecoveryCode()) != nil
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
        recoveryCodeAvailable = false
        onboarding = false
        autopilot = nil
        sheet = nil
        paths = [:]
        section = .activity
        onPendingChanged?([])
        onGrantsChanged?([])
        publish()
    }

    func signedIn(_ info: SessionInfo) async {
        setSession(.signedIn(info))
        await refreshSession()
    }

    /// Signing in or creating an account on the sign-in screen: the session, the approval role (taken even from a
    /// phone another one took it from), and the onboarding steps the first time this account signs in on this phone.
    /// Notifications are asked for once the session is there (`onSignedIn`).
    func finishSignIn(_ info: SessionInfo) async {
        registrationError = nil
        // The demo core keeps nothing across launches, so it shows the steps every time.
        onboarding = demo || OnboardingRecord.firstTime(server: info.serverUrl, email: info.email)
        await signedIn(info)
        if !approvalDevice {
            do {
                try await registerDevice(force: true)
            } catch {
                registrationError = error.userMessage
            }
        }
    }

    /// "Continue" came back. A new account (its keys were just made) or one this phone can open goes on like a password
    /// sign-in; one whose keys another phone has waits on the Unlock screen.
    func finishSso(_ outcome: SsoOutcome) async {
        registrationError = nil
        if outcome.keys == .locked {
            if !demo { KeysLock.set(outcome.session) }
            setSession(.keysLocked(outcome.session))
        } else {
            KeysLock.clear()
            await finishSignIn(outcome.session)
        }
    }

    /// The Unlock screen opened the keys (the other phone approved, or the recovery code): on as after a sign-in.
    func finishUnlock() async {
        guard case let .keysLocked(info) = session else { return }
        KeysLock.clear()
        await finishSignIn(info)
    }

    func finishOnboarding() {
        onboarding = false
    }

    func signOut() async {
        try? await core.logout()
        KeysLock.clear()
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
        let token = registrationToken
        do {
            try await core.registerDevice(fcmToken: token)
        } catch let CoreError.Server(status, _) where status == 400 && token != nil {
            // A server that refuses the push token still takes the phone, which then gets requests while the app is
            // open: servers from before October 2026 accept only 32-byte APNs tokens, and a simulator's are 80 bytes.
            try await core.registerDevice(fcmToken: nil)
        }
        DeviceStatus.replaced = false
        DeviceStatus.approvalDevice = true
        deviceReplaced = false
        approvalDevice = true
        registrationError = nil
        publish()
        if let code = waitingPairCode {
            waitingPairCode = nil
            await openPairing(code: code)
        }
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
        AppPresence.setActive(active)
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
                } catch let CoreError.Server(status, _) where status == 403 && self.approvalDevice {
                    // Only a phone that held the role was replaced. One whose registration has not gone through yet
                    // (registrationError says why) keeps trying, and its banner does not blame another phone.
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

    // MARK: Pairing codes

    /// A computer's pairing code from a link: redeemed now if this phone approves requests, else kept until it does.
    func openPairing(code: String) async {
        switch session {
        case .signedIn where approvalDevice:
            do {
                let view = try await redeemPairingCode(code)
                openSheet(.pairing(view.id))
            } catch {
                feedback.play(.error)
                notice = Self.pairingCodeMessage(error)
            }
        case .signedIn where deviceReplaced:
            feedback.play(.error)
            notice = "Use this phone for approvals (Settings), then open the code again."
        default:
            waitingPairCode = code
        }
    }

    /// Asks the server for the pairing `code` stands for; the core parks it like a pushed one, for the pairing sheet.
    func redeemPairingCode(_ code: String) async throws -> PairingView {
        let view = try await core.pairingByCode(userCode: code)
        // It is answered where it was opened, and does not pop up again by itself.
        presented.insert(view.id)
        await refreshPending()
        return view
    }

    /// What to say when a pairing code does not work.
    static func pairingCodeMessage(_ error: Error) -> String {
        if case CoreError.NotFound = error {
            return "This code has expired or was already used. Show a new one on your computer."
        }
        return error.userMessage
    }

    /// A link from a notification, widget, control, Live Activity or a computer's pairing code. Items open only if the
    /// core really has them.
    func handle(_ link: DeepLink) async {
        if case let .pair(code) = link { return await openPairing(code: code) }
        guard case .signedIn = session else { return }
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
        case .integrations:
            home()
            show(.integrations, in: .activity)
        case .home:
            home()
        case .pair:
            break
        }
    }

    // MARK: Widgets

    /// An item as widgets and Live Activities show it, with its connection's logo pick.
    func snapshotItem(_ item: PendingItem) -> Snapshot.Item {
        var s = item.snapshotItem
        s.connectionIcon = connection(item.connectionId)?.icon
        return s
    }

    /// Writes what widgets and controls show and asks WidgetKit to redraw.
    func publish() {
        var s = Snapshot()
        if case .signedIn = session { s.signedIn = true }
        s.approvalDevice = approvalDevice
        s.pending = pending.map(snapshotItem)
        s.latest = activity.prefix(6).map(\.snapshotEntry)
        if let a = autopilot {
            s.autopilotMode = a.mode.key
            s.bypassUntil = a.bypassUntil
            s.baseMode = a.baseMode.key
            s.anyBypassUntil = a.lastBypassEnd
        }
        s.activeGrants = activeGrants
        s.updatedAt = Int64(Date().timeIntervalSince1970)
        guard s != Snapshot.load() || s.updatedAt == 0 else { return }
        s.save()
        WidgetCenter.shared.reloadAllTimelines()
        ControlCenter.shared.reloadAllControls()
    }
}

/// Which accounts have been through the onboarding steps on this phone: they show once per account (server and
/// email), after it signs in on the sign-in screen, so never for a session from before they existed.
enum OnboardingRecord {
    static func key(server: String, email: String) -> String {
        var s = server.trimmingCharacters(in: .whitespaces).lowercased()
        while s.hasSuffix("/") { s.removeLast() }
        return "onboarded:\(s)|\(email.trimmingCharacters(in: .whitespaces).lowercased())"
    }

    /// True the first time `email` on `server` signs in on this phone; records that it did.
    static func firstTime(server: String, email: String, defaults: UserDefaults = AppGroup.defaults) -> Bool {
        let k = key(server: server, email: email)
        guard !defaults.bool(forKey: k) else { return false }
        defaults.set(true, forKey: k)
        return true
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
        case .pairing, .join: untrusted(title)
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
