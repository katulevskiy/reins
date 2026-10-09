import Foundation

/// The `-demo` launch argument's in-memory core (screenshots, UI tests, a look around without a server).
///
/// Launch arguments: `-signedout` starts signed out (any password signs in; an email containing "2fa" also needs a
/// code; creating an account works like signing in), `-demoArrive` makes a new request arrive about 6 s after launch,
/// `-demoNoModel` starts with the Autopilot model not downloaded. Any pairing code pairs a desktop app, except those
/// starting with "BBBB", which have expired. "Continue" (passwordless sign-in) makes a new account; `-demoLocked` finds
/// one whose keys are on another phone instead (the recovery code is `DemoData.recoveryCode`; asking the other phone
/// works after two polls, `-demoJoinDenied` refuses; resetting the vault starts it over as a new account).
/// `-demoJoin` has another phone ask this one for the keys.
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
            keysLocked: args.contains("-demoLocked"),
            joinWaiting: args.contains("-demoJoin"),
            approvalElsewhere: otherPhone,
            passkeys: !args.contains("-demoNoPasskeys"),
            passkeysOffline: args.contains("-demoPasskeysOffline")
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
    }

    struct State {
        var session: SessionInfo?
        var pending: [PendingItem] = []
        /// Bumped by every change to `pending`, so a waiting `sync` can return early.
        var pendingVersion = 0
        var views: [String: ApprovalView] = [:]
        var pairings: [String: PairingView] = [:]
        var blobs: [String: BlobView] = [:]
        var connections: [ConnectionView] = []
        var activity: [ActivityEntry] = []
        var grants: [GrantView] = []
        var accounts: [AccountView] = []
        var accountStatuses: [String: GmailStatus] = [:]
        var emails: [String: EmailContent] = [:]
        var mcp: [McpServerView] = []
        var telegramPhone = ""
        var apGlobal = ApRow()
        var apConnections: [String: ApRow] = [:]
        var model: ModelStatus
        var profiles: [ProfileView] = []
        var wifiOnly = true
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
        signedIn: Bool = true, arriveAfter: Int64? = nil, modelInstalled: Bool = true, syncCap: Double = 5, keysLocked: Bool = false,
        joinWaiting: Bool = false, approvalElsewhere: Bool = false, passkeys: Bool = true, passkeysOffline: Bool = false
    ) {
        let now = Self.now()
        let seeded = DemoData.pending(now)
        var s = State(model: modelInstalled ? DemoData.model(.installed) : DemoData.model(.notInstalled))
        s.session = signedIn ? SessionInfo(serverUrl: DemoData.server, email: DemoData.email) : nil
        s.pending = seeded.items
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

    /// "Continue": the browser part is skipped (`ssoBegin`'s URL is a data page the app never opens in the demo); the
    /// account is `Created`, or `Locked` with `-demoLocked` (another phone has its keys).
    /// The demo says nothing about its server: the sign-in shows every way in.
    func serverInfo(serverUrl: String) async throws -> ServerInfo {
        ServerInfo(browserSignIn: nil, pushAndroid: nil, pushIos: nil)
    }

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

    /// A desktop app on "studio" asks to pair, parked like a pushed pairing. Codes starting with "BBBB" have expired.
    func pairingByCode(userCode: String) async throws -> PairingView {
        try await latency()
        guard let code = PairingCode.normalize(userCode) else { throw CoreError.Invalid(reason: "That is not a pairing code.") }
        if code.hasPrefix("BBBB") { throw CoreError.NotFound }
        return locked { s in
            let id = "code-\(code)"
            if let parked = s.pairings[id] { return parked }
            let p = PairingView(
                id: id, clientName: "Reins desktop app on studio", clientHost: "studio", choices: Data([47, 12, 83]),
                createdAt: Self.now(), keyFingerprint: "3170 6422"
            )
            s.pairings[id] = p
            s.pending.insert(DemoData.pairingItem(p), at: 0)
            s.pendingVersion += 1
            return p
        }
    }

    func pairingView(pairingId: String) async throws -> PairingView {
        try locked { s in
            guard let p = s.pairings[pairingId] else { throw CoreError.NotFound }
            return p
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

    func answerPairing(pairingId: String, approve: Bool, chosenCode: UInt8?, label: String?) async throws {
        try await latency()
        try locked { s in
            guard let p = s.pairings[pairingId] else { throw CoreError.NotFound }
            Self.removePending(&s, pairingId)
            s.pairings[pairingId] = nil
            guard approve else { return }
            s.nextConnection += 1
            let name = label.map { $0.trimmingCharacters(in: .whitespaces) }.flatMap { $0.isEmpty ? nil : $0 } ?? p.clientName
            s.connections.append(ConnectionView(id: "c\(s.nextConnection)", label: name, clientHost: p.clientHost, createdAt: Self.now(), lastUsedAt: nil, icon: nil, keyFingerprint: p.keyFingerprint))
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
        s.apGlobal.mode ?? (s.model.state == .installed ? .assisted : .manual)
    }

    private static func globalMode(_ s: State) -> AutopilotMode {
        if s.apGlobal.mode == .lockdown { return .lockdown }
        if (s.apGlobal.bypassUntil ?? 0) > now() { return .bypass }
        return globalBase(s)
    }

    func setModelRuntime(runtime: ModelRuntime) {}

    func autopilotSettings() async throws -> AutopilotSettings {
        locked { s in
            let now = Self.now()
            let global = Self.globalMode(s)
            let defaultId = Self.defaultProfile(s)?.id ?? ""
            let rows = s.apConnections.sorted { $0.key < $1.key }.map { id, row in
                let mode: AutopilotMode = if global == .lockdown || row.mode == .lockdown {
                    .lockdown
                } else if (row.bypassUntil ?? 0) > now {
                    .bypass
                } else {
                    row.mode ?? global
                }
                return ConnectionAutopilot(connectionId: id, baseMode: row.mode, bypassUntil: row.bypassUntil.flatMap { $0 > now ? $0 : nil },
                                           mode: mode, profileId: row.profileId ?? defaultId)
            }
            return AutopilotSettings(
                mode: global, baseMode: Self.globalBase(s), bypassUntil: s.apGlobal.bypassUntil.flatMap { $0 > now ? $0 : nil },
                defaultProfileId: defaultId, wifiOnly: s.wifiOnly, model: s.model, connections: rows
            )
        }
    }

    /// Bypass runs for `minutes` (15 by default); Lockdown also refuses every request that is waiting.
    func setAutopilotMode(connectionId: String?, mode: AutopilotMode?, minutes: UInt32?) async throws {
        locked { s in
            var row = connectionId.map { s.apConnections[$0] ?? ApRow() } ?? s.apGlobal
            if mode == .bypass {
                row.mode = row.mode == .lockdown ? nil : row.mode
                row.bypassUntil = Self.now() + Int64(minutes ?? 15) * 60
            } else {
                row.mode = mode
                row.bypassUntil = nil
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

    func setAutopilotWifiOnly(wifiOnly: Bool) async throws { locked { $0.wifiOnly = wifiOnly } }

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
            let p = ProfileView(id: "p\(n)", name: name, icon: icon, preset: .balanced, isDefault: false, memoryCount: 0, connections: [], classes: [], trainedAt: nil)
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

    /// `locked` true: always asked; false: unlocked by hand (decides on its own); nil: back to what the numbers say.
    func setClassLock(profileId: String, classKey: String, locked lockedValue: Bool?) async throws {
        locked { s in
            guard let i = s.profiles.firstIndex(where: { $0.id == profileId }) else { return }
            s.profiles[i].classes = s.profiles[i].classes.map { c in
                guard c.classKey == classKey else { return c }
                var c = c
                c.manual = lockedValue.map { !$0 }
                c.autoApprove = lockedValue == false || (lockedValue == nil && c.decisionsToUnlock == 0 && (c.shadowAccuracy ?? 0) >= 0.95)
                if lockedValue == true { c.autoDeny = false }
                return c
            }
        }
    }

    func setPreset(profileId: String, preset: Preset) async throws {
        locked { s in
            if let i = s.profiles.firstIndex(where: { $0.id == profileId }) { s.profiles[i].preset = preset }
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

    /// Reports progress over about four seconds, then the model is installed. Cancelling leaves it not installed.
    func downloadModel(progress: DownloadProgress) async throws -> ModelStatus {
        let total = DemoData.modelSize
        locked { $0.model = DemoData.model(.downloading) }
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
            locked { $0.model = DemoData.model(.notInstalled) }
            throw error
        }
        return locked { s in
            s.model = DemoData.model(.installed)
            return s.model
        }
    }

    func deleteModel() async throws {
        locked { $0.model = DemoData.model(.notInstalled) }
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
