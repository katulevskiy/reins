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

/// The store is opening, or the session is being read. Someone signed in sees their main screen as it last was (the
/// widgets' snapshot: what waited, nothing secret) instead of a spinner; the live one replaces it within moments.
struct LoadingView: View {
    @State private var last: Snapshot?

    var body: some View {
        Group {
            if let last, last.signedIn {
                LaunchPreview(snapshot: last)
            } else {
                ProgressView()
                    .controlSize(.large)
                    .tint(Palette.accent)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .pageBackground()
        // The snapshot is sealed with a keychain key: read off the main thread.
        .task { last = await Task.detached(priority: .userInitiated) { Snapshot.load() }.value }
    }
}

/// The main screen's first moment, drawn from the last snapshot: the title and what was waiting, not yet tappable.
private struct LaunchPreview: View {
    var snapshot: Snapshot

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Activity").font(RFont.sans(34, .bold)).foregroundStyle(Palette.text).padding(.top, 12)
            let waiting = snapshot.waiting()
            if !waiting.isEmpty {
                SectionHeader("Waiting for you").padding(.top, 8)
                ForEach(waiting.prefix(6)) { item in
                    Text(item.title)
                        .font(RFont.sans(16, .semibold))
                        .foregroundStyle(Palette.text)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(14)
                        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                }
            }
            Spacer()
        }
        .padding(.horizontal, 16)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .allowsHitTesting(false)
        .accessibilityIdentifier("launchPreview")
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
