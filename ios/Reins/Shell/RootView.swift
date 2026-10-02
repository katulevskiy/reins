import SwiftUI

/// Signed out, signed in, or still opening the store.
struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            switch model.session {
            case .loading:
                ProgressView()
                    .controlSize(.large)
                    .tint(Palette.accent)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .pageBackground()
            case .signedOut:
                SignInScreen()
            case .signedIn:
                if model.onboarding { OnboardingScreen() } else { MainShell() }
            case .keysLocked, .otherApprovalDevice:
                UnlockScreen()
            }
        }
        .environment(\.feedback, model.feedback)
        // Links reach the model whatever shows: a pairing code opened while signed out waits for the sign-in.
        .onOpenURL { url in
            guard let link = DeepLink.opened(url) else { return }
            Task { await model.handle(link) }
        }
        // A universal link (a computer's QR code, https://<server>/pair?code=...): only its code is used.
        .onContinueUserActivity(NSUserActivityTypeBrowsingWeb) { activity in
            guard let url = activity.webpageURL, let link = DeepLink.opened(url) else { return }
            Task { await model.handle(link) }
        }
        #if DEBUG
        // `-gallery`: every avatar on one page (provider logos, service logos, blobatars).
        .overlay { if AvatarGallery.requested { AvatarGallery() } }
        #endif
        .tint(Palette.accent)
        .fontDesign(.default)
    }
}

/// The store could not be opened: nothing works without it, so say so plainly.
struct StartupFailureView: View {
    var message: String

    var body: some View {
        EmptyState(symbol: "exclamationmark.lock", title: "Reins could not start", message: message)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
    }
}
