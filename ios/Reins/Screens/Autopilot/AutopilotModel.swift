import Foundation
import Observation

/// Autopilot's screens and a connection's Autopilot section (the Android app's `AutopilotViewModel`): modes,
/// bypasses, the model, profiles and "Try it". Each screen keeps one; the modes and the model live in `AppModel`
/// (`autopilot`, `modelDownloads`), so every screen and the widgets show the same.
@Observable
@MainActor
final class AutopilotModel {
    private(set) var profiles: [ProfileView] = []
    private(set) var loaded = false
    private(set) var busy = false
    var error: String?
    /// A short confirmation ("Work is now the default profile.").
    var notice: String?
    /// "Try it": the last answer, and whether one is on its way.
    private(set) var evaluation: SuggestionView?
    private(set) var evaluating = false
    /// "This week": requests scored, how often Autopilot matched the user, kinds on Auto; nil until read.
    private(set) var stats: AutopilotStats?
    /// What a profile's last 30 days would have been at the thresholds being set; nil until read.
    private(set) var replay: ThresholdReplay?
    /// How long an approved email waits before it goes, in seconds (08 R10; 0: off); nil until read.
    private(set) var sendHoldSecs: UInt32?

    @ObservationIgnored private weak var app: AppModel?

    init(app: AppModel? = nil) {
        self.app = app
    }

    /// Screens make their model before the environment is there; they hand it the app model on appear.
    func bind(_ app: AppModel) {
        if self.app !== app {
            self.app = app
            bound += 1
        }
    }

    /// Changes when the app model is handed over, so views that read through it before then draw again.
    private var bound = 0

    var settings: AutopilotSettings? {
        _ = bound
        return app?.autopilot
    }

    /// The model as last read, with the download's own bytes while it runs (the core is not asked for them); an update
    /// coming down shows the same way, the older model still in use.
    var model: ModelStatus? {
        guard var m = settings?.model else { return nil }
        if let updates = app?.modelAutoUpdate, updates.running, updates.total > 0, m.state == .installed || m.state == .downloading {
            m.state = .downloading
            m.update = true
            m.error = nil
            m.downloadedBytes = updates.downloaded
            m.sizeBytes = updates.total
        } else if let downloads = app?.modelDownloads, downloads.job == .running {
            m.state = .downloading
            m.error = nil
            m.downloadedBytes = downloads.downloaded
            if downloads.total > 0 { m.sizeBytes = downloads.total }
        } else if app?.modelDownloads.job == .waiting, m.state == .failed {
            // Asked for again: the old failure no longer applies.
            m.state = .notInstalled
            m.error = nil
        }
        return m
    }

    /// Installed, or an older model still scoring while its update downloads.
    var modelReady: Bool { AutopilotText.modelUsable(model) }

    var downloadJob: DownloadJob {
        _ = bound
        return app?.modelDownloads.job ?? .idle
    }

    /// A waiting download waits for Wi-Fi in particular (else for any network).
    var waitingForWifi: Bool { app?.modelDownloads.waitingForWifi ?? true }

    func refresh() async {
        guard let app else { return }
        await app.refreshAutopilot()
        await loadProfiles()
        await loadSendHold()
    }

    /// The send hold as the core holds it (kept as it was when it cannot be read).
    func loadSendHold() async {
        guard let app else { return }
        if let secs = try? await app.core.sendHoldSecs() { sendHoldSecs = secs }
    }

    private func loadProfiles() async {
        guard let app else { return }
        do {
            profiles = try await app.core.autopilotProfiles()
            loaded = true
        } catch {
            self.error = Self.sentence(error.userMessage)
        }
    }

    func profile(_ id: String) -> ProfileView? { profiles.first { $0.id == id } }

    /// The last `days` of Autopilot, for the stat tiles (kept as they were when it cannot be read).
    func loadStats(days: UInt32 = 7) async {
        guard let app else { return }
        if let read = try? await app.core.autopilotStats(days: days) { stats = read }
    }

    func dismissMessages() {
        error = nil
        notice = nil
    }

    // MARK: Modes

    /// The mode in force for `connectionId` (nil = the global one).
    func modeOf(_ connectionId: String?) -> AutopilotMode? {
        guard let s = settings else { return nil }
        guard let connectionId else { return s.mode }
        return s.connections.first { $0.connectionId == connectionId }?.mode ?? s.mode
    }

    /// A connection paired less than 10 minutes ago cannot be bypassed (the core refuses it too): why, or nil.
    func bypassRefusal(_ connectionId: String?, now: Int64 = Int64(Date().timeIntervalSince1970)) -> String? {
        guard let connectionId, let c = app?.connection(connectionId) else { return nil }
        let left = c.createdAt + Self.newConnectionSecs - now
        guard left > 0 else { return nil }
        let minutes = max((left + 59) / 60, 1)
        return "\(untrusted(c.label)) was paired less than 10 minutes ago, so it cannot be bypassed yet. Try again in \(minutes) min."
    }

    static let newConnectionSecs: Int64 = 600

    /// The mode set for `connectionId` when no bypass runs (nil = the global one); nil when it follows every AI.
    func baseOf(_ connectionId: String?) -> AutopilotMode? {
        guard let s = settings else { return nil }
        guard let connectionId else { return s.baseMode }
        return s.connections.first { $0.connectionId == connectionId }?.baseMode
    }

    /// Sets the global mode or a connection's (`mode` nil: back to following the global mode). Bypass and Overnight
    /// take `minutes` and go through `startBypass` and `startOvernight`, with the owner's check. Lockdown is a plain
    /// tap: it only holds requests back. The change is felt at once; a refusal from the core says why.
    func setMode(_ mode: AutopilotMode?, minutes: UInt32? = nil, connectionId: String? = nil, label: String? = nil) async {
        if mode == .bypass {
            await startBypass(connectionId: connectionId, minutes: minutes ?? AutopilotText.bypassMinutes[0], label: label)
            return
        }
        if mode == .overnight {
            await startOvernight(connectionId: connectionId, minutes: minutes)
            return
        }
        if mode == .auto, connectionId == nil, await guidesToAutoSetup() { return }
        // Leaving a lockdown loosens what runs: only the phone's owner may (Face ID, Touch ID or the passcode), from
        // whichever control asks (the mode sheet, the chip, Trust's card, a widget or an intent).
        if leavesLockdown(mode, connectionId: connectionId) {
            guard let app, await OwnerPrompt.confirm(app, reason: "End lockdown") == .confirmed else { return }
        }
        await applyMode(mode, connectionId: connectionId)
    }

    /// Auto for every AI, picked before its setup was done (01 §9.1): the setup opens instead, and its end turns Auto
    /// on. True when it opened. A setup that cannot be read lets the pick through, as before.
    func guidesToAutoSetup() async -> Bool {
        guard let app, let setup = try? await app.core.autoSetup(), !setup.done else { return false }
        // The Mode sheet that picked it closes first: one presentation at a time.
        try? await Task.sleep(for: .milliseconds(350))
        app.feedback.play(.open)
        app.autoSetupOpen = true
        return true
    }

    /// Whether `mode` would end the lockdown in force for that scope (every AI, or one).
    func leavesLockdown(_ mode: AutopilotMode?, connectionId: String?) -> Bool {
        guard let s = settings, mode != .lockdown else { return false }
        guard let connectionId else { return s.mode == .lockdown }
        return s.connections.first { $0.connectionId == connectionId }?.mode == .lockdown
    }

    /// The change itself, once whatever check it needs has passed.
    private func applyMode(_ mode: AutopilotMode?, connectionId: String?) async {
        // Leaving an Overnight by picking another mode: what it interrupted no longer matters.
        if baseOf(connectionId) == .overnight { OvernightReturn.forget(connectionId) }
        guard let app else { return }
        let old = modeOf(connectionId)
        let new = mode ?? settings?.mode ?? .manual
        if let event = AutopilotFeedback.modeChange(from: old, to: new) {
            app.feedback.play(event)
        }
        app.patchAutopilot { $0.applyMode(mode, minutes: 0, connectionId: connectionId) }
        // Shown at once; the core gets the changes one after another, in the order they were made, so Manual, Auto,
        // Manual, Auto tapped quickly can never land out of order. Only the last one reads the settings back: an
        // earlier read would put a card the user already left back in force for a moment.
        modeGeneration += 1
        let generation = modeGeneration
        let previous = modeChanges
        let change = Task {
            await previous?.value
            await self.run {
                try await app.core.setAutopilotMode(connectionId: connectionId, mode: mode, minutes: nil)
                if generation == self.modeGeneration { await app.refreshAutopilot() }
            }
        }
        modeChanges = change
        await change.value
    }

    /// The mode change on its way to the core (see `setMode`), and how many were made.
    @ObservationIgnored private var modeChanges: Task<Void, Never>?
    @ObservationIgnored private var modeGeneration = 0

    /// Starts a bypass of every AI (`connectionId` nil) or of one, for `minutes` or, nil, until the user turns it off.
    /// The sheet has already been held to confirm; the phone's owner confirms it too (Face ID or the passcode).
    func startBypass(connectionId: String? = nil, minutes: UInt32?, label: String? = nil) async {
        guard let app else { return }
        let who = connectionId == nil ? "every AI" : untrusted(label ?? app.connection(connectionId ?? "")?.label ?? "this AI")
        guard await app.authenticator.confirm("Turn on Bypass for \(who)") else { return }
        let old = modeOf(connectionId)
        if let event = old == .bypass ? .bypassOn : AutopilotFeedback.modeChange(from: old, to: .bypass) {
            app.feedback.play(event)
        }
        let length = minutes.map { min(max($0, 1), AutopilotText.maxBypassMinutes) }
        let back = OvernightReturn.mode(connectionId)
        app.patchAutopilot { $0.applyBypass(minutes: length, connectionId: connectionId, overnightReturn: back) }
        await run {
            try await app.core.startBypass(connectionId: connectionId, minutes: length)
            await app.refreshAutopilot()
        }
    }

    /// Ends a bypass: the global one goes back to the mode it interrupted, a connection's to its own setting (an
    /// Overnight it replaced does not come back: only the owner starts that again).
    func stopBypass(_ connectionId: String? = nil) async {
        guard let app, let s = settings else { return }
        app.feedback.play(.bypassOff)
        app.patchAutopilot { $0.applyBypassEnd(connectionId: connectionId) }
        await run {
            try await app.endBypass(connectionId, in: s)
            await app.refreshAutopilot()
        }
    }

    /// Starts Overnight for every AI (`connectionId` nil) or one, for `minutes` or, nil, until the user turns it off;
    /// a restart or a new length is a start too. The sheet has already been held to confirm; the core then asks the
    /// phone's owner itself (Face ID or the passcode, in its own words), and without that nothing changes, not even for
    /// a moment. It remembers the mode it interrupts, for Stop.
    func startOvernight(connectionId: String? = nil, minutes: UInt32?) async {
        guard let app else { return }
        let old = modeOf(connectionId)
        let base = baseOf(connectionId)
        let length = minutes.map { min(max($0, 1), AutopilotText.maxBypassMinutes) }
        let started = await run {
            try await app.core.startOvernight(connectionId: connectionId, minutes: length, owner: app.ownerCheck)
            await app.refreshAutopilot()
        }
        guard started else { return }
        OvernightReturn.remember(base, connectionId: connectionId)
        if let event = AutopilotFeedback.modeChange(from: old, to: .overnight) {
            app.feedback.play(event)
        }
    }

    /// Stops an Overnight: back to the mode it interrupted (see `OvernightReturn`).
    func stopOvernight(_ connectionId: String? = nil) async {
        guard let app else { return }
        let back = OvernightReturn.mode(connectionId)
        if let event = AutopilotFeedback.modeChange(from: .overnight, to: back ?? settings?.mode ?? .manual) {
            app.feedback.play(event)
        }
        app.patchAutopilot { $0.applyMode(back, minutes: 0, connectionId: connectionId) }
        await run {
            try await app.endOvernight(connectionId)
            await app.refreshAutopilot()
        }
    }

    /// How long what Overnight parks waits after the morning (1 to 48 hours). It is not a loosening (an expired item
    /// is not approved), so no owner check; shown at once, and put back if the core refuses.
    func setOvernightExpiry(_ hours: UInt32) async {
        guard let app, settings?.overnightExpiryHours != hours else { return }
        app.feedback.play(.selection)
        app.patchAutopilot { $0.overnightExpiryHours = hours }
        await run {
            try await app.core.setOvernightExpiry(hours: hours)
            await app.refreshAutopilot()
        }
    }

    /// How long an approved email waits before it goes (0 to 120 s; 0: off). The level of a send is the same with or
    /// without it, so no owner check; shown at once, and put back if the core refuses.
    func setSendHold(_ secs: UInt32) async {
        guard let app, sendHoldSecs != secs else { return }
        app.feedback.play(.selection)
        let before = sendHoldSecs
        sendHoldSecs = secs
        let saved = await run { try await app.core.setSendHoldSecs(secs: secs) }
        if !saved { sendHoldSecs = before }
        await loadSendHold()
    }

    /// Ends the global Lockdown: back to the mode picked before it, or Manual.
    /// Ending a lockdown loosens what runs, so it asks for the owner's check first (Face ID, Touch ID or the passcode),
    /// as approving does; locking down never asks.
    func endLockdown() async {
        guard let s = settings, let app else { return }
        guard await OwnerPrompt.confirm(app, reason: "End lockdown") == .confirmed else { return }
        await applyMode(s.baseMode != .lockdown ? s.baseMode : .manual, connectionId: nil)
    }

    /// Which profile a connection's decisions train (nil = the default profile).
    func assignProfile(_ connectionId: String, _ profileId: String?) async {
        guard let app else { return }
        app.feedback.play(.selection)
        app.patchAutopilot { $0.applyProfile(profileId, connectionId: connectionId) }
        await run {
            try await app.core.assignProfile(connectionId: connectionId, profileId: profileId)
            await app.refreshAutopilot()
            await self.loadProfiles()
        }
    }

    // MARK: The model

    /// Starts the download, or waits for Wi-Fi. Asking before using mobile data is the screen's part.
    func download() {
        guard let app else { return }
        error = nil
        app.modelDownloads.start(wifiOnly: settings?.wifiOnly ?? true)
    }

    /// The user agreed to use mobile data this once.
    func downloadNow() {
        guard let app else { return }
        error = nil
        app.modelDownloads.startNow()
    }

    /// Only a download still waiting for its network can be called off.
    func cancelDownload() {
        app?.modelDownloads.cancel()
    }

    func needsMobileDataConsent() -> Bool {
        app?.modelDownloads.needsMobileDataConsent(wifiOnly: settings?.wifiOnly ?? true) ?? false
    }

    func deleteModel() async {
        guard let app else { return }
        app.feedback.play(.revoked)
        await run {
            try await app.core.deleteModel()
            await app.refreshAutopilot()
        }
    }

    func setWifiOnly(_ on: Bool) async {
        guard let app else { return }
        app.feedback.play(.toggle(on))
        app.modelDownloads.setWifiOnly(on)
        app.patchAutopilot { $0.wifiOnly = on }
        await run {
            try await app.core.setAutopilotWifiOnly(wifiOnly: on)
            await app.refreshAutopilot()
        }
    }

    /// "Update automatically": a newer model comes down by itself on Wi-Fi. Turned on, a due update goes now (or waits
    /// for Wi-Fi).
    func setAutoUpdate(_ on: Bool) async {
        guard let app else { return }
        app.feedback.play(.toggle(on))
        app.patchAutopilot { $0.autoUpdate = on }
        let done = await run {
            try await app.core.setAutopilotAutoUpdate(on: on)
            await app.refreshAutopilot()
        }
        if done, on { Task { await app.modelAutoUpdate.check(.switchedOn) } }
    }

    /// "Check now" runs (an update it found included); waiting for Wi-Fi does not count.
    var checkingModel: Bool {
        _ = bound
        guard let updates = app?.modelAutoUpdate else { return false }
        return updates.checking || updates.running
    }

    /// What the last "Check now" came to; nil before it ran and while it runs.
    var modelCheckResult: ModelCheckResult? {
        _ = bound
        guard let updates = app?.modelAutoUpdate, !updates.checking else { return nil }
        return updates.checkResult
    }

    /// The user's "Check now": the update, if there is one, comes down now (on mobile data only with Wi-Fi only off).
    func checkModelNow() async {
        guard let app else { return }
        error = nil
        await app.modelAutoUpdate.checkNow()
    }

    // MARK: Profiles

    /// Creates a profile and returns its id (nil when it was not made).
    @discardableResult
    func createProfile(name: String, icon: String?) async -> String? {
        guard let app else { return nil }
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return nil }
        app.feedback.play(.grantCreated)
        var id: String?
        await run {
            let profile = try await app.core.createProfile(name: clean, icon: icon)
            id = profile.id
            await self.loadProfiles()
        }
        return id
    }

    func renameProfile(_ id: String, name: String, icon: String?) async {
        guard let app else { return }
        let clean = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        await run {
            try await app.core.renameProfile(profileId: id, name: clean, icon: icon)
            await self.loadProfiles()
        }
    }

    func makeDefault(_ id: String) async {
        guard let app else { return }
        app.feedback.play(.selection)
        await run {
            try await app.core.setDefaultProfile(profileId: id)
            await self.loadProfiles()
            await app.refreshAutopilot()
            if let name = self.profile(id)?.name { self.notice = "\(untrusted(name)) is now the default profile." }
        }
    }

    /// Shown at once; a refusal reads the profiles back.
    func setPreset(_ id: String, _ preset: Preset) async {
        guard let app else { return }
        patchProfile(id) { profile in
            profile.preset = preset
            if let t = AutopilotText.presetThresholds(preset) {
                profile.approveAt = t.approveAt
                profile.denyAt = t.denyAt
            }
        }
        let done = await run {
            try await app.core.setPreset(profileId: id, preset: preset)
            await self.loadProfiles()
        }
        if !done { await loadProfiles() }
    }

    /// The profile's own thresholds (the Custom preset), each 80% to 99%. Shown at once; a refusal reads the profiles
    /// back.
    func setThresholds(_ id: String, approveAt: Float, denyAt: Float) async {
        guard let app else { return }
        let approve = Self.clampThreshold(approveAt)
        let deny = Self.clampThreshold(denyAt)
        patchProfile(id) { profile in
            profile.preset = .custom
            profile.approveAt = approve
            profile.denyAt = deny
        }
        let done = await run {
            try await app.core.setThresholds(profileId: id, approveAt: approve, denyAt: deny)
            await self.loadProfiles()
        }
        if !done { await loadProfiles() }
    }

    /// What the last 30 days would have been at these thresholds (kept as it was when it cannot be read).
    func loadReplay(_ id: String, approveAt: Float, denyAt: Float) async {
        guard let app else { return }
        if let read = try? await app.core.thresholdReplay(
            profileId: id, approveAt: Self.clampThreshold(approveAt), denyAt: Self.clampThreshold(denyAt)
        ) {
            replay = read
        }
    }

    /// 0.80 to 0.99, as the core keeps them.
    nonisolated static func clampThreshold(_ p: Float) -> Float {
        Float(AutopilotText.thresholdPercent(p)) / 100
    }

    /// `locked`: true always asks, false lets Auto approve now, nil lets the numbers decide. Shown at once; a refusal
    /// reads the profiles back.
    func setClassLock(_ id: String, _ classKey: String, locked: Bool?) async {
        guard let app else { return }
        // The chips sound their own choice; unlocking (after its warning) is felt as more autonomy.
        if locked == false { app.feedback.play(.autopilotOn) }
        patchProfile(id) { profile in
            guard let i = profile.classes.firstIndex(where: { $0.classKey == classKey }) else { return }
            // `manual`: false locked by hand, true unlocked by hand, nil automatic.
            profile.classes[i].manual = locked.map { !$0 }
            if let locked {
                profile.classes[i].autoApprove = !locked
                // a lock or a trust by hand: no pause shows
                profile.classes[i].paused = false
            }
        }
        let done = await run {
            try await app.core.setClassLock(profileId: id, classKey: classKey, locked: locked)
            await self.loadProfiles()
        }
        if !done { await loadProfiles() }
    }

    private func patchProfile(_ id: String, _ change: (inout ProfileView) -> Void) {
        guard let i = profiles.firstIndex(where: { $0.id == id }) else { return }
        change(&profiles[i])
    }

    func resetProfile(_ id: String) async {
        guard let app else { return }
        app.feedback.play(.revoked)
        await run {
            try await app.core.resetProfile(profileId: id)
            await self.loadProfiles()
            self.notice = "Forgotten. This profile starts learning again from your next answer."
        }
    }

    /// True when it was deleted (the screen then closes).
    @discardableResult
    func deleteProfile(_ id: String) async -> Bool {
        guard let app else { return false }
        app.feedback.play(.revoked)
        var done = false
        await run {
            try await app.core.deleteProfile(profileId: id)
            await self.loadProfiles()
            await app.refreshAutopilot()
            done = true
        }
        return done
    }

    // MARK: Try it

    func evaluate(profileId: String?, situation: String) async {
        guard let app else { return }
        let text = situation.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !evaluating else { return }
        evaluating = true
        error = nil
        do {
            let answer = try await app.core.autopilotEvaluate(profileId: profileId, situation: text)
            app.feedback.play(.refresh)
            evaluation = answer
        } catch {
            app.feedback.play(.error)
            self.error = Self.sentence(error.userMessage)
        }
        evaluating = false
    }

    func clearEvaluation() {
        evaluation = nil
    }

    // MARK: Running

    /// True when `block` went through.
    @discardableResult
    private func run(_ block: () async throws -> Void) async -> Bool {
        busy = true
        error = nil
        notice = nil
        defer { busy = false }
        do {
            try await block()
            return true
        } catch {
            app?.feedback.play(.error)
            self.error = Self.sentence(error.userMessage)
            await app?.refreshAutopilot()
            return false
        }
    }

    /// The core's reasons start in lower case and end without a full stop ("a connection paired ... in bypass").
    nonisolated static func sentence(_ text: String) -> String {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let first = t.first else { return t }
        let s = first.uppercased() + t.dropFirst()
        return s.hasSuffix(".") || s.hasSuffix("?") || s.hasSuffix("!") ? s : s + "."
    }
}

/// The core's rules for a mode change, applied to the settings in memory so the screens show it before the core
/// answers (the core's `set_autopilot_mode` and `modes::effective`). The next read replaces it with the real thing.
extension AutopilotSettings {
    /// `mode` for `connectionId` (nil = the global mode); `mode` nil goes back to the default (global) or to following
    /// the global mode (a connection). A bypass lasts `minutes` (see `applyBypass`).
    mutating func applyMode(_ mode: AutopilotMode?, minutes: UInt32, connectionId: String?,
                            now: Int64 = Int64(Date().timeIntervalSince1970)) {
        if mode == .bypass {
            applyBypass(minutes: minutes, connectionId: connectionId, now: now)
            return
        }
        if mode == .overnight {
            applyOvernight(minutes: minutes, connectionId: connectionId, now: now)
            return
        }
        if let connectionId {
            let i = ownIndex(connectionId)
            connections[i].baseMode = mode
            connections[i].bypassUntil = nil
            connections[i].bypassSince = nil
            connections[i].overnightUntil = nil
            connections[i].overnightSince = nil
        } else {
            // A mode never picked is Manual.
            baseMode = mode ?? .manual
            bypassUntil = nil
            bypassSince = nil
            overnightUntil = nil
            overnightSince = nil
        }
        settle(now: now)
    }

    /// Overnight for every AI (`connectionId` nil) or one, from `now` for `minutes`, or until turned off (nil). It is
    /// the mode set there while it runs (a lockdown it replaces ends); a bypass of the same scope stays on top.
    mutating func applyOvernight(minutes: UInt32?, connectionId: String?, now: Int64 = Int64(Date().timeIntervalSince1970)) {
        let until = minutes.map { now + Int64($0) * 60 } ?? AutopilotText.untilOff
        if let connectionId {
            let i = ownIndex(connectionId)
            connections[i].baseMode = .overnight
            connections[i].overnightUntil = until
            connections[i].overnightSince = now
        } else {
            baseMode = .overnight
            overnightUntil = until
            overnightSince = now
        }
        settle(now: now)
    }

    /// The bypass of every AI (`connectionId` nil) or of one ends; the mode under it stays (never an Overnight: the
    /// bypass ended that).
    mutating func applyBypassEnd(connectionId: String?, now: Int64 = Int64(Date().timeIntervalSince1970)) {
        if let connectionId {
            let i = ownIndex(connectionId)
            connections[i].bypassUntil = nil
            connections[i].bypassSince = nil
        } else {
            bypassUntil = nil
            bypassSince = nil
        }
        settle(now: now)
    }

    /// A bypass of every AI (`connectionId` nil) or of one, from `now` for `minutes`, or until turned off (nil). It
    /// ends a lockdown of its scope, and an Overnight: under the bypass is then `overnightReturn`, the mode that
    /// Overnight interrupted (nil: the default, or following every AI).
    mutating func applyBypass(minutes: UInt32?, connectionId: String?, overnightReturn: AutopilotMode? = nil,
                              now: Int64 = Int64(Date().timeIntervalSince1970)) {
        let until = minutes.map { now + Int64($0) * 60 } ?? AutopilotText.untilOff
        if let connectionId {
            let i = ownIndex(connectionId)
            if connections[i].baseMode == .lockdown { connections[i].baseMode = nil }
            if connections[i].baseMode == .overnight {
                connections[i].baseMode = overnightReturn
                connections[i].overnightUntil = nil
                connections[i].overnightSince = nil
            }
            connections[i].bypassUntil = until
            connections[i].bypassSince = now
        } else {
            if baseMode == .lockdown { baseMode = .manual }
            if baseMode == .overnight {
                baseMode = overnightReturn ?? .manual
                overnightUntil = nil
                overnightSince = nil
            }
            bypassUntil = until
            bypassSince = now
        }
        settle(now: now)
    }

    /// The modes in force after a change (the core's `modes::effective`).
    private mutating func settle(now: Int64) {
        mode = baseMode == .lockdown ? .lockdown : (bypassUntil ?? 0) > now ? .bypass : baseMode
        let globalOvernight = baseMode == .overnight && (overnightUntil ?? 0) > now
        for i in connections.indices {
            let c = connections[i]
            if mode == .lockdown || c.baseMode == .lockdown {
                connections[i].mode = .lockdown
            } else if (c.bypassUntil ?? 0) > now || (bypassUntil ?? 0) > now {
                // Its own bypass, or the global one: a global bypass applies whatever mode an AI has of its own.
                connections[i].mode = .bypass
            } else if (c.baseMode == .overnight && (c.overnightUntil ?? 0) > now) || (globalOvernight && c.baseMode != .manual) {
                // Its own Overnight, or the global one, which never loosens an AI kept on Manual.
                connections[i].mode = .overnight
            } else {
                connections[i].mode = c.baseMode ?? mode
            }
        }
    }

    /// The profile `connectionId` learns into (nil = the default profile).
    mutating func applyProfile(_ profileId: String?, connectionId: String) {
        let i = ownIndex(connectionId)
        connections[i].profileId = profileId ?? defaultProfileId
    }

    /// The index of `connectionId`'s own settings, added (following the global mode) when it has none yet.
    private mutating func ownIndex(_ connectionId: String) -> Int {
        if let i = connections.firstIndex(where: { $0.connectionId == connectionId }) { return i }
        connections.append(ConnectionAutopilot(
            connectionId: connectionId, baseMode: nil, bypassUntil: nil, bypassSince: nil, mode: mode, profileId: defaultProfileId
        ))
        return connections.count - 1
    }
}

/// What switching modes sounds and feels like (the Android app's `AutopilotText.modeChangeEvent`).
enum AutopilotFeedback {
    /// nil when nothing changes.
    static func modeChange(from old: AutopilotMode?, to new: AutopilotMode) -> FeedbackEvent? {
        if old == new { return nil }
        if new == .bypass { return .bypassOn }
        if new == .lockdown { return .lockdownOn }
        if old == .bypass { return .bypassOff }
        if old == .lockdown { return .lockdownOff }
        return .autopilotModeChanged(moreAutonomy: AutopilotText.autonomy(new) > AutopilotText.autonomy(old ?? .manual))
    }
}
