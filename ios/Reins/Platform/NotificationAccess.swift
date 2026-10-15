import SwiftUI
import UserNotifications

/// Whether approval requests can reach the user as notifications (the Android app's `NotificationAccess`). Without them
/// an AI waits for a phone that never rings while Reins is closed, so Activity and Settings keep saying so.
enum NotificationAccessState: Equatable {
    /// Not read yet.
    case unknown
    /// Never asked: the system prompt can still be shown.
    case notAsked
    /// Banners and sounds.
    case on
    /// Allowed, but delivered quietly (provisional, or alerts turned off): a request does not sound or show a banner.
    case quiet
    /// Refused: only the Settings app can turn them on.
    case off

    /// What the system says, in these terms.
    static func of(_ status: UNAuthorizationStatus, alert: UNNotificationSetting) -> Self {
        switch status {
        case .notDetermined: .notAsked
        case .denied: .off
        case .provisional: .quiet
        case .authorized, .ephemeral: alert == .disabled ? .quiet : .on
        @unknown default: .unknown
        }
    }

    /// The Settings row's line.
    var summary: String {
        switch self {
        case .unknown, .on: "On"
        case .notAsked: "Not turned on yet"
        case .quiet: "Quiet"
        case .off: "Off"
        }
    }

    /// Whether Activity says something about it.
    var needsAttention: Bool { self == .notAsked || self == .quiet || self == .off }
}

/// The current state, re-read whenever the app comes to the front (the user may have changed it in Settings).
@Observable
@MainActor
final class NotificationAccess {
    static let shared = NotificationAccess()

    private(set) var state = NotificationAccessState.unknown

    func refresh() async {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        state = .of(settings.authorizationStatus, alert: settings.alertSetting)
    }

    /// "Turn on": the system prompt while it can still be shown, else this app's page in the Settings app.
    func turnOn(openURL: OpenURLAction) async {
        if state == .notAsked {
            await NotificationRouter.shared.requestPermission()
            await refresh()
        } else if let url = URL(string: UIApplication.openNotificationSettingsURLString) {
            openURL(url)
        }
    }
}

/// Activity's card while requests cannot ring: what that means, and Turn on. Not in the demo, which asks only when told.
struct NotificationsOffCard: View {
    /// `-demoWarn` (debug): the warn field shows in the demo, for screenshots.
    static var demoWarn: Bool {
        #if DEBUG
        ProcessInfo.processInfo.arguments.contains("-demoWarn")
        #else
        false
        #endif
    }

    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    private var access: NotificationAccess { .shared }

    var body: some View {
        // A stack, not a Group, so the checks below run while nothing shows.
        VStack(spacing: 0) {
            if (access.state.needsAttention && !model.demo) || Self.demoWarn {
                CompactBanner(
                    text: access.state == .quiet ? "Notifications are quiet" : "Notifications are off",
                    symbol: "bell.slash.fill",
                    button: "Turn on",
                    label: "Reins · Notifications"
                ) { Task { await access.turnOn(openURL: openURL) } }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("notificationsOff")
            }
        }
        .task { await access.refresh() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await access.refresh() } }
        }
    }
}
