import Combine
import SwiftUI
import UIKit

/// Hides a sensitive screen (the recovery code, the unlock form, the vault's passkeys) whenever the app is not in
/// front, so the app switcher's snapshot does not keep it, and while the screen is recorded, mirrored or shared (the
/// Android app sets FLAG_SECURE on the same screens). A view modifier rather than one cover at the root: these screens
/// also show as sheets, which sit above the root.
struct PrivacyCover: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase
    @State private var captured = false

    func body(content: Content) -> some View {
        content
            .overlay {
                if scenePhase != .active || captured {
                    ZStack {
                        Palette.background
                        VStack(spacing: 14) {
                            BrandMark()
                            if captured {
                                Text("Hidden while the screen is recorded or shared.")
                                    .font(RFont.sans(15))
                                    .foregroundStyle(Palette.secondary)
                                    .multilineTextAlignment(.center)
                            }
                        }
                        .padding(32)
                    }
                    .ignoresSafeArea()
                    .accessibilityIdentifier("privacyCover")
                }
            }
            .onAppear { captured = Self.isCaptured }
            .onReceive(NotificationCenter.default.publisher(for: UIScreen.capturedDidChangeNotification)) { _ in
                captured = Self.isCaptured
            }
    }

    /// Any of the app's screens is being recorded, mirrored or shared.
    @MainActor static var isCaptured: Bool {
        UIApplication.shared.connectedScenes.contains { ($0 as? UIWindowScene)?.screen.isCaptured == true }
    }
}

extension View {
    /// See `PrivacyCover`.
    func privacyCover() -> some View { modifier(PrivacyCover()) }
}
