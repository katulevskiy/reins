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
    /// The passkey sheet for the passkeys that open the vault; the demo's answers by itself, tests put a fake here.
    @ObservationIgnored var passkeys: PasskeyPrompting
    /// The vault screens' state (Integrations > Password vault > Open the vault), shared by the list, an item and its
    /// form; emptied when the account changes.
    let vault = VaultModel()

    // MARK: State read from the core

    private(set) var session: SessionState = .loading
    private(set) var pending: [PendingItem] = []
    /// Everything that happened, newest first.
    private(set) var activity: [ActivityEntry] = []
    /// Entries read by Activity's search and its day pages, older than `activity` reaches; by id.
    private(set) var searchedEntries: [Int64: ActivityEntry] = [:]
    /// Activity's days opened or closed by hand this session, by day ("2026-10-15"); never saved.
    private(set) var activityOpenDays: [String: Bool] = [:]
    /// What Activity searches for and how it narrows it; kept while the user moves between tabs.
    var activitySearchText = ""
    var activityFilters = ActivityFilters()
    private(set) var grants: [GrantView] = []
    /// What a newly connected AI may do at first; nil until the user chose (it then asks for everything).
    private(set) var startingPolicy: StartingPolicy?
    private(set) var accounts: [AccountView] = []
    private(set) var services: [ServiceView] = []
    private(set) var connections: [ConnectionView] = []
    /// The server keeps the AI apps and computers to the approving phone and the phones it enrolled as viewers, and
    /// this phone is neither: Connections says where they are instead of "None yet".
    private(set) var connectionsHidden = false
    private(set) var mcpServers: [McpServerView] = []
    /// Autopilot's modes, bypasses and model; nil until first read.
    private(set) var autopilot: AutopilotSettings?
    private(set) var seenActivityId: Int64 = 0
    /// The server said another phone became the approval device.
    /// `-demoReplaced` (screenshots): the demo as if another phone had become the approval device.
    static var demoReplaced: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-demo") && ProcessInfo.processInfo.arguments.contains("-demoReplaced")
        #else
        false
        #endif
    }

    private(set) var deviceReplaced = false
    /// This phone currently receives approval requests.
    private(set) var approvalDevice = false
    /// Why registering this phone as the approval device failed, until it succeeds.
    var registrationError: String?
    /// A problem syncing with the account's other phone (from the core), a banner on Activity until dismissed. Sync
    /// never keeps the app on the welcome screen: this banner is all it shows.
    var syncNotice: String?
    /// The account was made without a master password and this phone keeps its secret: Settings offers the recovery
    /// code.
    private(set) var recoveryCodeAvailable = false
    /// Normal screens remain behind the mandatory acknowledgement, also after the app restarts.
    private(set) var recoveryToRecord: String?
    /// Before the recovery code, for an account without a passkey for its vault: the offer of one
    /// (`PasskeyOfferScreen`). The code follows either way.
    private(set) var passkeyOffer = false
    /// "Use the recovery code only" for this sign-in: the offer does not come back until the next one.
    private var passkeyDeclined = false
    private(set) var recoveryLoadError: String?
    /// How the last MCP sign-in ended, shown on that server's page until it is left.
    var mcpNotice: McpNotice?
    /// Verified connection receipts, kept across screen recreation until Done. Session UI, never permission state.
    var connectedAccounts: [String: String] = [:]
    var connectedServers: [String: McpServerView] = [:]

    func accountConnected(service: String, account: String, epoch: UInt64) {
        guard epoch == accountEpoch else { return }
        connectedAccounts[service] = account
    }

    func accountConnected(_ account: AccountView, epoch: UInt64) {
        guard epoch == accountEpoch else { return }
        accountConnected(service: account.service, account: account.account, epoch: epoch)
        if account.service == "gmail" {
            accounts.removeAll { $0.service == account.service && $0.account == account.account }
            accounts.append(account)
        }
        if let i = services.firstIndex(where: { $0.service == account.service }) {
            services[i].accounts.removeAll { $0.account == account.account }
            services[i].accounts.append(account)
        }
    }

    func serverConnected(_ server: McpServerView, epoch: UInt64) {
        guard epoch == accountEpoch else { return }
        setMcpServers(mcpServers.filter { $0.id != server.id } + [server])
        connectedServers[server.id] = server.status == "ok" ? server : nil
    }
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
    /// An answer folding into Activity's river.
    let fold = FoldState()
    /// Autopilot's reason lines, worked out once per entry.
    let reasons = ReasonMemory()
    var sheet: SheetTarget?
    /// The next "connect a computer" sheet opens on the code field rather than the camera (Settings' "Type code").
    var connectStartsTyped = false
    /// A one-line message over the content (a toast).
    var notice: String?
    /// Whether the "Expired" list on the Grants tab is open; kept while the user browses.
    var expiredOpen = false
    /// Changes for every visit to "New grant", so the form always starts empty.
    private(set) var newGrantToken = 0
    /// Items already popped up once; dismissing one leaves it in the list without popping up again.
    private var presented: Set<String> = []
    /// Requests a bypass, Lockdown, Overnight or Autopilot decided lately: kept out of `pending` even by a read made a
    /// moment before the decision (`AskGate`).
    private var autoDecided: [String] = []

    // MARK: Lifecycle

    /// The scene is active (in front and focused).
    private(set) var isActive = false
    /// The app is in the background (a push or a background refresh woke it, or it was just left): iOS gives it
    /// seconds, so `refreshPending` reads only what waits, and Autopilot does not load its model. Set by `AppHost`.
    private(set) var inBackground = false
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
    /// A newer model coming down by itself on Wi-Fi (no notification, sound or Live Activity), and "Check now".
    let modelAutoUpdate: ModelAutoUpdater

    init(core: any ReinsCoreProtocol, feedback: Feedback, authenticator: Authenticating, demo: Bool = false,
         modelDownloads: ModelDownloads? = nil, modelAutoUpdate: ModelAutoUpdater? = nil) {
        self.core = core
        self.feedback = feedback
        self.authenticator = authenticator
        self.demo = demo
        self.passkeys = demo ? DemoPasskeys() : PlatformPasskeys()
        self.modelDownloads = modelDownloads ?? ModelDownloads(core: core, feedback: feedback)
        self.modelAutoUpdate = modelAutoUpdate ?? ModelAutoUpdater(core: CoreModelUpdates(core: core))
        self.modelDownloads.onFinished = { [weak self] failure in
            await self?.refreshAutopilot()
            self?.onModelDownloadFinished?(failure)
        }
        self.modelAutoUpdate.canRun = { [weak self] in
            guard let self, case .signedIn = self.session else { return false }
            return true
        }
        self.modelAutoUpdate.downloadBusy = { [weak self] in self?.modelDownloads.job != .idle }
        self.modelAutoUpdate.wifiOnly = { [weak self] in self?.autopilot?.wifiOnly ?? true }
        self.modelAutoUpdate.onFinished = { [weak self] in await self?.refreshAutopilot() }
    }

    // MARK: Session

    /// The core's session. A read that fails (the store busy in the notification extension, a failure in the core)
    /// says nothing about the session: it is tried again a few times, then counts as signed out, as before reads
    /// could fail.
    private func readSession() async -> SessionInfo? {
        for attempt in 0..<3 {
            do {
                return try await core.session()
            } catch {
                if attempt < 2 { try? await Task.sleep(for: .milliseconds(400 << attempt)) }
            }
        }
        return nil
    }

    func refreshSession() async {
        let epoch = accountEpoch
        let info = await readSession()
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
        // Before the code: a passkey that opens the vault, unless the account has one or the offer was declined.
        var offerPasskey = false
        if let info, let code, !passkeyDeclined, !RecoveryRecord.confirmed(server: info.serverUrl, code: code) {
            offerPasskey = await hasNoVaultPasskey()
        }
        let current = await readSession()
        guard epoch == accountEpoch, current == info else { return }
        if let info, !demo, KeysLock.matches(info) || keys != .unlocked {
            setSession(.keysLocked(info))
            return
        }
        setSession(info.map(SessionState.signedIn) ?? .signedOut)
        if let info {
            if !demo { LastServer.value = info.serverUrl }
            DeviceStatus.selectAccount(info)
            seenActivityId = DeviceStatus.seenActivityId
            approvalDevice = DeviceStatus.approvalDevice && !DeviceStatus.replaced
            recoveryCodeAvailable = code != nil
            recoveryToRecord = code.flatMap { RecoveryRecord.confirmed(server: info.serverUrl, code: $0) ? nil : $0 }
            passkeyOffer = offerPasskey && recoveryToRecord != nil
            recoveryLoadError = recoveryFailure
            // Against the epoch of the session just set: signing in bumps it, so the one from before never matches.
            let signedIn = accountEpoch
            if let notice = try? await core.accountSyncNotice(), signedIn == accountEpoch { syncNotice = notice }
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
        autoDecided = []
        activity = []
        searchedEntries = [:]
        activityOpenDays = [:]
        activitySearchText = ""
        activityFilters = ActivityFilters()
        grants = []
        startingPolicy = nil
        accounts = []
        services = []
        connections = []
        connectionsHidden = false
        vault.reset()
        setMcpServers([])
        mcpNotice = nil
        connectedAccounts = [:]
        connectedServers = [:]
        approvalDevice = false
        recoveryCodeAvailable = false
        recoveryToRecord = nil
        passkeyOffer = false
        passkeyDeclined = false
        recoveryLoadError = nil
        syncNotice = nil
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
    /// Notifications are asked for on the setup's notifications page, or once the session is there (`onSignedIn`).
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

    /// The sync banner's OK.
    func dismissSyncNotice() async {
        syncNotice = nil
        try? await core.dismissAccountSyncNotice()
    }

    /// A sign-in whose account opened although the call reported a problem with it afterwards (an account sync
    /// conflict from a core before the merge): the phone is signed in, so it goes on signed in and the problem becomes
    /// the sync banner. Never an error on the welcome screen whose Continue only repeats it. False: not that case.
    func finishSignedInDespite(_ error: Error) async -> Bool {
        guard let failure = error as? CoreError, case .ServiceNeedsAttention = failure,
              let info = try? await core.session() else { return false }
        let keys = (try? await core.accountKeys()) ?? .unlocked
        await finishSso(SsoOutcome(session: info, keys: keys))
        syncNotice = error.userMessage
        return true
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

    /// Settings > Take the tour: the setup pages again, from the start.
    func startTour() {
        guard case .signedIn = session else { return }
        onboarding = true
    }

    /// Whether the account has no passkey for its vault yet. Unknown (offline) counts as having one: never a block.
    private func hasNoVaultPasskey() async -> Bool {
        (try? await core.vaultPasskeys().isEmpty) ?? false
    }

    /// "Use the recovery code only": the code shows next, and the offer does not come back for this sign-in.
    func declinePasskeyOffer() {
        passkeyDeclined = true
        passkeyOffer = false
    }

    /// A passkey opens the vault now: the offer is done, and the recovery code follows.
    func vaultPasskeyAdded() {
        passkeyOffer = false
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
                if self.inBackground {
                    await self.readWaiting()
                } else {
                    await self.readPending()
                }
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
            async let startingRead = core.startingPolicy()
            let newPending = AskGate.waiting(try await pendingRead, decided: autoDecided)
            let newActivity = try await activityRead
            let newGrants = try await grantsRead
            let newAccounts = try await accountsRead
            let newServices = try await servicesRead.filter { $0.service != "sms" }
            let newServers = try await serversRead
            let newStarting = try? await startingRead
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
            if startingPolicy != newStarting { startingPolicy = newStarting }
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
        deviceReplaced = DeviceStatus.replaced || Self.demoReplaced
        onPendingChanged?(pending)
        maybePresent()
        publish()
    }

    /// In the background: only what waits, for the notifications and the widgets. Everything else is read when the app
    /// comes to the front (`setActive`); a background wake has seconds, and the full read is a dozen calls.
    private func readWaiting() async {
        let epoch = accountEpoch
        let edits = listEdits
        do {
            let newPending = AskGate.waiting(try await core.pending(), decided: autoDecided)
            guard epoch == accountEpoch else { return }
            guard edits == listEdits else {
                refreshAgain = true
                return
            }
            if pending != newPending { pending = newPending }
        } catch CoreError.NotLoggedIn {
            if epoch == accountEpoch { sessionEnded() }
        } catch {
            // Busy or offline: the next wake or the front reads it.
        }
        guard epoch == accountEpoch else { return }
        deviceReplaced = DeviceStatus.replaced || Self.demoReplaced
        onPendingChanged?(pending)
        publish()
    }

    /// Whether a screen may start reading the store now. Not in the background: nobody looks, and iOS may suspend the app
    /// any moment (a read takes the store's lock in the shared container, which the app must not hold then); nor in
    /// the seconds before (`willResignActive` to `didEnterBackground`) when little time is left.
    var screensMayReadStore: Bool {
        !inBackground && UIApplication.shared.backgroundTimeRemaining > 5
    }

    /// The app went to the background or came back (`AppHost`): what a refresh reads, and whether Autopilot may load
    /// its model.
    func setInBackground(_ background: Bool) {
        inBackground = background
        core.setBackground(background: background)
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
            try? await endBypass(nil, in: settings)
        }
        for c in settings.connections where (c.bypassUntil ?? 0) > now {
            try? await endBypass(c.connectionId, in: settings)
        }
        await refreshAutopilot()
    }

    /// Ends the bypass of `connectionId` (nil: the global one) and puts back the mode under it. A bypass ended any
    /// Overnight of its scope when it started (the core returns to the mode that Overnight interrupted), so this never
    /// lands on Overnight: that only starts again through `startOvernight`, after the owner's check.
    func endBypass(_ connectionId: String?, in s: AutopilotSettings) async throws {
        let own = connectionId.flatMap { id in s.connections.first { $0.connectionId == id } }
        let base = connectionId == nil ? s.baseMode : own?.baseMode
        let back = base == .overnight ? OvernightReturn.mode(connectionId) : base
        try await core.setAutopilotMode(connectionId: connectionId, mode: back, minutes: nil)
    }

    /// The owner's check the core asks for itself before starting Overnight, changing its length or what it does with
    /// what is too risky: the same check as a bypass's.
    var ownerCheck: OwnerCheck { AuthenticatorOwnerCheck(authenticator) }

    /// Stops the Overnight of `connectionId` (nil: every AI's): back to the mode it interrupted. Setting that mode ends
    /// a bypass of the same scope too, which only ever makes Autopilot stricter.
    func endOvernight(_ connectionId: String?) async throws {
        try await core.setAutopilotMode(connectionId: connectionId, mode: OvernightReturn.mode(connectionId), minutes: nil)
        OvernightReturn.forget(connectionId)
    }

    /// Stops every Overnight now (the Live Activity's Stop, Siri): each goes back to the mode it interrupted.
    func stopOvernights() async {
        guard let settings = await refreshAutopilot() else { return }
        let now = Int64(Date().timeIntervalSince1970)
        if (settings.overnightUntil ?? 0) > now {
            try? await endOvernight(nil)
        }
        for c in settings.connections where (c.overnightUntil ?? 0) > now {
            try? await endOvernight(c.connectionId)
        }
        await refreshAutopilot()
    }

    /// The AI connections (a network call; failures keep the last list).
    func refreshConnections() async {
        let epoch = accountEpoch
        do {
            let list = try await core.connections()
            guard epoch == accountEpoch else { return }
            connections = list
            connectionsHidden = false
            publish()
        } catch CoreError.NotAViewer {
            // Not this phone's to read: say where they are, never "None yet".
            guard epoch == accountEpoch else { return }
            connections = []
            connectionsHidden = true
            publish()
        } catch {
            // Offline or signed out: the last list stays.
        }
    }

    /// Gives a connection the user's own name (nil or blank: it goes by its own again), kept with the account and
    /// synced by the core; then reads the connections and everything that shows their names again.
    func renameConnection(_ connectionId: String, to name: String?) async throws {
        _ = try await core.renameConnection(connectionId: connectionId, name: name)
        await refreshConnections()
        await refreshPending()
    }

    /// A change to a connection's blobatar from its Icon sheet.
    enum BlobatarChange: Equatable {
        case shape(UInt8)
        case colour(UInt8)
        case shuffle
    }

    /// Changes a connection's blobatar (kept and synced by the core) and, when the connection shows something else
    /// (a logo), makes it show the blobatar, so the change is seen. Then reads the connections again.
    func changeBlobatar(_ connectionId: String, _ change: BlobatarChange) async throws {
        guard let connection = connection(connectionId) else { return }
        let kept = connection.blobatar
        switch change {
        case let .shape(shape):
            _ = try await core.setConnectionBlobatar(connectionId: connectionId, shape: shape, colour: kept?.colour ?? 0)
        case let .colour(colour):
            _ = try await core.setConnectionBlobatar(connectionId: connectionId, shape: kept?.shape ?? 0, colour: colour)
        case .shuffle:
            _ = try await core.shuffleConnectionBlobatar(connectionId: connectionId)
        }
        if !ConnectionLook.showsBlob(connection) {
            try await core.setConnectionIcon(connectionId: connectionId, icon: Providers.blob)
        }
        await refreshConnections()
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

    /// Sets the starting rule for connections made from now on; the screen follows at once.
    func chooseStartingPolicy(_ policy: StartingPolicy) {
        startingPolicy = policy
        Task {
            do {
                try await core.setStartingPolicy(policy: policy)
            } catch {
                startingPolicy = try? await core.startingPolicy()
            }
        }
    }

    /// An answer the screen has already moved on from (the Android app's `answerInBackground`): the item leaves the list
    /// now and `work` runs without anyone waiting for it. If it fails, the item comes back with the next read and a few
    /// words say why ("Not approved: …").
    func answerInBackground(_ id: String, failed: String, work: @escaping @Sendable () async throws -> Void) {
        if let item = pending.first(where: { $0.id == id }) {
            fold.decided(FoldRequest(
                field: ActivityText.field(item, danger: false), connectionId: item.connectionId, action: item.action,
                after: activity.first?.id ?? 0, fromSheet: sheet?.isApproval == true
            ))
        }
        dropPending(id)
        Task {
            do {
                try await work()
            } catch {
                feedback.play(.error)
                notice = "\(failed): \(decisionErrorMessage(error))"
            }
            await refreshPending()
        }
    }

    /// A bypass, Lockdown, Overnight or Autopilot decided `id` (`AppNotifier.autoDecided`): it leaves the list now and
    /// stays out of it, and an ask open on it closes, since the owner has nothing left to answer.
    func decidedAutomatically(_ id: String) {
        autoDecided = AskGate.remember(id, in: autoDecided)
        dropPending(id)
        if AskGate.closes(sheet, decided: id) { closeSheet() }
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

    /// An entry by id: from the history read at once, else from what a search or an opened day brought in.
    func entry(_ id: Int64) -> ActivityEntry? {
        activity.first { $0.id == id } ?? searchedEntries[id]
    }

    /// Keeps the entries a search or a day page read, so opening one finds it beyond the history read at once.
    func remember(_ entries: [ActivityEntry]) {
        guard !entries.isEmpty else { return }
        if searchedEntries.count > 2_000 { searchedEntries.removeAll() }
        for e in entries where searchedEntries[e.id] != e { searchedEntries[e.id] = e }
    }

    /// An entry by id, however far back: what `entry(_:)` has, else read from the core by its id (the history read at
    /// once holds only the newest, and what searches and days brought in is dropped past 2,000). Kept once read; nil
    /// when the core has no such entry.
    func loadEntry(_ id: Int64) async throws -> ActivityEntry? {
        if let known = entry(id) { return known }
        guard let read = try await core.activityEntry(entryId: id) else { return nil }
        remember([read])
        return read
    }

    /// Whether Activity's day `key` ("2026-10-15") is open; today's is unless closed. In memory for the session only.
    func activityDayOpen(_ key: String, today: String) -> Bool { activityOpenDays[key] ?? (key == today) }

    func setActivityDay(_ key: String, open: Bool) { activityOpenDays[key] = open }

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

    // MARK: Switching profiles

    /// Before the app moves to another profile: the long poll stops, and nothing outside the model (the Live
    /// Activities, the notification sweep, grant reminders, the sign-in prompt) follows this account any more. Work
    /// already under way finishes on its own; the core closes once the last task holding it ends.
    func shutDownForSwitch() {
        syncTask?.cancel()
        syncTask = nil
        isActive = false
        onPendingChanged = nil
        onAutopilotChanged = nil
        onGrantsChanged = nil
        onModelDownloadFinished = nil
        onSignedIn = nil
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
                guard let self, case .signedIn = self.session, self.recoveryToRecord == nil, self.recoveryLoadError == nil else {
                    try? await Task.sleep(for: .seconds(2))
                    continue
                }
                // A phone that views the account follows its log here (the core's `sync`), whatever it once was.
                let role = (try? await self.core.phoneRole()) ?? .legacy
                guard Self.foregroundLoopRuns(role: role, deviceReplaced: self.deviceReplaced) else {
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

    /// Whether the foreground loop runs: on the approving phone, and on a phone that views the account (phone roles),
    /// which follows the account's log with it (Activity, the modes and their timers), even when it was the approval
    /// device once and was replaced. A replaced phone that views nothing waits (its poll is refused).
    nonisolated static func foregroundLoopRuns(role: PhoneRole, deviceReplaced: Bool) -> Bool {
        switch role {
        case .viewer, .primary: true
        case .legacy, .removed: !deviceReplaced
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

    /// A place's root, with nothing open on it (Autopilot settings is Trust's root, Integrations is Connections').
    func open(_ target: AppSection) {
        paths[target] = []
        section = target
    }

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
        guard let next = AskGate.next(pending, presented: presented) else { return }
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
            return "Code expired. Show a new one on your computer."
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
            section = .trust
            paths[.trust] = grants.contains { $0.id == id } ? [.grantDetail(id)] : []
        case .autopilot:
            open(.trust)
        case .integrations:
            open(.connections)
        case .home:
            home()
        case .orgReviews:
            // Only a work profile has them; the Personal one opens Settings.
            if WorkProfiles.active.managed { show(.orgReviews, in: .settings) } else { open(.settings) }
        case .pair:
            break
        }
    }

    // MARK: Widgets

    /// An item as widgets and Live Activities show it, with the icon its connection shows and the blobatar it keeps.
    func snapshotItem(_ item: PendingItem) -> Snapshot.Item {
        var s = item.snapshotItem
        let c = connection(item.connectionId)
        s.connectionIcon = c.flatMap(ConnectionLook.shownIcon)
        s.connectionBlobatar = c.flatMap(ConnectionLook.blobatar)
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
            s.overnightUntil = a.overnightUntil
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
        WidgetReloads.shared.request()
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
        case .auto: "auto"
        case .overnight: "overnight"
        case .bypass: "bypass"
        case .lockdown: "lockdown"
        }
    }

    /// Manual < Auto < Overnight < Bypass; Lockdown is the least autonomous.
    var autonomy: Int { AutopilotText.autonomy(self) }
}

extension PendingItem {
    /// How long the server holds a request for an answer (the relay's TTL).
    static let answerWindow: Int64 = 600

    var expiresAt: Int64 { waitUntil ?? (createdAt + Self.answerWindow) }

    /// "Claude: Search Gmail for …" (ASK-TEXT; the core's `headline` is the sentence of what approving does).
    var listTitle: String { AskLabels.listTitle(self) }

    var snapshotItem: Snapshot.Item {
        Snapshot.Item(
            id: id,
            kind: snapshotKind,
            title: listTitle,
            subtitle: untrusted(subtitle),
            connection: untrusted(connectionLabel),
            service: service,
            createdAt: createdAt,
            expiresAt: expiresAt,
            suggestion: suggestion,
            quick: kind == .request && quick,
            field: field.rawValue,
            label: AskLabels.pending(self, field: field).kind
        )
    }

    /// The field it is painted in outside the app, as on its waiting card (`ActivityText.field`).
    private var field: FieldKind { ActivityText.field(self, danger: false) }
}

extension ActivityEntry {
    var headline: String {
        entryTitle(label: connectionLabel, action: action, count: Int(count), service: service, outcome: outcome, title: opTitle, op: op)
    }

    var snapshotEntry: Snapshot.Entry {
        Snapshot.Entry(
            id: id, title: headline, outcome: outcome.capitalized, approved: !["denied", "failed", "expired"].contains(outcome), at: at,
            service: service, action: action
        )
    }
}
