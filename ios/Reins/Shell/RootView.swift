import SwiftUI

/// Signed out, signed in, or still opening the store.
struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            switch model.session {
            case .loading:
                LoadingView()
            case .signedOut:
                SignInScreen()
            case .signedIn:
                if let error = model.recoveryLoadError {
                    VStack(spacing: 20) {
                        Text("Recovery setup").font(RFont.sans(26, .semibold))
                        Text(error).font(RFont.sans(16))
                        Button("Try again") { Task { await model.refreshSession() } }
                            .buttonStyle(CapsuleButtonStyle(kind: .primary))
                    }.padding(24).pageBackground()
                } else if model.passkeyOffer {
                    // Before the code: a passkey that opens the vault (or not); the code follows either way.
                    PasskeyOfferScreen()
                } else if let code = model.recoveryToRecord {
                    RecoveryCodeSheet(code: code, required: true, onDone: model.confirmRecoveryRecord)
                } else if model.onboarding { OnboardingScreen() } else { MainShell() }
            case .keysLocked, .otherApprovalDevice:
                UnlockScreen()
            }
        }
        .environment(\.feedback, model.feedback)
        #if DEBUG
        // `-gallery`: every avatar on one page (provider logos, service logos, blobatars).
        .overlay { if AvatarGallery.requested { AvatarGallery() } }
        #endif
        .tint(Palette.accent)
        .fontDesign(.default)
    }
}

/// The store is opening, or the session is being read.
struct LoadingView: View {
    var body: some View {
        ProgressView()
            .controlSize(.large)
            .tint(Palette.accent)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
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
