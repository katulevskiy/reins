import SwiftUI

/// What Settings and a connection's page say about Autopilot and the other switches (pure, unit-tested). The
/// Autopilot screens have their own, fuller set; these are the few words these pages need.
enum SettingsText {
    /// The modes in the order the picker shows them, from the user deciding everything to Autopilot refusing everything.
    static let modes: [AutopilotMode] = [.manual, .auto, .overnight, .bypass, .lockdown]

    static func modeName(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "Manual"
        case .auto: "Auto"
        case .overnight: AutopilotText.overnightName
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    static func modeSymbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "person.fill.checkmark"
        case .auto: "bolt.fill"
        case .overnight: AutopilotText.overnightSymbol
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        }
    }

    static func modeTint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.kindRead
        case .auto: Palette.violet
        case .overnight: Palette.violet
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    /// The symbol beside the mode on the Autopilot row and an AI's page: Manual a person with a check, Auto a bolt, Overnight the moon, Bypass a
    /// filled bolt, Lockdown a lock.
    static func modeValueSymbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "person.fill.checkmark"
        case .auto: "bolt"
        case .overnight: AutopilotText.overnightSymbol
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        }
    }

    /// The Sounds & haptics row's value.
    static func soundsValue(_ s: FeedbackSettings) -> String {
        if !s.master { return "Off" }
        switch (s.soundsOn, s.hapticsOn) {
        case (true, true): return "On"
        case (true, false): return "Sounds only"
        case (false, true): return "Haptics only"
        case (false, false): return "Off"
        }
    }

    /// The Integrations row's value: how many accounts are connected.
    static func accountsValue(_ n: Int) -> String {
        switch n {
        case 0: "None"
        case 1: "1 account"
        default: "\(n) accounts"
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

    /// The Vault passkeys row's value: the count, or a word while there is none (`count` nil while unknown).
    static func passkeysValue(_ count: Int?) -> String {
        switch count {
        case nil: "Not set up"
        case 0: "None"
        case let n?: "\(n)"
        }
    }

    /// "Added 9 Oct 2026", in the phone's own date format.
    static func passkeyAdded(_ epochSeconds: Int64) -> String {
        "Added " + Date(timeIntervalSince1970: TimeInterval(epochSeconds)).formatted(date: .abbreviated, time: .omitted)
    }

    /// A connection's line in Settings: "claude.ai · 5 min ago" for an AI app, "computer · just now" for a computer,
    /// "never used" for either.
    static func connectionLine(_ c: ConnectionView, now: Int64 = nowSeconds()) -> String {
        guard let used = c.lastUsedAt else { return "never used" }
        let place = c.keyFingerprint != nil ? "computer" : untrusted(c.clientHost)
        return place + " · " + GrantText.relative(used, now: now)
    }

    /// The line under a connection's name on its page: "claude.ai · used 5 min ago"; a computer shows the key it
    /// paired with, which says more than the address the server saw ("Key 4821 9930 · used just now").
    static func connectionHeadline(_ c: ConnectionView, now: Int64 = nowSeconds()) -> String {
        let used = c.lastUsedAt.map { "used \(GrantText.relative($0, now: now))" } ?? "never used"
        if let key = c.keyFingerprint { return "Key \(key) · " + used }
        return untrusted(c.clientHost) + " · " + used
    }

    /// The quiet line under a renamed connection's name, or a numbered one's ("Claude 2"): what it calls itself,
    /// quoted ("Calls itself “Claude”"), so that a name never hides what it is. Nil while it goes by its own name.
    static func calledItself(_ c: ConnectionView) -> String? {
        guard c.name != nil || c.number != nil else { return nil }
        let own = untrustedName(c.clientName.isEmpty ? c.ownLabel : c.clientName, limit: 48)
        return own.isEmpty ? nil : "Calls itself “\(own)”"
    }

    /// "Used 5 min ago", "Never used": the Last used row.
    static func lastUsed(_ c: ConnectionView, now: Int64 = nowSeconds()) -> String {
        c.lastUsedAt.map { GrantText.relative($0, now: now) } ?? "Never"
    }

    /// How the account card shows notifications.
    enum PushChip: Equatable { case on, onlyWhileOpen, hidden }

    /// The account card's notification chip. Only the approval phone gets requests; with notifications off the card
    /// says so instead (and offers to turn them on). Without push in this app (`appPush`) or on the server
    /// (`serverPush` false; nil: it does not say), requests arrive only while Reins is open.
    static func pushChip(approvalDevice: Bool, notificationsOn: Bool, appPush: Bool, serverPush: Bool?) -> PushChip {
        if !approvalDevice || !notificationsOn { return .hidden }
        if !appPush || serverPush == false { return .onlyWhileOpen }
        return .on
    }

    /// What the account card's yellow row says when notifications need a look.
    static func notificationsAttention(_ state: NotificationAccessState) -> String {
        state == .quiet ? "Notifications are quiet" : "Notifications are off"
    }

    /// "42 min left" (the last minute counts as one), "5 h left", "3 days left", "until you stop it".
    static func minutesLeft(_ until: Int64, now: Int64) -> String { AutopilotText.timeLeft(until: until, now: now) }

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

    /// What "Delete account" deletes, line by line, before it asks for the email.
    static let deletionItems = [
        "Account and vault",
        "Connections and access",
        "Permissions and activity",
        "Sign-in",
        "Data on this phone",
    ]

    /// Whether the typed text names the account's email (case and surrounding spaces do not count), as the server
    /// checks it.
    static func deletionConfirmed(_ typed: String, email: String) -> Bool {
        let typed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        return !typed.isEmpty && typed.lowercased() == email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    }

    /// The legal pages sign-in links to.
    static let termsURL = URL(string: "https://reins2fa.com/terms")!
    static let privacyURL = URL(string: "https://reins2fa.com/privacy")!

    /// "Reins 1.4 · Terms · Privacy", the last two links.
    static func footer(_ info: [String: Any]? = Bundle.main.infoDictionary) -> AttributedString {
        var text = AttributedString("Reins \(shortVersion(info))  ·  ")
        var terms = AttributedString("Terms")
        terms.link = termsURL
        var privacy = AttributedString("Privacy")
        privacy.link = privacyURL
        text.append(terms)
        text.append(AttributedString("  ·  "))
        text.append(privacy)
        return text
    }

    /// A server or MCP address without its scheme or trailing slash, as Settings shows it ("app.reins2fa.com").
    static func host(_ url: String) -> String {
        var s = url
        if let r = s.range(of: "://") { s = String(s[r.upperBound...]) }
        while s.hasSuffix("/") { s.removeLast() }
        return s
    }

    /// "Sep 28": the day a connection was made, as its page shows it.
    static func shortDate(_ epochSeconds: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(epochSeconds)).formatted(.dateTime.month(.abbreviated).day())
    }

    /// The name of an icon choice: Auto, Blobatar, the platform's ("macOS") or the provider's.
    static func iconName(_ pick: String?) -> String {
        guard let pick else { return "Auto" }
        if pick == Providers.blob { return "Blobatar" }
        return Platforms.byKey(pick)?.name ?? Providers.byKey(pick)?.name ?? "Auto"
    }

    /// "now", "5m", "1h", "3d", then the date: how long ago a computer was used, as the Devices page lists it.
    static func compactAgo(_ at: Int64, now: Int64 = nowSeconds()) -> String {
        let s = max(0, now - at)
        switch s {
        case ..<60: return "now"
        case ..<3_600: return "\(s / 60)m"
        case ..<86_400: return "\(s / 3_600)h"
        case ..<(7 * 86_400): return "\(s / 86_400)d"
        default: return Date(timeIntervalSince1970: TimeInterval(at)).formatted(date: .abbreviated, time: .omitted)
        }
    }

    /// "1.4": the version of this app, for the footer.
    static func shortVersion(_ info: [String: Any]? = Bundle.main.infoDictionary) -> String {
        info?["CFBundleShortVersionString"] as? String ?? "?"
    }

    /// "1.4 (27)": the version and build of this app.
    static func version(_ info: [String: Any]? = Bundle.main.infoDictionary) -> String {
        let version = info?["CFBundleShortVersionString"] as? String ?? "?"
        let build = info?["CFBundleVersion"] as? String ?? "?"
        return "\(version) (\(build))"
    }
}
