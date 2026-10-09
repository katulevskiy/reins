import SwiftUI
import UIKit

/// What comes right after creating an account or signing in on the sign-in screen, once per account: this phone
/// becoming the approval device, then connecting a computer (scan the QR code it shows) and an AI on the web (the
/// server's /mcp address as a custom connector). "Done" opens the app.
struct OnboardingScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var step = Step.computer

    enum Step { case computer, ai }

    var body: some View {
        @Bindable var model = model
        GeometryReader { geo in
            ScrollView {
                VStack(spacing: 0) {
                    Spacer(minLength: sizeClass == .regular ? 40 : 24)
                    Group {
                        switch step {
                        case .computer: ComputerStep { step = .ai }
                        case .ai: AiStep { step = .computer }
                        }
                    }
                    .frame(maxWidth: 440)
                    .padding(.horizontal, sizeClass == .regular ? 32 : 24)
                    .transition(.opacity)
                    Spacer(minLength: 24)
                }
                .frame(maxWidth: .infinity, minHeight: geo.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: step)
        // The pairing a scanned code (or an opened link) stands for is answered right here.
        .sheet(item: $model.sheet) { target in
            SheetContent(target: target, regular: sizeClass == .regular)
        }
        .presentationFeedback(model.sheet != nil, opens: false)
        .overlay(alignment: .top) {
            if let notice = model.notice {
                Toast(text: notice) { model.notice = nil }
                    .padding(.top, 8)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.smooth, value: model.notice)
    }
}

/// "Next: connect your computer", with whether this phone is ready to approve.
private struct ComputerStep: View {
    var onNext: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// The connections there were when the step showed: a new one is the computer just connected.
    @State private var known: Set<String>?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(number: 1, symbol: "desktopcomputer", tint: Palette.pair, title: "Next: connect your computer")
            Text("Run `reins login` on your computer (or open the Reins desktop app) and scan the QR code it shows.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            PhoneStatus().padding(.vertical, 4)
            // Approving needs the passcode: better found out here than at the first request.
            ScreenLockBanner()
            if let added {
                FormBanner(text: "\(untrusted(added.label)) is connected.", kind: .info)
                    .accessibilityIdentifier("computerConnected")
            }
            // The sheet's Open cue is the sound (the style plays the default tap).
            Button {
                model.openSheet(.connectComputer)
            } label: {
                Label("Scan QR code", systemImage: "qrcode.viewfinder")
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .disabled(!model.approvalDevice)
            .opacity(model.approvalDevice ? 1 : 0.45)
            .accessibilityIdentifier("scanQR")
            Link(destination: URL(string: "https://reins2fa.com/download")!) {
                Text("No desktop app yet? Get it at reins2fa.com/download")
                    .font(RFont.sans(14.5, .medium))
                    .foregroundStyle(Palette.accent)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
            }
            .accessibilityIdentifier("downloadDesktop")
            // How a new AI starts (the computer included); someone who just installed Reins starts with the recommended rule.
            // The card brings its own side margin; the step's other content already has one.
            StartingRuleChooser().padding(.horizontal, -16).padding(.top, 8)
            Button("Next", action: onNext)
                .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                .padding(.top, 8)
                .accessibilityIdentifier("onboardingNext")
        }
        // The connections the account had once they are read, not the empty list from before.
        .task {
            if (try? await model.core.startingPolicy()) == nil { model.chooseStartingPolicy(.readsForADay) }
            await model.refreshConnections()
            if known == nil { known = Set(model.connections.map(\.id)) }
        }
    }

    private var added: ConnectionView? {
        guard let known else { return nil }
        return model.connections.last { !known.contains($0.id) }
    }
}

/// "Connect Claude.ai or ChatGPT": the server's /mcp address to add as a custom connector, and Done.
private struct AiStep: View {
    var onBack: () -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(number: 2, symbol: "sparkles", tint: Palette.accent, title: "Connect Claude.ai or ChatGPT")
            Text("In Claude.ai or ChatGPT, add a custom connector with this address. This phone then asks you to approve the connection.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) {
                Text(mcpAddress)
                    .font(RFont.mono(15))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("mcpAddress")
                Button(copied ? "Copied" : "Copy") {
                    UIPasteboard.general.string = mcpAddress
                    feedback.play(.copied)
                    copied = true
                }
                .buttonStyle(PillButtonStyle())
                .accessibilityIdentifier("copyMcp")
            }
            .padding(.leading, 14)
            .padding(.trailing, 6)
            .padding(.vertical, 6)
            .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            Button("Done") {
                feedback.play(.tap)
                model.finishOnboarding()
            }
            .buttonStyle(CapsuleButtonStyle(kind: .primary))
            .padding(.top, 14)
            .accessibilityIdentifier("onboardingDone")
            Button("Back", action: onBack)
                .font(RFont.sans(14.5, .medium))
                .foregroundStyle(Palette.secondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 6)
                .accessibilityIdentifier("onboardingBack")
        }
    }

    /// `<server>/mcp`, the address AI clients connect to.
    private var mcpAddress: String {
        guard case let .signedIn(info) = model.session else { return SignInState.defaultServer + "/mcp" }
        var server = info.serverUrl
        while server.hasSuffix("/") { server.removeLast() }
        return server + "/mcp"
    }
}

/// A step's tile, its place ("STEP 1 OF 2") and its title.
private struct StepHeader: View {
    var number: Int
    var symbol: String
    var tint: Color
    var title: String

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(tint)
                .frame(width: 52, height: 52)
                .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .accessibilityHidden(true)
                .padding(.bottom, 4)
            SectionHeader("Step \(number) of 2").padding(.horizontal, -4)
            Text(title)
                .font(RFont.sans(28, .semibold))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
        }
    }
}

/// Whether this phone became the approval device: still on it, done, or why not (with a retry).
private struct PhoneStatus: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false

    var body: some View {
        if model.approvalDevice {
            FormBanner(text: "This phone now approves what your AI assistants ask for.", kind: .info)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("phoneReady")
        } else if let error = model.registrationError, !busy {
            VStack(alignment: .leading, spacing: 8) {
                FormBanner(text: error)
                Button("Try again", action: retry)
                    .font(RFont.sans(15, .medium))
                    .foregroundStyle(Palette.accent)
                    .accessibilityIdentifier("registerAgain")
            }
        } else {
            HStack(spacing: 10) {
                ProgressView()
                Text("Setting up this phone for approvals...")
                    .font(RFont.sans(14.5))
                    .foregroundStyle(Palette.secondary)
            }
            .padding(.vertical, 6)
        }
    }

    private func retry() {
        busy = true
        feedback.play(.tap)
        Task {
            do {
                try await model.registerDevice(force: true)
            } catch {
                feedback.play(.error)
                model.registrationError = error.userMessage
            }
            busy = false
        }
    }
}
