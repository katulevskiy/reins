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
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @State private var set = true

    var body: some View {
        Group {
            if !set && !model.demo {
                Banner(
                    "No passcode: approving needs this iPhone's passcode or Face ID, so every approval fails until you set one in Settings, Face ID & Passcode.",
                    kind: .warning
                )
                .accessibilityIdentifier("screenLockOff")
            }
        }
        .task { set = ScreenLock.isSet() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { set = ScreenLock.isSet() }
        }
    }
}
