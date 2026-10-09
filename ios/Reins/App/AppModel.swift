import CryptoKit
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
    /// Signed in, but the server would not make this phone the approval device: the account has another one, and this
    /// phone could not prove it may take over (it holds neither the account's secret nor the password it signed in
    /// with). The Unlock screen shows with that reason: the other phone approves this one, or the recovery code (or
    /// the master password) gives it the proof. Not kept across launches: the next launch's registration is refused
    /// again.
    case otherApprovalDevice(SessionInfo)

    /// The account the Unlock screen is about.
    var unlocking: SessionInfo? {
        switch self {
        case let .keysLocked(info), let .otherApprovalDevice(info): info
        case .loading, .signedOut, .signedIn: nil
        }
    }
}

/// The account a passwordless sign-in left locked on this phone (server and email), kept across launches so a restart
/// comes back to the Unlock screen instead of registering the phone.
enum KeysLock {
    private static let key = "account.keysLocked"

    static func set(_ info: SessionInfo) {
        let data = Data("\(info.serverUrl)|\(info.email)".utf8)
        guard let sealed = try? SealedSnapshot.seal(data) else { return }
        AppGroup.defaults.set(sealed, forKey: key)
    }

    static func matches(_ info: SessionInfo) -> Bool {
        let value = "\(info.serverUrl)|\(info.email)"
        if let legacy = AppGroup.defaults.string(forKey: key) {
            AppGroup.defaults.removeObject(forKey: key)
            if legacy == value { set(info); return true }
        }
        guard let data = AppGroup.defaults.data(forKey: key), let plain = try? SealedSnapshot.open(data) else { return false }
        return String(data: plain, encoding: .utf8) == value
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

    private static func read(_ key: String) -> String? {
        guard let data = d.data(forKey: key), let plain = try? SealedSnapshot.open(data),
              let value = String(data: plain, encoding: .utf8), value.hasPrefix(key + "\u{0}") else { return nil }
        return String(value.dropFirst(key.count + 1))
    }
    private static func write(_ value: String, key: String) {
        guard let data = try? SealedSnapshot.seal(Data((key + "\u{0}" + value).utf8)) else {
            d.removeObject(forKey: key)
            return
        }
        d.set(data, forKey: key)
    }
    static func selectAccount(_ info: SessionInfo) {
        let owner = Snapshot.owner(server: info.serverUrl, email: info.email)
        if read("device.account") != owner {
            clear()
            write(owner, key: "device.account")
        }
    }
    static var replaced: Bool {
        get { read(replacedKey) == "true" }
        set {
            write(String(newValue), key: replacedKey)
            if newValue { write("false", key: approvalKey) }
        }
    }
    static var approvalDevice: Bool {
        get { read(approvalKey) == "true" }
        set { write(String(newValue), key: approvalKey) }
    }
    static var seenActivityId: Int64 {
        get { read(seenKey).flatMap(Int64.init) ?? 0 }
        set { write(String(newValue), key: seenKey) }
    }
    static func clear() {
        [replacedKey, approvalKey, seenKey, "device.account"].forEach(d.removeObject(forKey:))
    }
}

/// Everything the screens share: the core, the state read from it, navigation, and the lifecycle around them (the
/// Android app's `AppContainer`, `AppState` and `AppViewModel` in one). Main-actor bound; the core's calls are async
/// and never block it.
@Observable
@MainActor
final class AppModel {
    let core: any ReinsCoreProtocol
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
    /// Normal screens remain behind the mandatory acknowledgement, also after the app restarts.
    private(set) var recoveryToRecord: String?
    private(set) var recoveryLoadError: String?
    /// How the last MCP sign-in ended, shown on that server's page until it is left.
    var mcpNotice: McpNotice?
    /// After a fresh sign-in (or a new account) on the sign-in screen: the steps that connect a computer and an AI
    /// show instead of the app until "Done".
    private(set) var onboarding = false
    /// The onboarding steps were due when the approval role was refused (`otherApprovalDevice`): they show once this
    /// phone gets it.
    private var onboardingAfterUnlock = false
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

    init(core: any ReinsCoreProtocol, feedback: Feedback, authenticator: Authenticating, demo: Bool = false,
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
        let epoch = accountEpoch
        let info = await core.session()
        guard epoch == accountEpoch else { return }
        var keys: AccountKeys?
        if info != nil && !demo { keys = try? await core.accountKeys() }
        var recoveryFailure: String?
        var code: String?
        if info != nil {
            do { code = try await core.accountRecoveryCode() }
            catch CoreError.Invalid { /* Password accounts have no recovery secret. */ }
            catch { recoveryFailure = error.userMessage }
        }
        let current = await core.session()
        guard epoch == accountEpoch, current == info else { return }
        if let info, !demo, KeysLock.matches(info) || keys != .unlocked {
            setSession(.keysLocked(info))
            return
        }
        setSession(info.map(SessionState.signedIn) ?? .signedOut)
        if let info {
            DeviceStatus.selectAccount(info)
            seenActivityId = DeviceStatus.seenActivityId
            approvalDevice = DeviceStatus.approvalDevice && !DeviceStatus.replaced
            recoveryCodeAvailable = code != nil
            recoveryToRecord = code.flatMap { RecoveryRecord.confirmed(server: info.serverUrl, code: $0) ? nil : $0 }
            recoveryLoadError = recoveryFailure
            onSignedIn?()
            await refreshPending()
            await refreshConnections()
            if !DeviceStatus.replaced { await registerDeviceQuietly() }
        }
    }

    private(set) var accountEpoch: UInt64 = 0
    private func identity(_ state: SessionState) -> String? {
        if case let .signedIn(info) = state { return info.serverUrl + "\u{0}" + info.email }
        return nil
    }
    func setSession(_ state: SessionState) {
        let previous = identity(session)
        let next = identity(state)
        if previous != next { accountEpoch &+= 1 }
        session = state
        if next != nil && (previous == nil || previous == next) { return }
        syncTask?.cancel()
        syncTask = nil
        seenActivityId = 0
        deviceReplaced = false
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
        recoveryToRecord = nil
        recoveryLoadError = nil
        onboarding = false
        autopilot = nil
        sheet = nil
        paths = [:]
        section = .activity
        onPendingChanged?([])
        onGrantsChanged?([])
        publish()
    }

    /// The server ended the session (signed out elsewhere, the account removed): back to the sign-in screen, with the
    /// alert, since nothing the user did here explains it.
    private func sessionEnded() {
        if case .signedIn = session { feedback.play(.alert) }
        setSession(.signedOut)
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
        onboarding = demo || onboardingAfterUnlock || OnboardingRecord.firstTime(server: info.serverUrl, email: info.email)
        onboardingAfterUnlock = false
        await signedIn(info)
        // The quiet registration may already have been refused (`otherApprovalDevice`).
        if case .signedIn = session, !approvalDevice {
            do {
                try await registerDevice(force: true)
            } catch CoreError.OtherApprovalDevice {
                // The Unlock screen shows why.
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

    /// The Unlock screen opened the keys, or brought the proof that lets this phone take the approval role (the other
    /// phone approved, or the recovery code): on as after a sign-in.
    func finishUnlock() async {
        switch session {
        case let .keysLocked(info):
            KeysLock.clear()
            await finishSignIn(info)
        case let .otherApprovalDevice(info):
            await finishSignIn(info)
        case .loading, .signedOut, .signedIn:
            return
        }
    }

    func finishOnboarding() {
        guard recoveryToRecord == nil else { return }
        onboarding = false
    }

    func confirmRecoveryRecord() {
        guard case let .signedIn(info) = session, let code = recoveryToRecord else { return }
        RecoveryRecord.confirm(server: info.serverUrl, code: code)
        recoveryToRecord = nil
    }

    func signOut() async {
        let logoutUrl: String?
        do {
            logoutUrl = try await core.logoutWithBrowser()
        } catch {
            notice = error.userMessage
            return
        }
        forgetSignedInAccount()
        if let logoutUrl, let url = URL(string: logoutUrl) {
            // Use the authentication sheet's shared cookie store, like sign-in. Cancelling it never unlocks
            // the account again; subsequent mobile sign-in requires fresh authentication on the server.
            _ = try? await WebAuth.run(url, callbackScheme: "com.reins2fa.app")
        }
    }

    /// "Delete account": the core deletes the account on the server for good (with its WorkOS user) and this phone's
    /// encrypted copy, then the phone forgets it as signing out does. A refusal throws and changes nothing. The
    /// demo core only signs out.
    func deleteAccount(confirmEmail: String) async throws {
        try await core.deleteAccount(confirmEmail: confirmEmail)
        forgetSignedInAccount()
    }

    /// What signing out and deleting the account both forget on this phone besides the core's own data.
    private func forgetSignedInAccount() {
        KeysLock.clear()
        DeviceStatus.clear()
        approvalDevice = false
        deviceReplaced = false
        onboardingAfterUnlock = false
        setSession(.signedOut)
    }

    // MARK: Refreshing

    /// The re-read running now, and whether another was asked for while it ran.
    @ObservationIgnored private var refreshing: Task<Void, Never>?
    @ObservationIgnored private var refreshAgain = false
    /// Count the changes shown before the core confirmed them (lists, Autopilot): a read that began before one of
    /// them may predate it, so it is not shown.
    @ObservationIgnored private var listEdits = 0
    @ObservationIgnored private var autopilotEdits = 0

    /// Re-reads everything held on the phone: what waits, what happened, which permissions exist. Calls that come
    /// while a re-read runs (a burst of core callbacks, the sync loop) share one more pass after it, so each caller
    /// still sees state read after it asked.
    func refreshPending() async {
        if let running = refreshing {
            refreshAgain = true
            await running.value
            return
        }
        let task = Task { [weak self] in
            guard let self else { return }
            repeat {
                self.refreshAgain = false
                await self.readPending()
            } while self.refreshAgain
            self.refreshing = nil
        }
        refreshing = task
        await task.value
    }

    private func readPending() async {
        let epoch = accountEpoch
        let edits = listEdits
        let core = self.core
        do {
            // Independent reads: side by side rather than one after another.
            async let pendingRead = core.pending()
            async let activityRead = core.activity(limit: 300)
            async let grantsRead = core.grants()
            async let accountsRead = core.accounts()
            async let servicesRead = core.services()
            async let serversRead = core.mcpServers()
            let newPending = try await pendingRead
            let newActivity = try await activityRead
            let newGrants = try await grantsRead
            let newAccounts = try await accountsRead
            let newServices = try await servicesRead.filter { $0.service != "sms" }
            let newServers = try await serversRead
            guard epoch == accountEpoch else { return }
            guard edits == listEdits else {
                // Read before a change shown ahead of the core: read once more instead.
                refreshAgain = true
                return
            }
            // Unchanged lists are left alone, so the screens reading them do not draw again.
            if pending != newPending { pending = newPending }
            if activity != newActivity { activity = newActivity }
            if grants != newGrants { grants = newGrants }
            if accounts != newAccounts { accounts = newAccounts }
            if services != newServices { services = newServices }
            if mcpServers != newServers { setMcpServers(newServers) }
            await refreshAutopilot()
            guard epoch == accountEpoch else { return }
            onGrantsChanged?(grants)
        } catch CoreError.NotLoggedIn {
            if epoch == accountEpoch { sessionEnded() }
        } catch {
            // Offline: the current account keeps its last decrypted view.
        }
        guard epoch == accountEpoch else { return }
        deviceReplaced = DeviceStatus.replaced
        onPendingChanged?(pending)
        maybePresent()
        publish()
    }

    /// Re-reads Autopilot (modes, bypasses, model) and keeps what shows it in step.
    @discardableResult
    func refreshAutopilot() async -> AutopilotSettings? {
        let epoch = accountEpoch
        let edits = autopilotEdits
        guard let settings = try? await core.autopilotSettings(), epoch == accountEpoch else { return nil }
        // Read before a change shown ahead of the core: the read after that change shows it.
        guard edits == autopilotEdits else { return settings }
        if autopilot != settings { autopilot = settings }
        onAutopilotChanged?(settings)
        publish()
        return settings
    }

    /// Shows a change to Autopilot's settings at once, before the core has it: the next `refreshAutopilot` puts what
    /// the core holds in its place (the same, or the old value back after a refusal).
    func patchAutopilot(_ change: (inout AutopilotSettings) -> Void) {
        guard var s = autopilot else { return }
        change(&s)
        autopilotEdits += 1
        if s != autopilot { autopilot = s }
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
        let epoch = accountEpoch
        if let list = try? await core.connections(), epoch == accountEpoch {
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

    /// Re-reads only the MCP servers (after a change to one of them; `refreshPending` reads everything).
    func refreshMcpServers() async {
        let epoch = accountEpoch
        let edits = listEdits
        guard let list = try? await core.mcpServers(), epoch == accountEpoch, edits == listEdits, list != mcpServers else { return }
        setMcpServers(list)
    }

    /// Shows a tool's "large results" switch flipped at once, before the core has it.
    func patchMcpTool(serverId: String, tool: String, heavy: Bool) {
        guard let s = mcpServers.firstIndex(where: { $0.id == serverId }),
              let t = mcpServers[s].tools.firstIndex(where: { $0.name == tool }) else { return }
        listEdits += 1
        mcpServers[s].tools[t].heavy = heavy
    }

    /// An item the user just answered leaves the list now; the re-read that follows confirms it.
    func dropPending(_ id: String) {
        listEdits += 1
        pending.removeAll { $0.id == id }
    }

    /// A grant the user just ended or deleted shows that now; the re-read that follows confirms it.
    func grantChanged(_ id: String, deleted: Bool) {
        listEdits += 1
        if deleted {
            grants.removeAll { $0.id == id }
        } else if let i = grants.firstIndex(where: { $0.id == id }) {
            grants[i].active = false
            grants[i].state = "revoked"
        }
    }

    /// Re-reads everything without the caller waiting for it.
    func refreshPendingSoon() {
        Task { await refreshPending() }
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
    ///
    /// The server refuses a phone that is not the approval device and brings no proof it may take over
    /// (`CoreError.OtherApprovalDevice`): the Unlock screen then shows, or, for a quiet registration of a phone that held
    /// the role, it learns it was replaced.
    func registerDevice(force: Bool) async throws {
        if DeviceStatus.replaced && !force { return }
        let token = registrationToken
        do {
            do {
                try await core.registerDevice(fcmToken: token)
            } catch let CoreError.Server(status, _) where status == 400 && token != nil {
                // A server that refuses the push token still takes the phone, which then gets requests while the app
                // is open: servers from before October 2026 accept only 32-byte APNs tokens, a simulator's are 80 bytes.
                try await core.registerDevice(fcmToken: nil)
            }
        } catch CoreError.OtherApprovalDevice {
            takeoverRefused(force: force)
            throw CoreError.OtherApprovalDevice
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
        } catch CoreError.OtherApprovalDevice {
            // The Unlock screen, or the "replaced" banner, says it.
        } catch {
            registrationError = error.userMessage
        }
    }

    /// Another phone approves for the account and this one could not prove it may take over. A phone that held the role
    /// and only refreshed its registration lost it without hearing (a missed `replaced` push): it shows as replaced,
    /// and "Use this phone" in Settings comes back here with `force`. Otherwise the Unlock screen offers the two ways.
    private func takeoverRefused(force: Bool) {
        if !force && approvalDevice {
            markReplaced()
            return
        }
        guard case let .signedIn(info) = session else { return }
        onboardingAfterUnlock = onboarding
        registrationError = nil
        setSession(.otherApprovalDevice(info))
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
                guard let self, case .signedIn = self.session, !self.deviceReplaced, self.recoveryToRecord == nil, self.recoveryLoadError == nil else {
                    try? await Task.sleep(for: .seconds(2))
                    continue
                }
                do {
                    _ = try await self.core.sync(waitSecs: 25)
                    if Task.isCancelled { return }
                    // Requests that grants answered by themselves leave no prompt, only a new activity entry.
                    await self.refreshPending()
                    failures = 0
                } catch CoreError.NotLoggedIn {
                    if Task.isCancelled { return }
                    self.sessionEnded()
                } catch let CoreError.Server(status, _) where status == 403 && self.approvalDevice {
                    if Task.isCancelled { return }
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
        if case let .signedIn(info) = session {
            s.accountFingerprint = Snapshot.owner(server: info.serverUrl, email: info.email)
        }
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
        // What this process wrote last, while it is still what is stored (the notification extension writes it too):
        // no keychain read and no decryption for the common case of nothing new.
        let stored = AppGroup.defaults.data(forKey: Snapshot.key)
        let previous: Snapshot
        if let published, stored != nil, stored == published.sealed {
            previous = published.snapshot
        } else {
            previous = Snapshot.load()
        }
        s.updatedAt = previous.updatedAt
        guard s != previous || s.updatedAt == 0 else { return }
        s.updatedAt = Int64(Date().timeIntervalSince1970)
        s.save()
        published = AppGroup.defaults.data(forKey: Snapshot.key).map { (snapshot: s, sealed: $0) }
        WidgetCenter.shared.reloadAllTimelines()
        ControlCenter.shared.reloadAllControls()
    }

    /// The snapshot `publish` saved last, and the sealed bytes it stored.
    @ObservationIgnored private var published: (snapshot: Snapshot, sealed: Data)?
}

/// Which accounts have been through the onboarding steps on this phone: they show once per account (server and
/// email), after it signs in on the sign-in screen, so never for a session from before they existed.
enum OnboardingRecord {
    static func key(server: String, email: String) -> String {
        var s = server.trimmingCharacters(in: .whitespaces).lowercased()
        while s.hasSuffix("/") { s.removeLast() }
        let value = "\(s)|\(email.trimmingCharacters(in: .whitespaces).lowercased())"
        return "onboarded:" + SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    /// True the first time `email` on `server` signs in on this phone; records that it did.
    static func firstTime(server: String, email: String, defaults: UserDefaults = AppGroup.defaults) -> Bool {
        defaults.dictionaryRepresentation().keys.filter { $0.hasPrefix("onboarded:") && $0.contains("@") }.forEach { defaults.removeObject(forKey: $0) }
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
