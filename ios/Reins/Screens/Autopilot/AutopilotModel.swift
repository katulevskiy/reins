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

    /// The model as last read, with the download's own bytes while it runs (the core is not asked for them).
    var model: ModelStatus? {
        guard var m = settings?.model else { return nil }
        if let downloads = app?.modelDownloads, downloads.job == .running {
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

    var modelReady: Bool { settings?.model.state == .installed }

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

    /// Sets the global mode or a connection's (`mode` nil: back to following the global mode). Bypass takes
    /// `minutes`. Bypass and Lockdown are confirmed by the phone's owner first (Face ID or the passcode). The change
    /// is felt at once; a refusal from the core says why.
    func setMode(_ mode: AutopilotMode?, minutes: UInt32? = nil, connectionId: String? = nil, label: String? = nil) async {
        guard let app else { return }
        let old = modeOf(connectionId)
        let new = mode ?? settings?.mode ?? .manual
        if mode == .bypass || mode == .lockdown {
            let who = connectionId == nil ? "every AI" : untrusted(label ?? app.connection(connectionId ?? "")?.label ?? "this AI")
            let reason = mode == .bypass ? "Turn on Bypass for \(who)" : "Lock down \(who)"
            guard await app.authenticator.confirm(reason) else { return }
        }
        let restart = mode == .bypass && old == .bypass
        if let event = restart ? .bypassOn : AutopilotFeedback.modeChange(from: old, to: new) {
            app.feedback.play(event)
        }
        await run {
            try await app.core.setAutopilotMode(
                connectionId: connectionId, mode: mode,
                minutes: mode == .bypass ? (minutes ?? AutopilotText.bypassMinutes[0]) : nil
            )
            await app.refreshAutopilot()
        }
    }

    /// Ends a bypass: the global one goes back to the mode it interrupted, a connection's to its own setting.
    func stopBypass(_ connectionId: String? = nil) async {
        guard let app, let s = settings else { return }
        app.feedback.play(.bypassOff)
        await run {
            if let connectionId {
                let own = s.connections.first { $0.connectionId == connectionId }?.baseMode
                try await app.core.setAutopilotMode(connectionId: connectionId, mode: own, minutes: nil)
            } else {
                try await app.core.setAutopilotMode(connectionId: nil, mode: s.baseMode, minutes: nil)
            }
            await app.refreshAutopilot()
        }
    }

    /// Ends the global Lockdown: back to the mode picked before it, or Manual.
    func endLockdown() async {
        guard let s = settings else { return }
        await setMode(s.baseMode != .lockdown ? s.baseMode : .manual)
    }

    /// Which profile a connection's decisions train (nil = the default profile).
    func assignProfile(_ connectionId: String, _ profileId: String?) async {
        guard let app else { return }
        app.feedback.play(.selection)
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
        await run {
            try await app.core.setAutopilotWifiOnly(wifiOnly: on)
            await app.refreshAutopilot()
        }
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

    func setPreset(_ id: String, _ preset: Preset) async {
        guard let app else { return }
        await run {
            try await app.core.setPreset(profileId: id, preset: preset)
            await self.loadProfiles()
        }
    }

    /// `locked`: true always asks, false lets Auto approve now, nil lets the numbers decide.
    func setClassLock(_ id: String, _ classKey: String, locked: Bool?) async {
        guard let app else { return }
        // The chips sound their own choice; unlocking (after its warning) is felt as more autonomy.
        if locked == false { app.feedback.play(.autopilotOn) }
        await run {
            try await app.core.setClassLock(profileId: id, classKey: classKey, locked: locked)
            await self.loadProfiles()
        }
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

    private func run(_ block: () async throws -> Void) async {
        busy = true
        error = nil
        notice = nil
        do {
            try await block()
        } catch {
            app?.feedback.play(.error)
            self.error = Self.sentence(error.userMessage)
            await app?.refreshAutopilot()
        }
        busy = false
    }

    /// The core's reasons start in lower case and end without a full stop ("a connection paired ... in bypass").
    nonisolated static func sentence(_ text: String) -> String {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let first = t.first else { return t }
        let s = first.uppercased() + t.dropFirst()
        return s.hasSuffix(".") || s.hasSuffix("?") || s.hasSuffix("!") ? s : s + "."
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
