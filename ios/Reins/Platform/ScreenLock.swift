import LocalAuthentication
import SwiftUI

/// Approving anything needs the iPhone's passcode (or Face ID / Touch ID with it): `Authenticator` fails closed
/// without one. The onboarding and Activity say so before the first approval does (the Android app's `ScreenLock`).
enum ScreenLock {
    static func isSet() -> Bool {
        LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil)
    }
}

/// "No passcode": shown until one is set, re-checked whenever the app comes back to the front. Not in the demo, whose
/// approvals need no passcode (and the simulator has none).
struct ScreenLockBanner: View {
    /// Around the banner when it shows; nothing takes space while it does not.
    var padding = EdgeInsets()
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @State private var set = true

    var body: some View {
        // A stack, not a Group: a Group hands its modifiers to its content, so with the banner hidden the check
        // below would never run and the banner could never appear.
        VStack(spacing: 0) {
            if !set && !model.demo {
                Banner(
                    "No passcode. Approving needs one: Settings, Face ID & Passcode.",
                    kind: .warning
                )
                .padding(padding)
                .accessibilityIdentifier("screenLockOff")
            }
        }
        .task { set = ScreenLock.isSet() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { set = ScreenLock.isSet() }
        }
    }
}
