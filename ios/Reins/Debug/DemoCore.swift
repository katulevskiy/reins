import Foundation

/// The `-demo` launch argument's in-memory core (screenshots, UI tests, a look around without a server).
///
/// Launch arguments: `-signedout` starts signed out (any password signs in; an email containing "2fa" also needs a
/// code; creating an account works like signing in), `-demoArrive` makes a new request arrive about 6 s after launch,
/// `-demoNoModel` starts with the Autopilot model not downloaded, `-demoOldModel` with an older one that has an update.
/// Any pairing code pairs a desktop app, except those starting with "BBBB", which have expired. "Continue"
/// (passwordless sign-in) makes a new account; `-demoLocked` finds one whose keys are on another phone instead (the recovery code is `DemoData.recoveryCode`; asking the other phone
/// works after two polls, `-demoJoinDenied` refuses; resetting the vault starts it over as a new account).
/// `-demoJoin` has another phone ask this one for the keys.
/// `-demoQuiet`: nothing waits (Activity's pond and its mood line).
/// `-demoOtherPhone`: another phone approves for the account, and registering this one is refused
/// (`CoreError.OtherApprovalDevice`) until the recovery code or the other phone's approval gives it a proof.
enum DemoCore {
    static func make() -> (any ReinsCoreProtocol)? {
        let args = ProcessInfo.processInfo.arguments
        let otherPhone = args.contains("-demoOtherPhone")
        // Existing seeded demo accounts already completed recovery setup. A fresh sign-in exercises the gate.
        let recoveryKey = RecoveryRecord.key(server: DemoData.server, code: DemoData.recoveryCode)
        if args.contains("-signedout") || args.contains("-demoRecoverySetup") {
            AppGroup.defaults.removeObject(forKey: recoveryKey)
        } else {
            AppGroup.defaults.set(true, forKey: recoveryKey)
        }
        // This phone never held the role: the refusal shows the Unlock screen, not the "replaced" banner.
        if otherPhone { DeviceStatus.clear() }
        return DemoReinsCore(
            signedIn: !args.contains("-signedout"),
            arriveAfter: args.contains("-demoArrive") ? 6 : nil,
            modelInstalled: !args.contains("-demoNoModel"),
            modelOutdated: args.contains("-demoOldModel"),
            keysLocked: args.contains("-demoLocked"),
            joinWaiting: args.contains("-demoJoin"),
            approvalElsewhere: otherPhone,
            passkeys: !args.contains("-demoNoPasskeys"),
            passkeysOffline: args.contains("-demoPasskeysOffline"),
            quiet: args.contains("-demoQuiet")
        )
    }
}

/// A port of Android's test `FakeCore`, seeded with `DemoData`: it keeps state and behaves enough like the real core
/// for every UI flow (approving removes the item and logs it, a standing choice makes a grant, and so on).
/// All state sits behind one lock; nothing is held across a suspension point.
final class DemoReinsCore: ReinsCoreProtocol, @unchecked Sendable {
    /// A connection's own Autopilot row, as the core stores it.
    struct ApRow: Equatable {
        var mode: AutopilotMode?
        var bypassUntil: Int64?
        var profileId: String?
        /// When the running bypass started.
        var bypassSince: Int64?
        /// With `mode` Overnight: its end (`Int64.max` until turned off), its start and the mode it interrupted.
        var overnightUntil: Int64?
        var overnightSince: Int64?
        var overnightReturn: AutopilotMode?

        /// Overnight is set and has not reached its end.
        func overnighting(_ now: Int64) -> Bool { mode == .overnight && (overnightUntil ?? .max) > now }

        /// The mode set when no bypass runs: an Overnight that ended reads as the mode it interrupted.
        func base(_ now: Int64) -> AutopilotMode? {
            mode == .overnight && !overnighting(now) ? overnightReturn.flatMap { $0 == .overnight ? nil : $0 } : mode
        }
    }

    struct State {
        var session: SessionInfo?
        /// `-demoSyncNotice`'s OK was tapped.
        var syncNoticeDismissed = false
        var pending: [PendingItem] = []
        /// Bumped by every change to `pending`, so a waiting `sync` can return early.
        var pendingVersion = 0
        var views: [String: ApprovalView] = [:]
        /// The approval each request decided here showed, by its Activity entry (the seeded history has none, like
        /// entries logged before the core kept them).
        var recorded: [Int64: ApprovalView] = [:]
        /// `-demoSyncedPush`: the push entry 14 shows came from another device without its changes line by line; this
        /// is the push with them, what "Load from GitHub" brings. `syncedFails`: GitHub cannot be reached.
        var syncedPush: GitPushView?
        var syncedFails = false
        var pairings: [String: PairingView] = [:]
        var blobs: [String: BlobView] = [:]
        var connections: [ConnectionView] = []
        var activity: [ActivityEntry] = []
        var grants: [GrantView] = []
        var accounts: [AccountView] = []
        var setupRole: PhoneRole = .legacy
        var setupRoster: RosterSummary?
        var setupRequiresProof = false
        var setupCreated = 0
        var permissionViews: [String: AccountPermissionsView] = [:]
        var delegationRecords: [DelegationView] = []
        var accountStatuses: [String: GmailStatus] = [:]
        var emails: [String: EmailContent] = [:]
        var mcp: [McpServerView] = []
        var telegramPhone = ""
        var apGlobal = ApRow()
        var apConnections: [String: ApRow] = [:]
        var model: ModelStatus
        var profiles: [ProfileView] = []
        var wifiOnly = true
        /// "Update automatically" (on by default, as in the core).
        var autoUpdate = true
        var overnightWhenRisky: OvernightRisky = .wait
        var suggestions: [String: SuggestionView] = [:]
        var arriveAt: Int64?
        var nextGrant = 100
        var nextConnection = 100
        /// What a passwordless sign-in finds: a new account, or keys another phone has.
        var keys = AccountKeys.created
        /// This phone asked its other phone for the keys: how often it asked since.
        var joinPolls: Int?
        /// Other phones asking this one for the account's keys.
        var joins: [String: JoinView] = [:]
        /// Another phone approves for the account: `registerDevice` is refused until this one brings a proof.
        var approvalElsewhere = false
        var hasAccountSecret = true
        /// What opens the account's keys; a reset makes a new one (`resetRecoveryCode`).
        var recoveryCode = DemoData.recoveryCode
        /// The passkeys that open the vault (one, "iPhone 16", unless `-demoNoPasskeys`).
        var passkeys: [VaultPasskeyView] = []
        /// `vaultPasskeys` fails as if offline (`-demoPasskeysOffline`).
        var passkeysOffline = false
        /// The vault on the phone and its secret values ("id/key").
        var vault: [VaultItemDetail] = DemoReinsCore.demoVault
        var vaultSecrets: [String: String] = ["openai/password": "sk-demo-0000", "github/password": "hunter2"]
        var devices: [DeviceView] = DemoReinsCore.demoDevices
    }

    /// The passkey the demo account starts with.
    static let demoPasskey = VaultPasskeyView(credentialId: Data("demo-passkey-1".utf8), name: "iPhone 16", createdAt: 1_760_000_000)
    /// What every passkey's PRF answers with in the demo: 32 bytes, like the real output.
    static let demoPrfOutput = Data(repeating: 7, count: 32)

    /// The recovery code a reset vault gets.
    static let resetRecoveryCode = "HV3N-Q8RT-ZL2K-M7WD-PX4C-BJ9F-E6YS-NA5G-UT3R-KC8M-WQ2H-FD7L-YP4X"

    private let lock = NSLock()
    private var state: State
    /// The longest a `sync` waits, so tests need not wait the real 25 s.
    private let syncCap: Double

    init(
        signedIn: Bool = true, arriveAfter: Int64? = nil, modelInstalled: Bool = true, modelOutdated: Bool = false, syncCap: Double = 5,
        keysLocked: Bool = false, joinWaiting: Bool = false, approvalElsewhere: Bool = false, passkeys: Bool = true, passkeysOffline: Bool = false,
        quiet: Bool = false
    ) {
        let now = Self.now()
        let seeded = DemoData.pending(now)
        var s = State(model: modelInstalled
            ? DemoData.model(.installed, update: modelOutdated, updatedAt: now - 3 * 86_400)
            : DemoData.model(.notInstalled))
        s.session = signedIn ? SessionInfo(serverUrl: DemoData.server, email: DemoData.email) : nil
        s.pending = quiet ? [] : seeded.items
        s.views = seeded.views
        s.pairings = seeded.pairings
        s.blobs = seeded.blobs
        s.connections = DemoData.connections(now)
        s.activity = DemoData.activity(now)
        s.grants = DemoData.grants(now)
        s.accounts = DemoData.accounts(now)
        s.accountStatuses = ["gmail/work@corp.example": .needsConsent]
        s.emails = DemoData.emails(now)
        s.mcp = DemoData.mcpServers()
        s.profiles = DemoData.profiles(now)
        s.suggestions = DemoData.suggestions(now)
        s.apConnections = ["c1": ApRow(mode: .auto, bypassUntil: nil, profileId: "work")]
        s.arriveAt = arriveAfter.map { now + $0 }
        s.keys = keysLocked ? .locked : .created
        s.approvalElsewhere = approvalElsewhere
        s.passkeys = passkeys ? [Self.demoPasskey] : []
        s.passkeysOffline = passkeysOffline
        // `-demoTemporary` (screenshots): one temporary permission for me@gmail.com, as the Permissions board shows it.
        if ProcessInfo.processInfo.arguments.contains("-demoTemporary") {
            s.delegationRecords = [DelegationView(
                id: "temporary-0", revision: 1, title: "Selected agents", service: "gmail", account: "me@gmail.com", state: .active,
                expiresAt: now + 86_400, agentIds: ["c1"],
                rules: [CapabilityPermissionInput(capability: .mailSearch, mode: .auto, scope: PermissionScopeSelection(kind: .allInAccount, ids: []))],
                used: 0, usageVerified: true, unresolved: 0, maxUses: 20, scopeSummary: "Search mail in this account, for Claude",
                effectiveReason: "Explicit temporary permission", restrictions: []
            )]
        }
        // `-demoMode bypass|overnight|lockdown` (screenshots): start with that mode running, as if started 12 minutes ago.
        let args = ProcessInfo.processInfo.arguments
        // `-demoOnly computers|ais|none` (screenshots): only the computers, only the AI apps, or a new account with
        // nothing connected, waiting or done.
        if let i = args.firstIndex(of: "-demoOnly"), i + 1 < args.count {
            switch args[i + 1] {
            case "computers": s.connections = s.connections.filter { $0.keyFingerprint != nil }
            case "ais": s.connections = s.connections.filter { $0.keyFingerprint == nil }
            default:
                s.connections = []
                s.activity = []
                s.pending = []
            }
        }
        if let i = args.firstIndex(of: "-demoMode"), i + 1 < args.count {
            switch args[i + 1] {
            case "bypass": s.apGlobal = ApRow(mode: .manual, bypassUntil: now + 47 * 60 + 12, bypassSince: now - 12 * 60)
            case "overnight": s.apGlobal = ApRow(mode: .overnight, overnightUntil: now + 6 * 3_600, overnightSince: now - 12 * 60, overnightReturn: .manual)
            case "lockdown": s.apGlobal = ApRow(mode: .lockdown)
            case "auto": s.apGlobal = ApRow(mode: .auto)
            default: break
            }
            // `-demoRunningEntries N`: N requests that mode decided, one every few seconds of its 12 minutes.
            if let j = args.firstIndex(of: "-demoRunningEntries"), j + 1 < args.count, let n = Int(args[j + 1]) {
                let decidedBy = args[i + 1]
                let firstId = (s.activity.map(\.id).max() ?? 0) + 1
                let decided = (0..<n).map { k in
                    DemoData.entry(
                        firstId + Int64(k), at: now - 700 + Int64(k * 600 / max(n, 1)), action: "read",
                        outcome: decidedBy == "lockdown" ? "denied" : "released", detail: "Request \(k + 1)", decidedBy: decidedBy
                    )
                }
                s.activity = decided.reversed() + s.activity
            }
        }
        if args.contains("-demoSyncedPush") || args.contains("-demoSyncedPushFails"), let push = s.views["req20"], let git = push.git,
           let i = s.activity.firstIndex(where: { $0.id == 14 }) {
            var kept = git
            kept.refs = git.refs.map { ref in
                var r = ref
                r.files = r.files.map { f in var f = f; f.diff = []; return f }
                r.commits = r.commits.map { c in var c = c; c.files = c.files.map { f in var f = f; f.diff = []; return f }; return c }
                return r
            }
            var shown = push
            shown.git = kept
            s.recorded[14] = shown
            s.activity[i].request?.git = kept
            s.syncedPush = git
            s.syncedFails = args.contains("-demoSyncedPushFails")
        }
        if joinWaiting {
            let join = DemoData.join(now)
            s.joins[join.id] = join
            s.pending.insert(DemoData.joinItem(join), at: 0)
        }
        state = s
        self.syncCap = syncCap
    }

    static func now() -> Int64 { Int64(Date().timeIntervalSince1970) }

    private func locked<T>(_ body: (inout State) throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body(&state)
    }

    /// A short pause where the real core talks to a server, so spinners show.
    private func latency(_ seconds: Double = 0.35) async throws {
        try await Task.sleep(for: .milliseconds(Int(seconds * 1000)))
    }

    // ---- session ------------------------------------------------------------------------------------------------

    func session() async -> SessionInfo? { locked { $0.session } }

    /// The demo has one phone: nothing to sync with, so never a notice, except with `-demoSyncNotice` (tests,
    /// screenshots), until it is dismissed.
    func accountSyncNotice() async -> String? {
        locked { $0.syncNoticeDismissed } || !ProcessInfo.processInfo.arguments.contains("-demoSyncNotice")
            ? nil : "Synced with your other phone. A backup of this phone's changes is kept."
    }

    func dismissAccountSyncNotice() async throws { locked { $0.syncNoticeDismissed = true } }

    func restoreAccountSyncBackup() async throws -> Bool { false }

    func login(serverUrl: String, email: String, password: String, totp: String?) async throws -> SessionInfo {
        try await latency(0.6)
        guard serverUrl.hasPrefix("http") else { throw CoreError.Network(reason: "could not reach \(serverUrl)") }
        if password.isEmpty || password == "wrong" { throw CoreError.InvalidCredentials }
        if email.lowercased().contains("2fa") {
            guard let code = totp, !code.isEmpty else { throw CoreError.TwoFactorRequired }
            if code == "000000" { throw CoreError.InvalidCredentials }
        }
        let info = SessionInfo(serverUrl: serverUrl, email: email)
        locked { $0.session = info; $0.hasAccountSecret = false }
        return info
    }

    /// Like `login`, with the server's checks: an email containing "taken" is already registered, a master password
    /// needs 12 characters.
    func createAccount(serverUrl: String, email: String, password: String) async throws -> SessionInfo {
        try await latency(0.6)
        guard serverUrl.hasPrefix("http") else { throw CoreError.Network(reason: "could not reach \(serverUrl)") }
        if email.lowercased().contains("taken") { throw CoreError.Invalid(reason: "An account with this email already exists.") }
        if password.count < 12 { throw CoreError.Invalid(reason: "The master password must be at least 12 characters long.") }
        let info = SessionInfo(serverUrl: serverUrl, email: email)
        locked { $0.session = info; $0.hasAccountSecret = false }
        return info
    }

    // ---- passwordless sign-in, the account's keys, another phone ----------------------------------------------------

    /// The demo says nothing about its server: the sign-in shows every way in.
    func serverInfo(serverUrl: String) async throws -> ServerInfo {
        ServerInfo(browserSignIn: nil, pushAndroid: nil, pushIos: nil)
    }

    /// "Continue": the browser part is skipped (`ssoBegin`'s URL is a data page the app never opens in the demo); the
    /// account is `Created`, or `Locked` with `-demoLocked` (another phone has its keys).
    func ssoBegin(serverUrl: String) async throws -> SsoStart {
        guard serverUrl.hasPrefix("http") else { throw CoreError.Network(reason: "could not reach \(serverUrl)") }
        return SsoStart(url: "\(serverUrl)/identity/connect/authorize?demo=1", callbackScheme: "com.reins2fa.app", state: "demo-state", verifier: "demo-verifier")
    }

    func ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome {
        try await latency(0.6)
        // Like the core: the callback must answer this sign-in (its `state`).
        let answered = URLComponents(string: callbackUrl)?.queryItems?.first { $0.name == "state" }?.value
        guard state == "demo-state", answered == state, callbackUrl.hasPrefix("com.reins2fa.app://sso-callback") else {
            throw CoreError.Invalid(reason: "The sign-in did not come back as expected. Try again.")
        }
        let info = SessionInfo(serverUrl: serverUrl, email: DemoData.email)
        let keys = locked { s -> AccountKeys in
            s.session = info
            s.hasAccountSecret = true
            return s.keys
        }
        return SsoOutcome(session: info, keys: keys)
    }

    /// "Reset the vault": like the core, the confirming sign-in must answer this sign-in and be the locked account's
    /// (the demo's sign-in is always `DemoData.email`). The vault empties (requests, integrations, grants, activity,
    /// Autopilot's settings), the computers and AIs must be connected again, and the account gets new keys, as a new
    /// one does, with a new recovery code (`resetRecoveryCode`). This phone keeps what opens them, and nobody else
    /// approves.
    func resetAccount(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome {
        try await latency(0.6)
        let answered = URLComponents(string: callbackUrl)?.queryItems?.first { $0.name == "state" }?.value
        guard state == "demo-state", answered == state, callbackUrl.hasPrefix("com.reins2fa.app://sso-callback") else {
            throw CoreError.Invalid(reason: "The sign-in did not come back as expected. Try again.")
        }
        return try locked { s -> SsoOutcome in
            guard let current = s.session else { throw CoreError.NotLoggedIn }
            guard current.email.lowercased() == DemoData.email.lowercased() else {
                throw CoreError.Invalid(reason: "You signed in as \(DemoData.email). Sign in as \(current.email) to reset its vault.")
            }
            s.pending = []
            s.pendingVersion += 1
            s.views = [:]
            s.accounts = []
            s.accountStatuses = [:]
            s.emails = [:]
            s.mcp = []
            s.grants = []
            s.activity = []
            s.suggestions = [:]
            s.apGlobal = ApRow()
            s.apConnections = [:]
            s.joins = [:]
            s.joinPolls = nil
            s.connections = []
            s.keys = .unlocked
            s.hasAccountSecret = true
            s.approvalElsewhere = false
            s.recoveryCode = DemoReinsCore.resetRecoveryCode
            return SsoOutcome(session: current, keys: .created)
        }
    }

    /// Like the real core: a phone that keeps the keys is `unlocked` (`created` only ever answers the sign-in).
    func accountKeys() async throws -> AccountKeys {
        try locked { s in
            guard s.session != nil else { throw CoreError.NotLoggedIn }
            return s.keys == .locked ? .locked : .unlocked
        }
    }

    /// The demo's recovery code is `DemoData.recoveryCode` (`resetRecoveryCode` after a reset); "correct horse battery
    /// staple" stands for a master password.
    func unlockAccount(codeOrPassword: String) async throws {
        try await latency(0.5)
        let clean = codeOrPassword.uppercased().filter { $0.isLetter || $0.isNumber }
        let code = locked { $0.recoveryCode }
        guard clean == code.filter({ $0 != "-" }) || codeOrPassword == "correct horse battery staple" else {
            throw CoreError.Invalid(reason: "That recovery code does not open this account.")
        }
        locked { s in
            s.keys = .unlocked
            s.approvalElsewhere = false
        }
    }

    func accountRecoveryCode() async throws -> String {
        try await latency(0.2)
        return try locked { s in
            guard s.session != nil else { throw CoreError.NotLoggedIn }
            guard s.hasAccountSecret else { throw CoreError.Invalid(reason: "This account has no recovery code: it was made with a master password.") }
            guard s.keys != .locked else { throw CoreError.Invalid(reason: "This phone cannot open the account yet.") }
            return s.recoveryCode
        }
    }

    // ---- vault passkeys --------------------------------------------------------------------------------------------

    func vaultPasskeyOptions() async throws -> VaultPasskeyOptions {
        try await latency(0.2)
        return try locked { s in
            guard let session = s.session else { throw CoreError.NotLoggedIn }
            return VaultPasskeyOptions(
                rpId: "app.reins2fa.com", userHandle: Data("demo-user".utf8), userName: session.email,
                challenge: Data((0..<32).map { _ in UInt8.random(in: 0...255) }), prfSalt: Data(repeating: 1, count: 32),
                credentialIds: s.passkeys.map(\.credentialId)
            )
        }
    }

    func vaultPasskeys() async throws -> [VaultPasskeyView] {
        try await latency(0.2)
        return try locked { s in
            guard s.session != nil else { throw CoreError.NotLoggedIn }
            if s.passkeysOffline { throw CoreError.Network(reason: "offline") }
            return s.passkeys
        }
    }

    /// Like the core: only where the vault is open, and only with a 32-byte PRF output.
    func addVaultPasskey(credentialId: Data, prfOutput: Data, name: String) async throws -> [VaultPasskeyView] {
        try await latency(0.4)
        return try locked { s in
            guard s.session != nil else { throw CoreError.NotLoggedIn }
            guard s.keys != .locked else { throw CoreError.Invalid(reason: "This phone cannot open the account yet.") }
            guard prfOutput.count == 32 else { throw CoreError.Invalid(reason: "The passkey did not answer as expected.") }
            s.passkeys.removeAll { $0.credentialId == credentialId }
            s.passkeys.append(VaultPasskeyView(credentialId: credentialId, name: name, createdAt: Self.now()))
            return s.passkeys
        }
    }

    func removeVaultPasskey(credentialId: Data) async throws -> [VaultPasskeyView] {
        try await latency(0.3)
        return try locked { s in
            guard s.passkeys.contains(where: { $0.credentialId == credentialId }) else { throw CoreError.NotFound }
            s.passkeys.removeAll { $0.credentialId == credentialId }
            return s.passkeys
        }
    }

    /// Opens a locked account like `unlockAccount`: any of the account's passkeys with the demo's PRF output.
    func unlockWithVaultPasskey(credentialId: Data, prfOutput: Data) async throws {
        try await latency(0.5)
        try locked { s in
            guard s.passkeys.contains(where: { $0.credentialId == credentialId }), prfOutput == Self.demoPrfOutput else {
                throw CoreError.Invalid(reason: "That passkey does not open this account.")
            }
            s.keys = .unlocked
            s.approvalElsewhere = false
        }
    }

    /// The other phone answers after two polls (`-demoJoinDenied`: it refuses).
    func joinBegin(deviceName: String) async throws -> JoinStart {
        try await latency()
        return locked { s in
            s.joinPolls = 0
            return JoinStart(id: "join-1", code: "482 193", expiresAt: Self.now() + 600)
        }
    }

    func joinPoll() async throws -> JoinProgress {
        try await latency(0.2)
        let denied = ProcessInfo.processInfo.arguments.contains("-demoJoinDenied")
        return try locked { s in
            guard let polls = s.joinPolls else { throw CoreError.NotFound }
            if polls < 2 {
                s.joinPolls = polls + 1
                return .waiting
            }
            s.joinPolls = nil
            if denied { return .denied }
            s.keys = .unlocked
            s.approvalElsewhere = false
            return .joined
        }
    }

    func joinCancel() async throws { locked { $0.joinPolls = nil } }

    // Synthetic phone-role setup for native previews and tests; no real authority or proof is persisted.
    func phoneRole() async throws -> PhoneRole { locked { $0.setupRole } }
    func rosterSummary(refresh: Bool) async throws -> RosterSummary? { locked { $0.setupRoster } }
    func configurePrimarySetup(role: PhoneRole, requiresProof: Bool = false) {
        locked { state in
            state.setupRole = role; state.setupRequiresProof = requiresProof
            state.setupRoster = role == .legacy ? nil : RosterSummary(epoch: 1, role: role, members: [RosterMemberView(deviceId: "primary", name: "Primary phone", primary: true, thisPhone: role == .primary)], ownerFingerprint: "fixture-owner", warnings: [])
        }
    }
    var primarySetupCreated: Int { locked { $0.setupCreated } }
    func liveSync(waitSecs: UInt32) async throws -> LiveSyncState { Self.legacyLive }
    func liveSyncState() async throws -> LiveSyncState { Self.legacyLive }
    func proposalCreate(kind: String, title: String, rowsJson: String) async throws -> String {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }
    func proposalWithdraw(id: String) async throws { throw CoreError.NotFound }
    func proposals() async throws -> [ProposalView] { [] }
    func proposalPreview(id: String) async throws -> ProposalPreview { throw CoreError.NotFound }
    func proposalKeep(id: String, digest: String) async throws { throw CoreError.NotFound }
    func proposalDiscard(id: String) async throws { throw CoreError.NotFound }
    private static let legacyLive = LiveSyncState(
        role: .legacy, epoch: 0, appliedSeq: 0, headSeq: 0, primaryName: "", primarySeenAt: nil, primaryHeadSeq: nil,
        live: false, forked: nil, disagrees: false, waitingForKey: false, queued: 0, counterGaps: 0, notice: nil
    )
    func rosterCreate(deviceName: String, codeOrPassword: String?) async throws -> RosterSummary {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }
    func rosterCreateAuthorized(deviceName: String, codeOrPassword: String?, owner: OwnerCheck) async throws -> RosterSummary {
        try await Self.confirmOwner(owner, "Make this the approval phone for your whole Reins account")
        return try locked { state in
            guard state.setupRole == .legacy, state.setupRoster == nil else { throw CoreError.Invalid(reason: "This account has phone roles already.") }
            if let proof = codeOrPassword, proof.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { throw CoreError.Invalid(reason: "Enter the recovery code or master password.") }
            if state.setupRequiresProof, codeOrPassword == nil { throw CoreError.Invalid(reason: "Enter the recovery code or master password.") }
            let result = RosterSummary(epoch: 1, role: .primary, members: [RosterMemberView(deviceId: "this-phone", name: deviceName, primary: true, thisPhone: true)], ownerFingerprint: "fixture-owner", warnings: [])
            state.setupRole = .primary; state.setupRoster = result; state.setupCreated += 1
            for (key, value) in state.permissionViews where value.service == "gmail" && !value.conflicted && value.usageVerified {
                var view = value; view.availablePresets = [.reviewEveryRequest, .readAssist, .custom]
                view.capabilities = view.capabilities.map { cap in var next = cap; next.supportedModes = [.ask, .auto, .block]; return next }
                state.permissionViews[key] = view
            }
            return result
        }
    }
    /// `-demoPhoneRoles` (screenshots): the account has phone roles. This phone can ask to join as a second phone
    /// (the approving phone's code comes back at once), and a waiting `-demoJoin` request is an enrollment.
    private var phoneRoles: Bool { ProcessInfo.processInfo.arguments.contains("-demoPhoneRoles") }

    func enrollBegin(deviceName: String) async throws -> EnrollStart {
        guard phoneRoles else { throw CoreError.Invalid(reason: "Phone roles are not in the demo.") }
        return EnrollStart(id: "enroll-demo", expiresAt: Int64(Date().timeIntervalSince1970) + 600)
    }
    func enrollPoll() async throws -> EnrollProgress {
        guard phoneRoles else { throw CoreError.NotFound }
        try await latency(0.4)
        return .compare(code: "482 193")
    }
    func enrollCancel() async throws {}
    func enrollView(id: String) async throws -> EnrollView {
        guard phoneRoles else { throw CoreError.NotFound }
        return EnrollView(id: id, deviceName: "Pixel 9", code: "482 193", proven: true, createdAt: Int64(Date().timeIntervalSince1970) - 30)
    }
    func enrollConfirm(id: String, approve: Bool) async throws {
        guard phoneRoles else { throw CoreError.NotFound }
    }
    func handoverRequest() async throws -> String {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }
    func handoverPoll() async throws -> HandoverProgress { throw CoreError.NotFound }
    func handoverRequests() async throws -> [HandoverView] { [] }
    func handoverApprove(id: String, deviceId: String) async throws { throw CoreError.NotFound }
    func handoverDeny(id: String) async throws { throw CoreError.NotFound }
    func rosterRecoverPreview(codeOrPassword: String) async throws -> RecoveryPreview {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }
    func rosterRecover(codeOrPassword: String, deviceName: String, tip: String, confirmation: String?) async throws -> RosterSummary {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }
    func rosterRemoveMember(deviceId: String) async throws { throw CoreError.NotFound }
    func rosterRewrap(newPassword: String) async throws {
        throw CoreError.Invalid(reason: "Phone roles are not in the demo.")
    }

    /// The demo has no phone roles: every waiting phone asks for the account's keys.
    func joinIsEnrollment(id: String) async throws -> Bool { phoneRoles }

    func joinView(id: String) async throws -> JoinView {
        try locked { s in
            guard let v = s.joins[id] else { throw CoreError.NotFound }
            return v
        }
    }

    func answerJoin(id: String, approve: Bool) async throws {
        try await latency()
        try locked { s in
            guard s.joins[id] != nil else { throw CoreError.NotFound }
            s.joins[id] = nil
            Self.removePending(&s, id)
        }
    }

    func logout() async throws { locked { $0.session = nil } }
    func logoutWithBrowser() async throws -> String? { try await logout(); return nil }

    /// Like the server: the typed email must be the account's, and with `-demoOtherPhone` this phone may not until
    /// it has the role. Nothing is kept anyway, so deleting is signing out.
    func deleteAccount(confirmEmail: String) async throws {
        try await latency()
        try locked { s in
            guard let email = s.session?.email else { throw CoreError.NotLoggedIn }
            let typed = confirmEmail.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !typed.isEmpty, typed.lowercased() == email.lowercased() else {
                throw CoreError.Invalid(reason: "The email you typed is not this account's email.")
            }
            if s.approvalElsewhere {
                throw CoreError.Invalid(reason: "Another phone approves requests for this account. Delete the account from that phone, or make this phone the approval device first.")
            }
            s.session = nil
        }
    }

    /// Refused with `-demoOtherPhone` until the recovery code or the other phone's approval, like the server, which
    /// wants the proof the core attaches.
    func registerDevice(fcmToken: String?) async throws {
        if locked({ $0.approvalElsewhere }) { throw CoreError.OtherApprovalDevice }
    }

    func handlePush(kind: String, id: String) async throws {}

    func handlePushDeferringAutopilot(kind: String, id: String) async throws {}

    // ---- pending and the long poll --------------------------------------------------------------------------------

    /// `-demoArrive`: puts the new request in once its time has come.
    private static func deliverArrival(_ s: inout State) {
        guard let at = s.arriveAt, now() >= at else { return }
        s.arriveAt = nil
        let (item, view) = DemoData.arrival(now())
        s.views[view.requestId] = view
        s.pending.insert(item, at: 0)
        s.pendingVersion += 1
    }

    private static func visiblePending(_ s: State) -> [PendingItem] {
        let installed = s.model.state == .installed
        return s.pending.map { item in
            var item = item
            if !installed { item.suggestion = nil }
            return item
        }
    }

    func pending() async throws -> [PendingItem] {
        locked { s in
            Self.deliverArrival(&s)
            return Self.visiblePending(s)
        }
    }

    /// The demo has no modes deciding in the background: nothing is ever left for one.
    func deciding() async throws -> [PendingItem] { [] }

    /// Waits like the server's long poll (at most a few seconds), returning early when something changes.
    func sync(waitSecs: UInt32) async throws -> [PendingItem] {
        let start = try locked { s -> Int in
            guard s.session != nil else { throw CoreError.NotLoggedIn }
            return s.pendingVersion
        }
        let deadline = Date().addingTimeInterval(min(Double(waitSecs), syncCap))
        while Date() < deadline {
            try await Task.sleep(for: .milliseconds(250))
            let changed = locked { s -> Bool in
                Self.deliverArrival(&s)
                return s.pendingVersion != start
            }
            if changed { break }
        }
        return locked { Self.visiblePending($0) }
    }

    func approvalView(requestId: String) async throws -> ApprovalView {
        try locked { s in
            guard let v = s.views[requestId], s.pending.contains(where: { $0.id == requestId }) else { throw CoreError.NotFound }
            return v
        }
    }

    /// An AI app asks to connect, parked like a pushed pairing (a second account of one already connected, for one).
    /// Returns its id.
    func addDemoPairing(clientName: String, clientHost: String) async throws -> String {
        locked { s in
            s.nextConnection += 1
            let id = "pair-demo-\(s.nextConnection)"
            let p = PairingView(
                id: id, clientName: clientName, clientHost: clientHost, choices: Data([7, 42, 88]),
                createdAt: Self.now(), keyFingerprint: nil
            )
            s.pairings[id] = p
            s.pending.insert(DemoData.pairingItem(p), at: 0)
            s.pendingVersion += 1
            return id
        }
    }

    /// A desktop app on "studio" asks to pair, parked like a pushed pairing. Codes starting with "BBBB" have expired.
    func pairingByCode(userCode: String) async throws -> PairingView {
        try await latency()
        guard let code = PairingCode.normalize(userCode) else { throw CoreError.Invalid(reason: "That is not a pairing code.") }
        if code.hasPrefix("BBBB") { throw CoreError.NotFound }
        return locked { s in
            let id = "code-\(code)"
            if let parked = s.pairings[id] { return Self.named(parked, in: s).view }
            let p = PairingView(
                id: id, clientName: "studio", clientHost: "studio", choices: Data([47, 12, 83]),
                createdAt: Self.now(), keyFingerprint: "3170 6422"
            )
            s.pairings[id] = p
            s.pending.insert(DemoData.pairingItem(p), at: 0)
            s.pendingVersion += 1
            // Filled as pairingView fills it: the name field's start and whether the client is connected already.
            return Self.named(p, in: s).view
        }
    }

    /// As the core: the name field starts with the client's own name, or "Claude 2" when the same client is connected
    /// already (the same name from the same host).
    func pairingView(pairingId: String) async throws -> PairingView {
        try locked { s in
            guard let p = s.pairings[pairingId] else { throw CoreError.NotFound }
            return Self.named(p, in: s).view
        }
    }

    /// As the core's `named_pairing`: the pairing with its suggestion filled from the connections there are now, and
    /// the number a second of the same client is told apart by (nil for the first).
    private static func named(_ pairing: PairingView, in s: State) -> (view: PairingView, number: UInt32?) {
        var p = pairing
        let same = s.connections.filter {
            $0.clientHost.lowercased() == p.clientHost.lowercased()
                && ($0.clientName.isEmpty ? $0.label : $0.clientName).lowercased() == p.clientName.lowercased()
        }.count
        p.sameClient = UInt32(same)
        p.suggestedName = p.clientName
        guard same > 0 else { return (p, nil) }
        let taken = Set(s.connections.map { $0.label.lowercased() })
        var n = same + 1
        while taken.contains("\(p.clientName) \(n)".lowercased()) { n += 1 }
        p.suggestedName = "\(p.clientName) \(n)"
        return (p, UInt32(n))
    }

    /// As the core: one clean line of at most 64 characters; blank takes the name away.
    func renameConnection(connectionId: String, name: String?) async throws -> String? {
        try locked { s in
            guard let i = s.connections.firstIndex(where: { $0.id == connectionId }) else { throw CoreError.NotFound }
            let clean = name.map { untrustedName($0, limit: 64) }.flatMap { $0.isEmpty ? nil : $0 }
            var c = s.connections[i]
            if c.ownLabel.isEmpty { c.ownLabel = c.name == nil ? c.label : c.clientName }
            if c.clientName.isEmpty { c.clientName = c.ownLabel }
            let old = c.label
            c.name = clean
            c.label = clean ?? c.ownLabel
            s.connections[i] = c
            for j in s.pending.indices where s.pending[j].connectionId == connectionId && s.pending[j].connectionLabel == old {
                s.pending[j].connectionLabel = c.label
            }
            s.pendingVersion += 1
            return clean
        }
    }

    func blobView(id: String) async throws -> BlobView {
        try locked { s in
            guard let b = s.blobs[id] else { throw CoreError.NotFound }
            return b
        }
    }

    private static func removePending(_ s: inout State, _ id: String) {
        s.pending.removeAll { $0.id == id }
        s.pendingVersion += 1
        s.suggestions[id] = nil
    }

    private static func nextActivityId(_ s: State) -> Int64 { (s.activity.map(\.id).max() ?? 0) + 1 }

    /// The history line for a decided request, like the core's audit log writes it.
    private static func logDecision(
        _ s: inout State, _ v: ApprovalView, outcome: String, selected: [String], grantId: String?, decidedBy: String = ""
    ) {
        let messages = v.messages.filter { selected.isEmpty || selected.contains($0.id) }
        let accounts = v.accounts.filter { selected.isEmpty || selected.contains($0) }
        let count: UInt32 = switch v.kind {
        case .search, .read, .fetch: UInt32(messages.count)
        case .accounts: UInt32(accounts.count)
        default: v.count
        }
        let detail: String = if let email = v.email {
            "To \(email.to.first ?? "")"
        } else if let q = v.query {
            q
        } else if let g = v.grant {
            g.summary
        } else if let first = v.preview.first {
            first
        } else {
            v.resources.first?.label ?? ""
        }
        let info = ActivityInfo(
            query: v.query,
            messages: messages.map { ActivityMessage(id: $0.id, text: v.service == "gmail" ? "" : $0.snippet, from: $0.from, subject: $0.subject, date: $0.date) },
            email: v.email, note: v.grant?.reason, grantSummary: grantId == nil ? nil : s.grants.first { $0.id == grantId }?.summary,
            accounts: v.kind == .accounts ? accounts : []
        )
        let entry = ActivityEntry(
            id: nextActivityId(s), at: now(), connectionId: v.connectionId, connectionLabel: v.connectionLabel, action: v.action,
            outcome: outcome, detail: detail, grantId: grantId, service: v.service, account: v.account, count: count, info: info,
            op: v.op, opTitle: v.opTitle, decidedBy: decidedBy, autopilot: nil
        )
        s.activity.insert(entry, at: 0)
        s.recorded[entry.id] = v
    }

    func approve(requestId: String, choice: ApprovalChoice) async throws {
        try locked { s in
            guard let v = s.views[requestId], s.pending.contains(where: { $0.id == requestId }) else { throw CoreError.NotFound }
            var grantId: String?
            if v.kind == .grant, let g = v.grant {
                let standing = choice.standing ?? StandingGrant(durationSecs: g.durationSecs, maxUses: g.maxUses, scope: DemoData.scope())
                let made = Self.makeGrant(&s, connectionId: v.connectionId, label: v.connectionLabel, action: g.action, service: v.service,
                                          account: v.account, standing: standing, origin: "request", summary: g.summary.prefix(1).uppercased() + g.summary.dropFirst(),
                                          lines: g.lines)
                grantId = made.id
            } else if let standing = choice.standing, let r = v.envRelease {
                // As the core: the command picked among those offered, and the variables asked for or every one.
                let command = r.commands.first { $0.id == standing.scope.secrets?.command }?.label ?? r.command
                let every = r.allVariables || standing.scope.secrets?.allVariables == true
                let made = Self.makeGrant(&s, connectionId: v.connectionId, label: v.connectionLabel, action: "secrets", service: v.service,
                                          account: v.account, standing: standing, origin: "approval",
                                          summary: "Use \(r.project) · \(r.env) for \(command)",
                                          lines: ["\(r.runnerLabel) runs \(command)", every ? "Every variable" : r.variables.joined(separator: ", "), "In \(r.root)"])
                if let i = s.grants.firstIndex(where: { $0.id == made.id }) {
                    s.grants[i].secretsProject = r.project
                    s.grants[i].secretsEnv = r.env
                }
                grantId = made.id
            } else if let standing = choice.standing {
                grantId = Self.makeGrant(&s, connectionId: v.connectionId, label: v.connectionLabel, action: Self.grantAction(v.kind, v.action),
                                         service: v.service, account: v.account, standing: standing, origin: "approval", view: v).id
            }
            let outcome = switch v.kind {
            case .grant: "granted"
            case .send, .write: "sent"
            default: "released"
            }
            Self.removePending(&s, requestId)
            Self.logDecision(&s, v, outcome: outcome, selected: choice.selectedMessageIds, grantId: grantId)
        }
    }

    private var starting: StartingPolicy?

    func startingPolicy() async throws -> StartingPolicy? { locked { _ in starting } }

    func setStartingPolicy(policy: StartingPolicy) async throws { locked { _ in starting = policy } }

    /// As the core: what the sheet approves untouched, refused for what is asked every time.
    func approveQuick(requestId: String) async throws {
        let view: ApprovalView? = locked { s in s.views[requestId] }
        guard let v = view else { throw CoreError.NotFound }
        guard v.quick != nil else { throw CoreError.Invalid(reason: "Open this request to decide.") }
        let selected = switch v.kind {
        case .search, .read, .fetch: v.messages.filter { !$0.sensitive }.map(\.id)
        default: [String]()
        }
        try await approve(requestId: requestId, choice: ApprovalChoice(selectedMessageIds: selected, standing: nil))
    }

    func deny(requestId: String) async throws {
        try locked { s in
            guard let v = s.views[requestId], s.pending.contains(where: { $0.id == requestId }) else { throw CoreError.NotFound }
            Self.removePending(&s, requestId)
            Self.logDecision(&s, v, outcome: "denied", selected: [], grantId: nil)
        }
    }

    func answerPairing(pairingId: String, approve: Bool, chosenCode: UInt8?, label: String?, nameEdited: Bool) async throws {
        try await latency()
        try locked { s in
            guard let parked = s.pairings[pairingId] else { throw CoreError.NotFound }
            Self.removePending(&s, pairingId)
            s.pairings[pairingId] = nil
            guard approve else { return }
            // As the core, the suggestion and number are worked out again now, from the connections there are.
            let (p, number) = Self.named(parked, in: s)
            s.nextConnection += 1
            let typed = label.map { untrustedName($0, limit: 64) }.flatMap { $0.isEmpty ? nil : $0 }
            // As the core: only a name the user edited is theirs, and never the client's own or the suggestion.
            let own = [p.clientName, p.suggestedName].map { untrustedName($0, limit: 64).lowercased() }
            let given: String? = nameEdited ? typed.flatMap { own.contains($0.lowercased()) ? nil : $0 } : nil
            // A second of the same client keeps the suggestion's number as part of its own name ("Claude 2").
            let ownLabel = number.map { "\(p.clientName) \($0)" } ?? p.clientName
            let name = given ?? ownLabel
            let id = "c\(s.nextConnection)"
            // The demo's computers are Macs (a real one says its platform in the name it registers with).
            let platform: String? = p.keyFingerprint != nil ? "macos" : nil
            s.connections.append(ConnectionView(
                id: id, label: name, clientHost: p.clientHost, createdAt: Self.now(), lastUsedAt: nil, icon: nil, keyFingerprint: p.keyFingerprint,
                autoIcon: Self.autoIcon(label: p.clientName, computer: p.keyFingerprint != nil, platform: platform), platform: platform,
                blobatar: Self.firstBlobatar(id, others: s.connections.compactMap(\.blobatar)),
                name: given, ownLabel: ownLabel, number: given == nil ? number : nil, clientName: p.clientName
            ))
        }
    }

    func answerBlob(id: String, approve: Bool) async throws {
        try locked { s in
            guard let b = s.blobs[id] else { throw CoreError.NotFound }
            let item = s.pending.first { $0.id == id }
            Self.removePending(&s, id)
            s.blobs[id] = nil
            let entry = ActivityEntry(
                id: Self.nextActivityId(s), at: Self.now(), connectionId: item?.connectionId ?? "", connectionLabel: b.connectionLabel,
                action: "upload", outcome: approve ? "released" : "denied",
                detail: "\(b.name) · \(ByteCountFormatter.string(fromByteCount: Int64(b.size), countStyle: .file))",
                grantId: nil, service: "files", account: nil, count: 1, info: DemoData.info(note: b.purpose), op: "", opTitle: "",
                decidedBy: "", autopilot: nil
            )
            s.activity.insert(entry, at: 0)
        }
    }

    // ---- activity -------------------------------------------------------------------------------------------------

    func activity(limit: UInt32) async throws -> [ActivityEntry] {
        locked { Array($0.activity.prefix(Int(limit))) }
    }

    func activityEntry(entryId: Int64) async throws -> ActivityEntry? {
        locked { $0.activity.first { $0.id == entryId } }
    }

    /// As the core: the approval a request decided here showed; nil for the seeded history.
    func activityApproval(entryId: Int64) async throws -> ApprovalView? {
        locked { $0.recorded[entryId] }
    }

    func activityPush(entryId: Int64) async throws -> ActivityPush? {
        locked { s in
            s.recorded[entryId]?.git.map { git in
                let diffs = git.refs.contains { r in
                    (r.files + r.commits.flatMap(\.files)).contains { !$0.diff.isEmpty }
                }
                let synced = entryId == 14 && s.syncedPush != nil
                return ActivityPush(git: git, diffs: diffs, diffsNotKept: synced, thisDevice: !synced)
            }
        }
    }

    func activityCommitChange(entryId: Int64, shortSha: String) async throws -> GitCommitView {
        try await latency(0.8)
        return try locked { s in
            guard !s.syncedFails else { throw CoreError.Service(reason: "GitHub did not answer. Check the connection and try again.") }
            guard entryId == 14, let commit = s.syncedPush?.refs.flatMap(\.commits).first(where: { $0.shortSha == shortSha }) else {
                throw CoreError.Service(reason: "the demo has no GitHub connection")
            }
            return commit
        }
    }

    /// The core's search, in memory: every word a case-insensitive substring of the entry's text, the filters as the
    /// core reads them, newest first, a page at a time.
    func searchActivity(query: ActivityQuery, after: ActivityCursor?, limit: UInt32) async throws -> ActivityPage {
        locked { s in
            let terms = DemoSearch.terms(query.text)
            let all = s.activity.filter { DemoSearch.keeps($0, query, terms) }.sorted { ($0.at, $0.id) > ($1.at, $1.id) }
            let rest = after.map { a in all.filter { ($0.at, $0.id) < (a.at, a.id) } } ?? all
            let page = Array(rest.prefix(Int(min(max(limit, 1), 200))))
            let next = rest.count > page.count ? page.last.map { ActivityCursor(at: $0.at, id: $0.id) } : nil
            return ActivityPage(
                hits: page.map { ActivityHit(entry: $0, snippet: DemoSearch.snippet($0, terms)) }, next: next, total: UInt32(all.count),
                terms: terms
            )
        }
    }

    func activityStamps(query: ActivityQuery) async throws -> [ActivityStamp] {
        locked { s in
            let terms = DemoSearch.terms(query.text)
            return s.activity.filter { DemoSearch.keeps($0, query, terms) }
                .sorted { ($0.at, $0.id) > ($1.at, $1.id) }
                .map(ActivityDays.stamp)
        }
    }

    func activityFacets() async throws -> ActivityFacets {
        locked { s in
            func mostUsed(_ values: [String]) -> [String] {
                let counts = Dictionary(values.map { ($0, 1) }, uniquingKeysWith: +)
                return counts.keys.sorted { (counts[$0]!, $1) > (counts[$1]!, $0) }
            }
            var seen: Set<String> = []
            let connections = s.activity.sorted { $0.at > $1.at }.compactMap { e -> ActivityConnection? in
                guard !e.connectionId.isEmpty, seen.insert(e.connectionId).inserted else { return nil }
                return ActivityConnection(id: e.connectionId, label: untrustedName(e.connectionLabel))
            }
            return ActivityFacets(
                services: mostUsed(s.activity.map(\.service).filter { !$0.isEmpty }), actions: mostUsed(s.activity.map(\.action)),
                connections: connections
            )
        }
    }

    func fetchEmail(account: String?, messageId: String) async throws -> EmailContent {
        try await latency(0.25)
        return try locked { s in
            guard let e = s.emails[messageId] else { throw CoreError.Gmail(reason: "that email is no longer in Gmail") }
            return e
        }
    }

    // ---- grants -----------------------------------------------------------------------------------------------------

    private static func grantAction(_ kind: ApprovalKind, _ action: String) -> String {
        switch kind {
        case .search, .read, .fetch: "read"
        case .send: "send"
        case .write: "write"
        default: action == "search" ? "read" : action
        }
    }

    /// The lines a grant's scope reads as ("From @bank.com", "All mail", resource labels).
    private static func lines(_ scope: GrantScopeChoice, resources: [ResourceView] = []) -> [String] {
        var out: [String] = []
        if scope.allMail { out.append("All mail") }
        out += scope.senderDomains.map { "From @\($0)" } + scope.senderAddresses.map { "From \($0)" }
        out += scope.recipientDomains.map { "To @\($0)" } + scope.recipientAddresses.map { "To \($0)" }
        if let p = scope.subjectPattern { out.append("Subject: \(p)") }
        out += scope.resources.map { id in resources.first { $0.id == id }?.label ?? id }
        if !scope.classes.isEmpty { out.append(scope.classes.map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined(separator: ", ")) }
        if scope.selectedMessagesOnly { out.append("Only the emails you picked") }
        return out
    }

    private static func summary(action: String, service: String, scope: GrantScopeChoice, opTitle: String) -> String {
        if service == "gmail" {
            switch action {
            case "send":
                if let d = scope.recipientDomains.first { return "Send emails to @\(d)" }
                if let a = scope.recipientAddresses.first { return "Send emails to \(a)" }
                return "Send emails"
            default:
                if scope.allMail { return "Read all emails" }
                if let d = scope.senderDomains.first { return "Read emails from @\(d)" }
                if let a = scope.senderAddresses.first { return "Read emails from \(a)" }
                return "Read these emails"
            }
        }
        return opTitle.isEmpty ? "\(action.prefix(1).uppercased() + action.dropFirst()) on \(serviceName(service))" : opTitle
    }

    @discardableResult
    private static func makeGrant(
        _ s: inout State, connectionId: String, label: String, action: String, service: String, account: String?,
        standing: StandingGrant, origin: String, view: ApprovalView? = nil, summary: String? = nil, lines: [String]? = nil
    ) -> GrantView {
        s.nextGrant += 1
        let now = now()
        let scope = standing.scope
        let grant = GrantView(
            id: "g\(s.nextGrant)", connectionId: connectionId, connectionLabel: label, action: action,
            summary: summary ?? Self.summary(action: action, service: service, scope: scope, opTitle: view?.opTitle ?? ""),
            expiresAt: standing.durationSecs.map { now + Int64($0) }, maxUses: standing.maxUses, uses: 0, createdAt: now, lastUsedAt: nil,
            origin: origin, service: service, account: account,
            lines: lines ?? Self.lines(scope, resources: view?.resources ?? []), active: true, state: "active",
            allMail: scope.allMail, editableScope: scope
        )
        s.grants.insert(grant, at: 0)
        return grant
    }

    func grants() async throws -> [GrantView] { locked { $0.grants } }

    func createGrant(connectionId: String, account: String, kind: ApprovalKind, standing: StandingGrant) async throws {
        try await latency()
        try locked { s in
            guard let c = s.connections.first(where: { $0.id == connectionId }) else { throw CoreError.NotFound }
            let service = s.accounts.first { $0.account == account }?.service ?? "gmail"
            Self.makeGrant(&s, connectionId: connectionId, label: c.label, action: Self.grantAction(kind, ""), service: service,
                           account: account, standing: standing, origin: "manual")
        }
    }

    func revokeGrant(grantId: String) async throws {
        try locked { s in
            guard let i = s.grants.firstIndex(where: { $0.id == grantId }) else { throw CoreError.NotFound }
            s.grants[i].active = false
            s.grants[i].state = "revoked"
            s.grants[i].expiresAt = Self.now()
        }
    }

    /// As the core: the permissions for secrets of `project` (and `env`) end here. The demo's second computer, "studio",
    /// is offline, so the note to it is not delivered yet.
    func revokeSecretsEverywhere(project: String?, env: String?) async throws -> SecretsRevoked {
        try await latency()
        return locked { s -> SecretsRevoked in
            var ended: UInt32 = 0
            for i in s.grants.indices where s.grants[i].action == "secrets" && s.grants[i].active
                && (project == nil || s.grants[i].secretsProject == project) && (env == nil || s.grants[i].secretsEnv == env) {
                s.grants[i].active = false
                s.grants[i].state = "revoked"
                s.grants[i].expiresAt = Self.now()
                ended += 1
            }
            return SecretsRevoked(ended: ended, told: 1, notDelivered: ["studio"])
        }
    }

    func setSecretsAlwaysAsk(project: String, env: String, always: Bool?) async throws {}

    func deleteGrant(grantId: String) async throws {
        locked { $0.grants.removeAll { $0.id == grantId } }
    }

    func resumeGrant(grantId: String, durationSecs: UInt64) async throws {
        try locked { s in
            guard let i = s.grants.firstIndex(where: { $0.id == grantId }) else { throw CoreError.NotFound }
            s.grants[i].active = true
            s.grants[i].state = "active"
            s.grants[i].uses = 0
            s.grants[i].createdAt = Self.now()
            s.grants[i].expiresAt = Self.now() + Int64(durationSecs)
        }
    }

    /// An active grant runs for `durationSecs` from now; its uses stay.
    func extendGrant(grantId: String, durationSecs: UInt64) async throws {
        try locked { s in
            guard let i = s.grants.firstIndex(where: { $0.id == grantId }), s.grants[i].active else { throw CoreError.NotFound }
            s.grants[i].createdAt = Self.now()
            s.grants[i].expiresAt = Self.now() + Int64(durationSecs)
        }
    }

    func resumeGrantEdited(grantId: String, standing: StandingGrant) async throws {
        try locked { s in
            guard let i = s.grants.firstIndex(where: { $0.id == grantId }) else { throw CoreError.NotFound }
            var g = s.grants[i]
            g.active = true
            g.state = "active"
            g.uses = 0
            g.createdAt = Self.now()
            g.expiresAt = standing.durationSecs.map { Self.now() + Int64($0) }
            g.maxUses = standing.maxUses
            g.editableScope = standing.scope
            g.allMail = standing.scope.allMail
            g.lines = Self.lines(standing.scope)
            g.summary = Self.summary(action: g.action, service: g.service, scope: standing.scope, opTitle: g.service == "gmail" ? "" : g.summary)
            s.grants[i] = g
        }
    }

    // ---- connections ------------------------------------------------------------------------------------------------

    func connections() async throws -> [ConnectionView] { locked { $0.connections } }

    // MARK: Devices and the vault on the phone

    static let demoDevices = [
        DeviceView(id: "d-this", name: "iPhone", kind: .phone, platform: "iOS", createdAt: 1_699_000_000, lastSeenAt: 1_700_000_090, approval: true, thisDevice: true),
        DeviceView(id: "d-old", name: "Old iPhone", kind: .phone, platform: "iOS", createdAt: 1_690_000_000, lastSeenAt: 1_699_900_000, approval: false, thisDevice: false),
    ]

    static let demoVault = [
        VaultItemDetail(
            id: "openai", name: "OpenAI", kind: .login,
            fields: [
                VaultField(key: "password", label: "Password", value: nil, secret: true, multiline: false),
                VaultField(key: "uris", label: "Website", value: "https://platform.openai.com", secret: false, multiline: false),
            ],
            uses: [VaultUse(reference: "vault:OpenAI/password", hint: "reins run --env, and secret = in an [[api]] block")],
            warning: nil, favorite: true
        ),
        VaultItemDetail(
            id: "github", name: "GitHub", kind: .login,
            fields: [
                VaultField(key: "username", label: "Username", value: "octo", secret: false, multiline: false),
                VaultField(key: "password", label: "Password", value: nil, secret: true, multiline: false),
            ],
            uses: [
                VaultUse(reference: "vault:GitHub/password", hint: "reins run --env, and secret = in an [[api]] block"),
                VaultUse(reference: "vault:GitHub/username", hint: "reins run --env, and secret = in an [[api]] block"),
            ],
            warning: nil, favorite: false
        ),
        VaultItemDetail(
            id: "deploy", name: "Deploy key", kind: .sshKey,
            fields: [
                VaultField(key: "private_key", label: "Private key", value: nil, secret: true, multiline: true),
                VaultField(key: "public_key", label: "Public key", value: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoDeployKey00000000000000000000000000 Deploy key", secret: false, multiline: true),
                VaultField(key: "fingerprint", label: "Fingerprint", value: "SHA256:x1PqDemo0000000000000000000000000009fZk", secret: false, multiline: false),
            ],
            uses: [VaultUse(reference: "vault:Deploy key/public", hint: "Offered by the SSH agent of the desktop app (reins ssh setup); the private key stays on the phone")],
            warning: nil, favorite: false
        ),
    ]

    func devices() async throws -> [DeviceView] { locked { $0.devices } }

    func signOutDevice(deviceId: String, codeOrPassword: String) async throws {
        try locked { s in
            guard codeOrPassword == s.recoveryCode else { throw CoreError.Invalid(reason: "That is neither the recovery code nor the master password.") }
            s.devices.removeAll { $0.id == deviceId }
        }
    }

    func phoneKeyFingerprint() async throws -> String { "4821-9930-1274" }

    func vaultItems(query: String) async throws -> [VaultItemSummary] {
        locked { s in
            s.vault
                .filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
                .sorted { $0.name.lowercased() < $1.name.lowercased() }
                .map { d in
                    VaultItemSummary(
                        id: d.id, name: d.name, kind: d.kind,
                        subtitle: d.kind == .sshKey ? "SSH key · ed25519" : d.fields.first { !$0.secret }?.value ?? "", favorite: d.favorite
                    )
                }
        }
    }

    func vaultItem(id: String) async throws -> VaultItemDetail {
        try locked { s in
            guard let item = s.vault.first(where: { $0.id == id }) else { throw CoreError.Service(reason: "That item is no longer in the vault.") }
            return item
        }
    }

    func vaultReveal(id: String, key: String) async throws -> String {
        try locked { s in
            guard let value = s.vaultSecrets["\(id)/\(key)"] else { throw CoreError.Service(reason: "That item has no such field.") }
            return value
        }
    }

    func vaultCreate(input: VaultItemInput) async throws -> String {
        locked { s in
            let id = "new-\(s.vault.count)"
            let fields = input.fields.map { f in
                let secret = ["password", "notes", "totp", "number", "code", "private_key"].contains(f.key)
                if secret { s.vaultSecrets["\(id)/\(f.key)"] = f.value }
                return VaultField(key: f.key, label: f.key.capitalized, value: secret ? nil : f.value, secret: secret, multiline: false)
            }
            let uses = input.kind == .login ? [VaultUse(reference: "vault:\(input.name)/password", hint: "reins run --env, and secret = in an [[api]] block")] : []
            s.vault.append(VaultItemDetail(id: id, name: input.name, kind: input.kind, fields: fields, uses: uses, warning: nil, favorite: false))
            return id
        }
    }

    func vaultUpdate(id: String, input: VaultItemInput) async throws {
        locked { s in
            guard let i = s.vault.firstIndex(where: { $0.id == id }) else { return }
            s.vault[i].name = input.name
        }
    }

    func vaultDelete(id: String) async throws {
        locked { $0.vault.removeAll { $0.id == id } }
    }

    func vaultGenerateSshKey(name: String) async throws -> VaultSshKey {
        locked { s in
            let id = "ssh-\(s.vault.count)"
            let publicKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoKeyOnlyForTheDemoBuild000000000000000 \(name)"
            let fingerprint = "SHA256:demofingerprint000000000000000000000000000"
            s.vault.append(VaultItemDetail(
                id: id, name: name, kind: .sshKey,
                fields: [
                    VaultField(key: "private_key", label: "Private key", value: nil, secret: true, multiline: true),
                    VaultField(key: "public_key", label: "Public key", value: publicKey, secret: false, multiline: true),
                    VaultField(key: "fingerprint", label: "Fingerprint", value: fingerprint, secret: false, multiline: false),
                ],
                uses: [VaultUse(reference: fingerprint, hint: "Offered by the SSH agent of the desktop app (reins ssh setup); the private key stays on the phone")],
                warning: nil, favorite: false
            ))
            return VaultSshKey(id: id, publicKey: publicKey, fingerprint: fingerprint)
        }
    }

    func revokeConnection(connectionId: String) async throws {
        locked { s in
            s.connections.removeAll { $0.id == connectionId }
            s.grants.removeAll { $0.connectionId == connectionId }
            s.apConnections[connectionId] = nil
        }
    }

    func setConnectionIcon(connectionId: String, icon: String?) async throws {
        locked { s in
            if let i = s.connections.firstIndex(where: { $0.id == connectionId }) { s.connections[i].icon = icon }
        }
    }

    /// As the core: a new shape and colour, the seed kept; refused for a shape or colour that does not exist.
    func setConnectionBlobatar(connectionId: String, shape: UInt8, colour: UInt8) async throws -> Blobatar {
        guard Int(shape) < BlobatarPalette.shapes.count, Int(colour) < BlobatarPalette.colours.count else {
            throw CoreError.Invalid(reason: "There is no such shape or colour.")
        }
        return try locked { s in
            guard let i = s.connections.firstIndex(where: { $0.id == connectionId }) else { throw CoreError.NotFound }
            let b = Blobatar(seed: s.connections[i].blobatar?.seed ?? connectionId, shape: shape, colour: colour)
            s.connections[i].blobatar = b
            return b
        }
    }

    /// As the core: a fresh seed, another shape and another colour, one no other connection has while one is free.
    func shuffleConnectionBlobatar(connectionId: String) async throws -> Blobatar {
        try locked { s in
            guard let i = s.connections.firstIndex(where: { $0.id == connectionId }) else { throw CoreError.NotFound }
            let current = s.connections[i].blobatar ?? Self.firstBlobatar(connectionId, others: [])
            let taken = Set(s.connections.filter { $0.id != connectionId }.compactMap { $0.blobatar?.colour })
            let colours = (0..<UInt8(BlobatarPalette.colours.count)).filter { $0 != current.colour }
            let free = colours.filter { !taken.contains($0) }
            let shapes = (0..<UInt8(BlobatarPalette.shapes.count)).filter { $0 != current.shape }
            let b = Blobatar(
                seed: String(format: "%08x", UInt32.random(in: .min ... .max)),
                shape: shapes.randomElement() ?? current.shape,
                colour: (free.isEmpty ? colours : free).randomElement() ?? current.colour
            )
            s.connections[i].blobatar = b
            return b
        }
    }

    /// What Auto shows, as the core works it out: a computer its platform's logo, an AI its provider's when its name
    /// says which, else the blobatar.
    static func autoIcon(label: String, computer: Bool, platform: String?) -> String {
        if computer { return platform ?? Providers.blob }
        return Providers.infer(label)?.key ?? Providers.blob
    }

    /// A new connection's blobatar, standing apart from `others`: the least used colour and shape.
    static func firstBlobatar(_ id: String, others: [Blobatar]) -> Blobatar {
        func least(_ count: Int, _ used: [UInt8]) -> UInt8 {
            let uses = { (i: Int) in used.filter { Int($0) == i }.count }
            return UInt8((0..<count).min { uses($0) < uses($1) } ?? 0)
        }
        return Blobatar(
            seed: id,
            shape: least(BlobatarPalette.shapes.count, others.map(\.shape)),
            colour: least(BlobatarPalette.colours.count, others.map(\.colour))
        )
    }

    // ---- accounts and integrations --------------------------------------------------------------------------------

    func accounts() async throws -> [AccountView] { locked { $0.accounts.filter { $0.service == "gmail" } } }

    func services() async throws -> [ServiceView] {
        locked { s in
            DemoData.catalogue.map { c in
                ServiceView(service: c.service, name: c.name, kind: c.kind, available: c.available, note: c.note,
                            accounts: s.accounts.filter { $0.service == c.service })
            }
        }
    }

    func gmailStatus() async -> GmailStatus { .ready }

    func accountStatus(account: String) async -> GmailStatus {
        locked { $0.accountStatuses["gmail/\(account)"] ?? .ready }
    }

    func serviceAccountStatus(service: String, account: String) async -> GmailStatus {
        locked { $0.accountStatuses["\(service)/\(account)"] ?? .ready }
    }

    private static func addAccount(_ s: inout State, service: String, account: String) -> AccountView {
        s.accountStatuses["\(service)/\(account)"] = nil
        if let existing = s.accounts.first(where: { $0.service == service && $0.account == account }) { return existing }
        let view = AccountView(service: service, account: account, addedAt: now())
        s.accounts.append(view)
        return view
    }

    func addAccount(hint: String) async throws -> AccountView {
        try await latency(0.6)
        let account = hint.isEmpty ? "new.account@gmail.com" : hint.lowercased()
        return locked { Self.addAccount(&$0, service: "gmail", account: account) }
    }

    func addServiceAccount(service: String, hint: String) async throws -> AccountView {
        try await latency(0.6)
        let account = !hint.isEmpty ? hint.lowercased() : service.hasPrefix("device_") || service == "sms" ? "this phone" : "me@gmail.com"
        return locked { Self.addAccount(&$0, service: service, account: account) }
    }

    func addTokenAccount(service: String, token: String) async throws -> AccountView {
        try await latency(0.8)
        if token.trimmingCharacters(in: .whitespaces).isEmpty || token == "bad" {
            throw CoreError.Service(reason: "\(serviceName(service)) did not accept that token. Check that it is complete and has the scopes listed.")
        }
        let account = switch service {
        case "vault": DemoData.email
        case "github": "octo-cat"
        case "gitlab": "ada.l"
        default: "ada"
        }
        return locked { Self.addAccount(&$0, service: service, account: account) }
    }

    func loginBegin(service: String, phone: String) async throws {
        try await latency(0.6)
        guard phone.filter(\.isNumber).count >= 7 else { throw CoreError.Invalid(reason: "That phone number does not look right. Include the country code.") }
        locked { $0.telegramPhone = phone }
    }

    func loginCode(service: String, code: String) async throws -> LoginProgress {
        try await latency(0.6)
        if code == "00000" { throw CoreError.Invalid(reason: "That code is wrong or has expired. Ask for a new one.") }
        if code == "22222" { return .needsPassword(hint: "pet") }
        return locked { s in
            .done(account: Self.addAccount(&s, service: service, account: s.telegramPhone).account)
        }
    }

    func loginPassword(service: String, password: String) async throws -> AccountView {
        try await latency(0.6)
        if password == "wrong" { throw CoreError.Invalid(reason: "That password is wrong.") }
        return locked { s in Self.addAccount(&s, service: service, account: s.telegramPhone) }
    }

    func removeAccount(account: String) async throws {
        locked { s in
            s.accounts.removeAll { $0.service == "gmail" && $0.account == account }
            s.grants.removeAll { $0.service == "gmail" && $0.account == account }
        }
    }

    func removeServiceAccount(service: String, account: String) async throws {
        locked { s in
            s.accounts.removeAll { $0.service == service && $0.account == account }
            s.grants.removeAll { $0.service == service && $0.account == account }
        }
    }

    // ---- MCP servers --------------------------------------------------------------------------------------------------

    func mcpServers() async throws -> [McpServerView] { locked { $0.mcp } }

    /// "https://mcp.notion.com/mcp" → "notion".
    private static func serverId(_ url: URL) -> String {
        let parts = (url.host ?? "server").split(separator: ".").filter { $0 != "mcp" && $0 != "www" }
        return String(parts.first ?? "server")
    }

    /// A URL with "notion" or "auth" in it needs a sign-in first; any other is added at once with four tools; one
    /// with "fail" in it does not answer.
    func mcpAdd(url: String, name: String?) async throws -> McpAddStep {
        try await latency(0.8)
        guard let parsed = URL(string: url), parsed.scheme == "https" || parsed.scheme == "http", parsed.host != nil else {
            throw CoreError.Invalid(reason: "That is not the address of an MCP server. It starts with https://")
        }
        if url.contains("fail") { throw CoreError.Network(reason: "the server did not answer") }
        let id = Self.serverId(parsed)
        let display = name.flatMap { $0.isEmpty ? nil : $0 } ?? id.prefix(1).uppercased() + id.dropFirst()
        return locked { s in
            if url.contains("notion") || url.contains("auth") {
                if !s.mcp.contains(where: { $0.id == id }) {
                    s.mcp.append(McpServerView(id: id, name: display, url: url, status: "needs_sign_in", error: nil, tools: []))
                }
                return .needsSignIn(serverId: id, authorizeUrl: "https://auth.example.com/authorize?client_id=reins&server=\(id)")
            }
            let server = McpServerView(id: id, name: display, url: url, status: "ok", error: nil, tools: DemoData.mcpTools())
            s.mcp.removeAll { $0.id == id }
            s.mcp.append(server)
            return .added(server: server)
        }
    }

    func mcpAddWithToken(url: String, token: String, name: String?) async throws -> McpServerView {
        try await latency(0.8)
        guard let parsed = URL(string: url), parsed.host != nil else { throw CoreError.Invalid(reason: "That is not the address of an MCP server.") }
        let id = Self.serverId(parsed)
        let server = McpServerView(id: id, name: name.flatMap { $0.isEmpty ? nil : $0 } ?? id.capitalized, url: url, status: "ok", error: nil, tools: DemoData.mcpTools())
        locked { s in
            s.mcp.removeAll { $0.id == id }
            s.mcp.append(server)
        }
        return server
    }

    func mcpFinishSignIn(serverId: String, redirectUrl: String) async throws -> McpServerView {
        try await latency(0.6)
        return try locked { s in
            guard let i = s.mcp.firstIndex(where: { $0.id == serverId }) else { throw CoreError.NotFound }
            s.mcp[i].status = "ok"
            s.mcp[i].error = nil
            s.mcp[i].tools = DemoData.mcpTools()
            return s.mcp[i]
        }
    }

    func mcpRefresh(id: String) async throws -> McpAddStep {
        try await latency(0.8)
        return try locked { s in
            guard let i = s.mcp.firstIndex(where: { $0.id == id }) else { throw CoreError.NotFound }
            if s.mcp[i].status == "needs_sign_in" {
                return .needsSignIn(serverId: id, authorizeUrl: "https://auth.example.com/authorize?client_id=reins&server=\(id)")
            }
            s.mcp[i].status = "ok"
            s.mcp[i].error = nil
            if s.mcp[i].tools.count < 4 { s.mcp[i].tools = DemoData.mcpTools() }
            return .added(server: s.mcp[i])
        }
    }

    func mcpRemove(id: String) async throws {
        locked { s in
            s.mcp.removeAll { $0.id == id }
            s.grants.removeAll { $0.service == "mcp:\(id)" }
        }
    }

    func mcpSetHeavy(id: String, tool: String, heavy: Bool) async throws {
        locked { s in
            guard let i = s.mcp.firstIndex(where: { $0.id == id }) else { return }
            s.mcp[i].tools = s.mcp[i].tools.map { t in
                var t = t
                if t.name == tool { t.heavy = heavy }
                return t
            }
        }
    }

    // ---- Autopilot --------------------------------------------------------------------------------------------------

    private static func defaultProfile(_ s: State) -> ProfileView? { s.profiles.first { $0.isDefault } ?? s.profiles.first }

    private static func globalBase(_ s: State) -> AutopilotMode {
        s.apGlobal.base(now()) ?? .manual
    }

    private static func globalMode(_ s: State) -> AutopilotMode {
        if s.apGlobal.mode == .lockdown { return .lockdown }
        if (s.apGlobal.bypassUntil ?? 0) > now() { return .bypass }
        return globalBase(s)
    }

    /// A connection's mode in force (the core's `modes::effective`): Lockdown, then a bypass, then its own Overnight,
    /// then the global one (not for an AI kept on Manual).
    private static func effective(_ s: State, _ row: ApRow, now: Int64) -> AutopilotMode {
        let global = globalMode(s)
        let own = row.base(now)
        if global == .lockdown || row.mode == .lockdown { return .lockdown }
        if (row.bypassUntil ?? 0) > now || global == .bypass { return .bypass }
        if row.overnighting(now) || (s.apGlobal.overnighting(now) && own != .manual) { return .overnight }
        return own ?? global
    }

    /// A bypass ends the Overnight of its scope, as in the core: when the bypass ends, the row is back in the mode that
    /// Overnight interrupted, never in Overnight without the owner.
    private static func endOvernightForBypass(_ row: inout ApRow) {
        guard row.mode == .overnight else { return }
        row.mode = row.overnightReturn
        row.overnightUntil = nil
        row.overnightSince = nil
        row.overnightReturn = nil
    }

    func setModelRuntime(runtime: ModelRuntime) {}

    func setBackground(background: Bool) {}

    func autopilotSettings() async throws -> AutopilotSettings {
        locked { s in
            let now = Self.now()
            let global = Self.globalMode(s)
            let defaultId = Self.defaultProfile(s)?.id ?? ""
            let rows = s.apConnections.sorted { $0.key < $1.key }.map { id, row in
                let own = row.base(now)
                let mode = Self.effective(s, row, now: now)
                let running = (row.bypassUntil ?? 0) > now
                let night = row.overnighting(now)
                return ConnectionAutopilot(connectionId: id, baseMode: own, bypassUntil: running ? row.bypassUntil : nil,
                                           bypassSince: running ? row.bypassSince : nil,
                                           overnightUntil: night ? row.overnightUntil ?? .max : nil, overnightSince: night ? row.overnightSince : nil,
                                           mode: mode, profileId: row.profileId ?? defaultId)
            }
            let running = (s.apGlobal.bypassUntil ?? 0) > now
            let night = s.apGlobal.overnighting(now)
            return AutopilotSettings(
                mode: global, baseMode: Self.globalBase(s), bypassUntil: running ? s.apGlobal.bypassUntil : nil,
                bypassSince: running ? s.apGlobal.bypassSince : nil,
                overnightUntil: night ? s.apGlobal.overnightUntil ?? .max : nil, overnightSince: night ? s.apGlobal.overnightSince : nil,
                overnightWhenRisky: s.overnightWhenRisky,
                defaultProfileId: defaultId, wifiOnly: s.wifiOnly, autoUpdate: s.autoUpdate, model: s.model, connections: rows
            )
        }
    }

    /// Bypass runs for `minutes` (15 by default); Lockdown also refuses every request that is waiting. As in the core,
    /// it never starts Overnight, nor puts an AI kept on Manual into a global one: that is `startOvernight`, after the
    /// owner's check.
    func setAutopilotMode(connectionId: String?, mode: AutopilotMode?, minutes: UInt32?) async throws {
        if mode == .overnight {
            throw CoreError.Invalid(reason: "Overnight starts with start_overnight, after the owner check")
        }
        try locked { s in
            var row = connectionId.map { s.apConnections[$0] ?? ApRow() } ?? s.apGlobal
            if connectionId != nil && mode != .bypass {
                let now = Self.now()
                var after = row
                after.mode = mode
                after.bypassUntil = nil
                after.overnightUntil = nil
                if Self.effective(s, row, now: now) != .overnight && Self.effective(s, after, now: now) == .overnight {
                    throw CoreError.Invalid(reason: "this would put the AI in Overnight: start it with start_overnight, after the owner check")
                }
            }
            if mode == .bypass {
                Self.endOvernightForBypass(&row)
                row.mode = row.mode == .lockdown ? nil : row.mode
                row.bypassUntil = Self.now() + Int64(minutes ?? 15) * 60
                row.bypassSince = Self.now()
            } else {
                row.mode = mode
                row.bypassUntil = nil
                row.bypassSince = nil
                row.overnightUntil = nil
                row.overnightSince = nil
                row.overnightReturn = nil
            }
            if let connectionId { s.apConnections[connectionId] = row } else { s.apGlobal = row }
            if mode == .lockdown {
                let waiting = s.pending.filter { $0.kind == .request && (connectionId == nil || $0.connectionId == connectionId) }
                for item in waiting {
                    guard let v = s.views[item.id] else { continue }
                    Self.removePending(&s, item.id)
                    Self.logDecision(&s, v, outcome: "denied", selected: [], grantId: nil, decidedBy: "lockdown")
                }
            }
        }
    }

    /// As the core: a minute to a year, or until turned off (`minutes` nil: `bypassUntil` reads `Int64.max`); not for
    /// an AI paired less than 10 minutes ago.
    func startBypass(connectionId: String?, minutes: UInt32?) async throws {
        try locked { s in
            let now = Self.now()
            if let minutes, minutes < 1 || minutes > 525_600 {
                throw CoreError.Invalid(reason: "a bypass lasts from a minute to a year")
            }
            if let connectionId, let c = s.connections.first(where: { $0.id == connectionId }), now - c.createdAt < 600 {
                throw CoreError.Invalid(reason: "a connection paired less than 10 minutes ago cannot be bypassed")
            }
            var row = connectionId.map { s.apConnections[$0] ?? ApRow() } ?? s.apGlobal
            Self.endOvernightForBypass(&row)
            row.mode = row.mode == .lockdown ? nil : row.mode
            row.bypassUntil = minutes.map { now + Int64($0) * 60 } ?? .max
            row.bypassSince = now
            if let connectionId { s.apConnections[connectionId] = row } else { s.apGlobal = row }
        }
    }

    /// As the core: a minute to a year, or until turned off, after the owner confirmed (asked in the core's words); it
    /// keeps the mode it interrupted (Lockdown is not kept: it returns to the default) and leaves a bypass of the same
    /// scope running.
    func startOvernight(connectionId: String?, minutes: UInt32?, owner: OwnerCheck) async throws {
        if let minutes, minutes < 1 || minutes > 525_600 {
            throw CoreError.Invalid(reason: "Overnight lasts between 1 minute and 365 days")
        }
        let reason = switch minutes {
        case let m? where m % 60 == 0: "Start Overnight for \(m / 60) h"
        case let m?: "Start Overnight for \(m) min"
        case nil: "Start Overnight until you turn it off"
        }
        try await Self.confirmOwner(owner, reason)
        try locked { s in
            let now = Self.now()
            var row = connectionId.map { s.apConnections[$0] ?? ApRow() } ?? s.apGlobal
            let back: AutopilotMode? = switch row.mode {
            case .lockdown?: nil
            case .overnight?: row.overnightReturn
            default: row.mode
            }
            row.mode = .overnight
            row.overnightUntil = minutes.map { now + Int64($0) * 60 } ?? .max
            row.overnightSince = now
            row.overnightReturn = back
            if let connectionId { s.apConnections[connectionId] = row } else { s.apGlobal = row }
        }
    }

    /// A change either way asks the owner first, as in the core.
    func setOvernightWhenRisky(risky: OvernightRisky, owner: OwnerCheck) async throws {
        if locked({ $0.overnightWhenRisky }) == risky { return }
        try await Self.confirmOwner(owner, risky == .wait ? "Overnight: wait for you when something is too risky" : "Overnight: deny what is too risky")
        locked { $0.overnightWhenRisky = risky }
    }


    func accountPermissions(service: String, account: String) async throws -> AccountPermissionsView {
        locked { $0.permissionViews["\(service)/\(account)"] ?? DemoPermissions.view(service, account) }
    }
    func accountPermissionDraft(service: String, account: String, preset: AccountPermissionPreset) async throws -> AccountPermissionsDraft {
        DemoPermissions.draft(try await accountPermissions(service: service, account: account), preset)
    }
    func permissionScopeOptions(service: String, account: String, capability: PermissionCapability, query: String, limit: UInt32) async throws -> PermissionScopeOptions {
        let recipient = capability == .mailSend
        let kind: PermissionScopeKind = recipient ? .selectedRecipients : .selectedResources
        let value = query.lowercased()
        return PermissionScopeOptions(kinds: recipient ? [kind] : [.allInAccount, kind], options: query.contains("@") ? [PermissionScopeOption(id: "recipient:" + value, label: value, detail: "Exact address", service: service, account: account, kind: kind)] : [], allowsAddressEntry: recipient, unsupportedReason: nil)
    }
    func previewAccountPermissions(draft: AccountPermissionsDraft) async throws -> PermissionPreview {
        guard try await accountPermissions(service: draft.service, account: draft.account).revision == draft.expectedRevision else { throw CoreError.Invalid(reason: "Permissions changed. Reload current settings.") }
        return DemoPermissions.preview(draft.expectedRevision)
    }
    func saveAccountPermissions(draft: AccountPermissionsDraft, owner: OwnerCheck) async throws -> AccountPermissionsView {
        try await Self.confirmOwner(owner, "Save permissions for \(draft.account)")
        _ = try await previewAccountPermissions(draft: draft)
        var result = try await accountPermissions(service: draft.service, account: draft.account)
        result.revision &+= 1
        result.capabilities = result.capabilities.map { capability in
            var next = capability; next.configuredMode = draft.rules.first(where: { $0.capability == capability.capability })?.mode; return next
        }
        result.expiresAt = draft.expiresAt; result.remainingUses = draft.maxUses
        locked { $0.permissionViews["\(draft.service)/\(draft.account)"] = result }
        return result
    }
    func delegations() async throws -> [DelegationView] { locked { $0.delegationRecords } }
    func delegationDraft(service: String, account: String) async throws -> DelegationDraft {
        let base = try await accountPermissionDraft(service: service, account: account, preset: .readAssist)
        return DelegationDraft(service: service, account: account, expectedRevision: base.expectedRevision, agentIds: base.agentIds, grantAccountAccess: false, rules: base.rules.filter { $0.mode == .auto }, expiresAt: Int64(Date().timeIntervalSince1970) + 3600, maxUses: 20)
    }
    func previewDelegation(draft: DelegationDraft) async throws -> PermissionPreview { DemoPermissions.preview(draft.expectedRevision) }
    func issueDelegation(draft: DelegationDraft, owner: OwnerCheck) async throws -> DelegationView {
        try await Self.confirmOwner(owner, "Create temporary permission for \(draft.account)")
        return locked { state in
            let record = DelegationView(id: "temporary-\(state.delegationRecords.count)", revision: draft.expectedRevision + 1, title: "Temporary account permission", service: draft.service, account: draft.account, state: .active, expiresAt: draft.expiresAt, agentIds: draft.agentIds, rules: draft.rules, used: 0, usageVerified: true, unresolved: 0, maxUses: draft.maxUses, scopeSummary: "Selected agents only", effectiveReason: "Explicit temporary permission", restrictions: [])
            state.delegationRecords.append(record); return record
        }
    }
    func changeDelegation(id: String, expectedRevision: UInt64, change: DelegationChange, owner: OwnerCheck) async throws -> DelegationView {
        try await Self.confirmOwner(owner, "Change temporary permission")
        return try locked { state in
            guard let index = state.delegationRecords.firstIndex(where: { $0.id == id }), state.delegationRecords[index].revision == expectedRevision else { throw CoreError.Invalid(reason: "Permissions changed. Reload current settings.") }
            state.delegationRecords[index].revision &+= 1
            state.delegationRecords[index].state = change == .pause ? .paused : change == .resume ? .active : .revoked
            return state.delegationRecords[index]
        }
    }

    /// Refuses unless the owner confirmed now (a cancel or an error refuses too).
    private static func confirmOwner(_ owner: OwnerCheck, _ reason: String) async throws {
        let confirmed = (try? await owner.confirm(reason: reason)) ?? false
        if !confirmed { throw CoreError.Invalid(reason: "the owner did not confirm") }
    }

    /// From the profiles' fake numbers: what they remember stands for what was scored this week.
    func autopilotStats(days: UInt32) async throws -> AutopilotStats {
        locked { s in
            let classes = s.profiles.flatMap(\.classes)
            let judged = classes.filter { $0.shadowAccuracy != nil }
            let compared = judged.reduce(UInt32(0)) { $0 + $1.decisions }
            let agreed = judged.reduce(Float(0)) { $0 + ($1.shadowAccuracy ?? 0) * Float($1.decisions) }
            let scored = s.profiles.reduce(UInt32(0)) { $0 + $1.memoryCount } + UInt32(s.activity.count)
            return AutopilotStats(
                days: min(max(days, 1), 30),
                scored: s.model.state == .installed ? scored : 0,
                matched: compared > 0 ? agreed / Float(compared) : nil,
                compared: compared,
                kindsOnAuto: UInt32(classes.filter(\.autoApprove).count),
                kinds: UInt32(Set(classes.map(\.classKey)).count)
            )
        }
    }

    func setThresholds(profileId: String, approveAt: Float, denyAt: Float) async throws {
        try locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { throw CoreError.NotFound }
            s.profiles[i].preset = .custom
            s.profiles[i].approveAt = min(max(approveAt, 0.8), 0.99)
            s.profiles[i].denyAt = min(max(denyAt, 0.8), 0.99)
        }
    }

    /// Plausible numbers: lower thresholds decide more and, low enough, decide some the user would have undone.
    func thresholdReplay(profileId: String, approveAt: Float, denyAt: Float) async throws -> ThresholdReplay {
        try locked { s in
            guard let p = s.profiles.first(where: { $0.id == profileId }) else { throw CoreError.NotFound }
            let total = max(p.memoryCount, 53)
            let approve = Double(min(max(approveAt, 0.8), 0.99))
            let deny = Double(min(max(denyAt, 0.8), 0.99))
            let share = min(max(0.25 + (0.99 - approve) * 2.6 + (0.99 - deny) * 1.2, 0), 0.9)
            let decided = UInt32((Double(total) * share).rounded())
            let undone: UInt32 = approve < 0.85 ? 2 : approve < 0.9 ? 1 : 0
            return ThresholdReplay(days: 30, decided: decided, asked: total - decided, undone: min(undone, decided))
        }
    }

    func setAutopilotWifiOnly(wifiOnly: Bool) async throws { locked { $0.wifiOnly = wifiOnly } }

    func setAutopilotAutoUpdate(on: Bool) async throws { locked { $0.autoUpdate = on } }

    func autopilotProfiles() async throws -> [ProfileView] {
        locked { s in
            s.profiles.map { p in
                var p = p
                p.connections = s.apConnections.filter { $0.value.profileId == p.id }.map(\.key).sorted()
                return p
            }
        }
    }

    func createProfile(name: String, icon: String?) async throws -> ProfileView {
        locked { s in
            var n = s.profiles.count + 1
            while s.profiles.contains(where: { $0.id == "p\(n)" }) { n += 1 }
            let p = ProfileView(
                id: "p\(n)", name: name, icon: icon, preset: .balanced, approveAt: 0.95, denyAt: 0.90, agreement: nil, isDefault: false,
                memoryCount: 0, connections: [], classes: [], trainedAt: nil
            )
            s.profiles.append(p)
            return p
        }
    }

    func renameProfile(profileId: String, name: String, icon: String?) async throws {
        try locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { throw CoreError.NotFound }
            s.profiles[i].name = name
            s.profiles[i].icon = icon
        }
    }

    func deleteProfile(profileId: String) async throws {
        try locked { s in
            guard s.profiles.count > 1 else { throw CoreError.Invalid(reason: "the last profile cannot be deleted") }
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { throw CoreError.NotFound }
            let wasDefault = s.profiles[i].isDefault
            s.profiles.remove(at: i)
            if wasDefault { s.profiles[0].isDefault = true }
            for (id, row) in s.apConnections where row.profileId == profileId { s.apConnections[id]?.profileId = nil }
        }
    }

    func resetProfile(profileId: String) async throws {
        locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { return }
            s.profiles[i].memoryCount = 0
            s.profiles[i].classes = []
            s.profiles[i].trainedAt = nil
        }
    }

    func setDefaultProfile(profileId: String) async throws {
        locked { s in
            for i in s.profiles.indices { s.profiles[i].isDefault = s.profiles[i].id == profileId }
        }
    }

    func assignProfile(connectionId: String, profileId: String?) async throws {
        locked { s in
            var row = s.apConnections[connectionId] ?? ApRow()
            row.profileId = profileId
            s.apConnections[connectionId] = row
        }
    }

    /// `locked` true: always asked; false: trusted by hand (no cautious start, no pause); nil: back to Autopilot.
    func setClassLock(profileId: String, classKey: String, locked lockedValue: Bool?) async throws {
        locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { return }
            s.profiles[i].classes = s.profiles[i].classes.map { c in
                guard c.classKey == classKey else { return c }
                var c = c
                c.manual = lockedValue.map { !$0 }
                // as the core: a lock wins, a trust by hand clears a pause, else Auto runs unless paused
                c.autoApprove = lockedValue == false || (lockedValue == nil && !c.paused)
                c.paused = lockedValue == nil && c.paused
                if lockedValue == true { c.autoDeny = false }
                return c
            }
        }
    }

    func setPreset(profileId: String, preset: Preset) async throws {
        locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { return }
            s.profiles[i].preset = preset
            if let t = AutopilotText.presetThresholds(preset) {
                s.profiles[i].approveAt = t.approveAt
                s.profiles[i].denyAt = t.denyAt
            }
        }
    }

    func autopilotSuggestion(requestId: String) async throws -> SuggestionView? {
        locked { $0.model.state == .installed ? $0.suggestions[requestId] : nil }
    }

    func correctDecision(activityId: Int64, shouldHave: Verdict) async throws {
        locked { s in
            guard let i = s.activity.firstIndex(where: { $0.id == activityId }) else { return }
            s.activity[i].autopilot?.correctable = false
        }
    }

    func modelStatus() async -> ModelStatus { locked { $0.model } }

    /// Reports progress over about four seconds, then the model is installed. Cancelling leaves it not installed (or
    /// the older model in place). An update, like the real core's, installs the newer model (which rates risk).
    func downloadModel(progress: DownloadProgress) async throws -> ModelStatus {
        let total = DemoData.modelSize
        let updating = locked { $0.model.state == .installed && $0.model.update }
        locked { $0.model = DemoData.model(.downloading, update: updating) }
        progress.progress(downloaded: 0, total: total)
        let steps: UInt64 = 16
        do {
            for step in 1...steps {
                try await Task.sleep(for: .milliseconds(250))
                let done = total * step / steps
                locked { $0.model.downloadedBytes = done }
                progress.progress(downloaded: done, total: total)
            }
        } catch {
            locked { $0.model = updating ? DemoData.model(.installed, update: true) : DemoData.model(.notInstalled) }
            throw error
        }
        return locked { s in
            s.model = DemoData.model(.installed, risk: updating, version: updating ? "2" : "1", updatedAt: Self.now())
            return s.model
        }
    }

    func deleteModel() async throws {
        locked { $0.model = DemoData.model(.notInstalled) }
    }

    /// The demo's update is too short to stop.
    func pauseModelDownload() async {}

    /// An older model is in use and "Update automatically" is on.
    func modelUpdatePending() async -> Bool {
        locked { $0.autoUpdate && $0.model.state == .installed && $0.model.update }
    }

    /// Like the real core's: an older model is replaced by the newer one (which rates risk), downloaded over about four
    /// seconds while the older one keeps scoring; anything else is up to date.
    func updateModel(automatic: Bool, progress: DownloadProgress) async throws -> ModelUpdateOutcome {
        let (installed, older, off) = locked { ($0.model.state == .installed, $0.model.update, !$0.autoUpdate) }
        guard installed else { return .notInstalled }
        if automatic && off { return .off }
        guard older else { return .upToDate }
        let total = DemoData.modelSize
        locked { $0.model = DemoData.model(.downloading, update: true) }
        progress.progress(downloaded: 0, total: total)
        let steps: UInt64 = 16
        do {
            for step in 1...steps {
                try await Task.sleep(for: .milliseconds(250))
                let done = total * step / steps
                locked { $0.model.downloadedBytes = done }
                progress.progress(downloaded: done, total: total)
            }
        } catch {
            locked { $0.model = DemoData.model(.installed, update: true) }
            throw CoreError.Network(reason: "the download stopped")
        }
        locked { $0.model = DemoData.model(.installed, risk: true, version: "2", updatedAt: Self.now()) }
        return .updated
    }

    /// "Try it": denies what mentions a password, money going out or an unknown address; approves the rest.
    func autopilotEvaluate(profileId: String?, situation: String) async throws -> SuggestionView {
        try await latency(0.7)
        let now = Self.now()
        let text = situation.lowercased()
        let risky = ["password", "wire", "transfer", "unknown", "delete"].contains { text.contains($0) }
        let profile = locked { s in s.profiles.first { $0.id == profileId } ?? Self.defaultProfile(s) }
        var view = risky
            ? DemoData.suggestion(
                "", .deny, pApprove: 0.03, pDeny: 0.95, confidence: 0.88, novel: true,
                reason: "Like 2 times you denied: Send an email · unknown recipient",
                neighbours: [
                    NeighbourView(label: "Send an email · unknown recipient", verdict: .deny, similarity: 0.89, at: now - 86_400),
                    NeighbourView(label: "Forward emails · outside address", verdict: .deny, similarity: 0.81, at: now - 86_400 * 5),
                ],
                classKey: "gmail/send"
            )
            : DemoData.suggestion("", neighbours: DemoData.neighbours(now))
        if let profile {
            view.profileId = profile.id
            view.profileName = profile.name
        }
        return view
    }
}
