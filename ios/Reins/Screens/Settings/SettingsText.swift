import SwiftUI

/// What Settings and a connection's page say about Autopilot and the other switches (pure, unit-tested). The
/// Autopilot screens have their own, fuller set; these are the few words these pages need.
enum SettingsText {
    /// The modes in the order the picker shows them, from the user deciding everything to Autopilot refusing everything.
    static let modes: [AutopilotMode] = [.manual, .assisted, .auto, .bypass, .lockdown]

    /// How long a bypass can run, in minutes (the core allows 1 to 60).
    static let bypassMinutes: [UInt32] = [15, 30, 60]

    static func modeName(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "Manual"
        case .assisted: "Assisted"
        case .auto: "Auto"
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    static func modeSymbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "hand.raised"
        case .assisted: "sparkles"
        case .auto: "location.north.fill"
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        }
    }

    static func modeTint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.secondary
        case .assisted: Palette.search
        case .auto: Palette.accent
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    /// What the Autopilot row says: the mode, and whether the model is here.
    static func autopilotSummary(_ s: AutopilotSettings?) -> String {
        guard let s else { return "Manual" }
        let model = switch s.model.state {
        case .installed: "model on this phone"
        case .downloading: "downloading the model"
        default: "no model yet"
        }
        return "\(modeName(s.mode)) · \(model)"
    }

    /// What the Sounds & haptics row says about the switches behind it.
    static func soundsSummary(_ s: FeedbackSettings) -> String {
        if !s.master { return "Off" }
        switch (s.soundsOn, s.hapticsOn) {
        case (true, true): return "Sounds and haptics on"
        case (true, false): return "Sounds on, haptics off"
        case (false, true): return "Haptics on, sounds off"
        case (false, false): return "Off"
        }
    }

    /// "1 account connected".
    static func accountsLine(_ n: Int) -> String {
        switch n {
        case 0: "Connect Gmail and more"
        case 1: "1 account connected"
        default: "\(n) accounts connected"
        }
    }

    /// What the Vault passkeys row says: how many there are (`count` nil while unknown).
    static func passkeysSummary(_ count: Int?) -> String {
        switch count {
        case nil: "Not set up"
        case 0: "None"
        case 1: "1 passkey"
        case let n?: "\(n) passkeys"
        }
    }

    /// "Added 9 Oct 2026", in the phone's own date format.
    static func passkeyAdded(_ epochSeconds: Int64) -> String {
        "Added " + Date(timeIntervalSince1970: TimeInterval(epochSeconds)).formatted(date: .abbreviated, time: .omitted)
    }

    /// "claude.ai · used 5 min ago".
    static func connectionLine(_ c: ConnectionView, now: Int64 = nowSeconds()) -> String {
        let used = c.lastUsedAt.map { "used \(GrantText.relative($0, now: now))" } ?? "never used"
        // A computer: the key compared when it paired says more than the address the server saw.
        if let key = c.keyFingerprint { return "Key \(key) · " + used }
        return untrusted(c.clientHost) + " · " + used
    }

    /// How requests reach this phone: by push when both this app and the server can send it (`serverPush` nil: the
    /// server does not say), otherwise only while the app is open.
    static func approvalFooter(approvalDevice: Bool, appPush: Bool, serverPush: Bool?) -> String {
        if !approvalDevice { return "Only one phone at a time approves requests. Use this one to take over." }
        if !appPush { return "Push notifications are not set up yet. Requests arrive while the app is open." }
        if serverPush == false { return "This server sends no push notifications: requests arrive only while Reins is open." }
        return "Requests reach this phone by push notification and while the app is open."
    }

    /// "42 min left", "1 min left" (the last minute counts as one).
    static func minutesLeft(_ until: Int64, now: Int64) -> String {
        let left = max(until - now, 0)
        return "\(max((left + 59) / 60, 1)) min left"
    }

    /// How much a mode lets happen without the user: Lockdown least, Bypass most.
    static func autonomy(_ mode: AutopilotMode) -> Int { mode.autonomy }

    /// What switching from `old` to `new` sounds and feels like; nil when nothing changes.
    static func modeChangeEvent(_ old: AutopilotMode?, _ new: AutopilotMode) -> FeedbackEvent? {
        if old == new { return nil }
        if new == .bypass { return .bypassOn }
        if new == .lockdown { return .lockdownOn }
        if old == .bypass { return .bypassOff }
        if old == .lockdown { return .lockdownOff }
        return .autopilotModeChanged(moreAutonomy: new.autonomy > (old ?? .manual).autonomy)
    }

    /// The emoji a profile shows.
    static func profileIcon(_ p: ProfileView) -> String {
        // A plain word ("person", "work" in accounts made before the default profiles had emoji) is no icon.
        if let icon = p.icon?.trimmingCharacters(in: .whitespaces), !icon.isEmpty,
           !icon.allSatisfy({ $0.isASCII && $0.isLetter }) { return icon }
        return p.name.lowercased() == "work" ? "💼" : "🙂"
    }

    /// What "Delete account" deletes, line by line, before it asks for the email.
    static let deletionItems = [
        "Your account and its vault on the server",
        "Your AI and computer connections, and their access",
        "Your approval phone, permissions, activity and files",
        "Your sign-in: this email would start a new, empty account",
        "Everything Reins keeps for this account on this phone",
    ]

    /// Whether the typed text names the account's email (case and surrounding spaces do not count), as the server
    /// checks it.
    static func deletionConfirmed(_ typed: String, email: String) -> Bool {
        let typed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        return !typed.isEmpty && typed.lowercased() == email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// "1.4 (27)": the version and build of this app.
    static func version(_ info: [String: Any]? = Bundle.main.infoDictionary) -> String {
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(version) (\(build))"
    }
}
