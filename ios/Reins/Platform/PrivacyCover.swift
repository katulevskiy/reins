import SwiftUI
import UIKit

/// Hides a sensitive screen (the recovery code, the unlock form, the vault's passkeys) whenever the app is not in
/// front, so the app switcher's snapshot does not keep it, and while the screen is recorded, mirrored or shared (the
/// Android app sets FLAG_SECURE on the same screens). The screen only says it is sensitive while it shows; the cover
/// itself is a window of its own above the app's (`PrivacyShield`), so nothing shows behind it: not the navigation
/// bar, the tab bar, or the sheet these screens may sit in.
struct PrivacyCover: ViewModifier {
    @State private var token = UUID()

    func body(content: Content) -> some View {
        content
            .onAppear { PrivacyShield.shared.add(token) }
            .onDisappear { PrivacyShield.shared.remove(token) }
    }
}

extension View {
    /// See `PrivacyCover`.
    func privacyCover() -> some View { modifier(PrivacyCover()) }
}

/// The cover over the whole window while a sensitive screen shows and the app is not in front or is captured: a
/// window at the alert level on the app's scene, with paper, the brand mark and, while captured, why.
@MainActor
final class PrivacyShield {
    static let shared = PrivacyShield()

    /// The sensitive screens on screen now.
    private var screens: Set<UUID> = []
    private var inactive = false
    private var window: UIWindow?
    private var observers: [NSObjectProtocol] = []

    private init() {
        let center = NotificationCenter.default
        // Before the app switcher takes its snapshot: shown at once, not on the next frame.
        observers = [
            center.addObserver(forName: UIScene.willDeactivateNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.setInactive(true) }
            },
            center.addObserver(forName: UIScene.didActivateNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.setInactive(false) }
            },
            center.addObserver(forName: UIScreen.capturedDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated { self?.update() }
            },
        ]
    }

    func add(_ screen: UUID) {
        screens.insert(screen)
        update()
    }

    func remove(_ screen: UUID) {
        screens.remove(screen)
        update()
    }

    private func setInactive(_ value: Bool) {
        inactive = value
        update()
    }

    /// Any of the app's screens is being recorded, mirrored or shared.
    static var isCaptured: Bool {
        UIApplication.shared.connectedScenes.contains { ($0 as? UIWindowScene)?.screen.isCaptured == true }
    }

    /// Whether the cover shows: a sensitive screen is up, and the app is not in front or is captured.
    nonisolated static func shows(screens: Int, inactive: Bool, captured: Bool, forced: Bool = false) -> Bool {
        screens > 0 && (inactive || captured || forced)
    }

    /// Debug builds can force the cover for screenshots (not shipped: always false).
    static var forced: Bool { false }

    private func update() {
        let captured = Self.isCaptured
        guard Self.shows(screens: screens.count, inactive: inactive, captured: captured, forced: Self.forced) else {
            window?.isHidden = true
            window = nil
            return
        }
        if let window {
            (window.rootViewController as? UIHostingController<PrivacyCoverView>)?.rootView = PrivacyCoverView(captured: captured)
            return
        }
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene })
            .first(where: { $0.activationState != .unattached }) else { return }
        let cover = UIWindow(windowScene: scene)
        cover.windowLevel = .alert + 1
        cover.overrideUserInterfaceStyle = Self.style
        let host = UIHostingController(rootView: PrivacyCoverView(captured: captured))
        host.view.backgroundColor = .clear
        cover.rootViewController = host
        cover.isHidden = false
        window = cover
    }

    /// The app's theme (Settings > Appearance), so the cover is the same paper as the screen it hides.
    private static var style: UIUserInterfaceStyle {
        switch Appearance.Theme(rawValue: AppGroup.defaults.string(forKey: Appearance.themeKey) ?? "")?.scheme {
        case .light: .light
        case .dark: .dark
        default: .unspecified
        }
    }
}

/// What the cover shows: paper edge to edge, the brand mark, and while the screen is captured, why.
struct PrivacyCoverView: View {
    var captured: Bool

    var body: some View {
        ZStack {
            Palette.paper
            VStack(spacing: 14) {
                BrandMark()
                if captured {
                    Text("Hidden while the screen is recorded or shared.")
                        .font(RFont.sans(15))
                        .foregroundStyle(Palette.ink2)
                        .multilineTextAlignment(.center)
                }
            }
            .padding(32)
        }
        .ignoresSafeArea()
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("privacyCover")
    }
}
