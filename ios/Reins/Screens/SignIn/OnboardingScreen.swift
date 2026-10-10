import SwiftUI
import UIKit

/// The setup after creating an account or signing in, once per account, and again from Settings > Take the tour (the
/// Android app's `SetupScreen`): what Reins is and how a request travels, notifications, the integrations an AI can
/// use, how much a new AI may do, Autopilot, connecting a computer and an AI app, and a summary. Every page can be
/// skipped; "Skip setup" ends it. An integration opens over the setup and closes back to the same page.
struct OnboardingScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.feedback) private var feedback
    @State private var page = Page.welcome
    /// The integration opened from the integrations page.
    @State private var service: ServiceSheet?

    enum Page: Int, CaseIterable { case welcome, notifications, integrations, rules, autopilot, computer, ai, done }

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            SetupTopBar(page: page) { model.finishOnboarding() }
            GeometryReader { geo in
                ScrollView {
                    VStack(spacing: 0) {
                        content
                            .frame(maxWidth: 440)
                            .padding(.horizontal, sizeClass == .regular ? 32 : 24)
                            .padding(.top, 20)
                            .id(page)
                            .transition(.opacity)
                        Spacer(minLength: 24)
                    }
                    .frame(maxWidth: .infinity, minHeight: geo.size.height, alignment: .top)
                }
                .scrollBounceBehavior(.basedOnSize)
            }
            .sheet(item: $service) { s in
                NavigationStack {
                    s.screen
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Done") { service = nil }.accessibilityIdentifier("setupServiceDone")
                            }
                        }
                }
                .environment(self.model)
            }
            SetupButtons(page: page, onBack: { go(-1) }, onNext: { go(1) }, onDone: { model.finishOnboarding() })
        }
        .pageBackground()
        .animation(.smooth(duration: 0.25), value: page)
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

    @ViewBuilder private var content: some View {
        switch page {
        case .welcome: WelcomePage()
        case .notifications: NotificationsPage()
        case .integrations: IntegrationsPage { service = ServiceSheet(id: $0) }
        case .rules: RulesPage()
        case .autopilot: AutopilotPage()
        case .computer: ComputerStep()
        case .ai: AiStep()
        case .done: DonePage()
        }
    }

    private func go(_ step: Int) {
        guard let next = Page(rawValue: page.rawValue + step) else { return }
        feedback.play(.tap)
        page = next
    }
}

/// An integration's page over the setup: its accounts, or adding an MCP server.
private struct ServiceSheet: Identifiable {
    var id: String

    @ViewBuilder var screen: some View {
        if id == "mcp" { McpAddScreen() } else { AccountsScreen(serviceId: id) }
    }
}

/// A segment per page (filled up to this one), and "Skip setup".
private struct SetupTopBar: View {
    var page: OnboardingScreen.Page
    var onSkip: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            HStack(spacing: 5) {
                ForEach(OnboardingScreen.Page.allCases, id: \.rawValue) { p in
                    Capsule()
                        .fill(p.rawValue <= page.rawValue ? Palette.accent : Palette.controlFill)
                        .frame(height: 4)
                }
            }
            .accessibilityElement()
            .accessibilityLabel("Step \(page.rawValue + 1) of \(OnboardingScreen.Page.allCases.count)")
            .accessibilityIdentifier("setupProgress")
            if page != .done {
                Button("Skip setup", action: onSkip)
                    .font(RFont.sans(14.5, .medium))
                    .foregroundStyle(Palette.accent)
                    .accessibilityIdentifier("setupSkip")
            }
        }
        .padding(.horizontal, 24)
        .padding(.top, 12)
        .frame(minHeight: 36)
    }
}

/// Back and the page's forward button. On the notifications page, while they were never asked for, forward is
/// "Not now" beside "Allow" (which shows the system's question).
private struct SetupButtons: View {
    var page: OnboardingScreen.Page
    var onBack: () -> Void
    var onNext: () -> Void
    var onDone: () -> Void
    private var access: NotificationAccess { .shared }

    var body: some View {
        HStack(spacing: 12) {
            switch page {
            case .welcome:
                Button("Show me around", action: onNext)
                    .buttonStyle(CapsuleButtonStyle(kind: .primary))
                    .accessibilityIdentifier("onboardingNext")
            case .notifications where access.state == .notAsked:
                Button("Not now", action: onNext)
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                    .accessibilityIdentifier("onboardingNext")
                Button {
                    Task {
                        await NotificationRouter.shared.requestPermission()
                        onNext()
                    }
                } label: {
                    Label("Allow", systemImage: "bell")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .accent))
                .accessibilityIdentifier("allowNotifications")
            case .done:
                Button("Start using Reins", action: onDone)
                    .buttonStyle(CapsuleButtonStyle(kind: .primary))
                    .accessibilityIdentifier("onboardingDone")
            default:
                Button("Back", action: onBack)
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                    .accessibilityIdentifier("onboardingBack")
                Button("Next", action: onNext)
                    .buttonStyle(CapsuleButtonStyle(kind: .primary))
                    .accessibilityIdentifier("onboardingNext")
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 12)
    }
}

/// A page's place ("STEP 2 · INTEGRATIONS"), its tile, title and what it is about.
private struct StepHeader: View {
    var eyebrow: String
    var symbol: String
    var tint: Color
    var title: String
    var text: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            SectionHeader(eyebrow).padding(.horizontal, -4)
            Image(systemName: symbol)
                .font(.system(size: 24, weight: .semibold))
                .foregroundStyle(tint)
                .frame(width: 52, height: 52)
                .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .accessibilityHidden(true)
                .padding(.vertical, 4)
            Text(title)
                .font(RFont.sans(28, .semibold))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            if let text {
                Text(text)
                    .font(RFont.sans(15.5))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

// MARK: 1. Welcome

private struct WelcomePage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionHeader("Welcome to Reins").padding(.horizontal, -4)
            Text("Your AIs ask.\nYou decide.")
                .font(RFont.sans(34, .semibold))
                .foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            Text("Reins sits between your AI agents and your accounts. Whenever one wants to read, send or change something, this phone asks you first.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(spacing: 0) {
                Promise(symbol: "hand.raised", tint: Palette.accent, title: "Nothing happens behind your back",
                        text: "Every read, send or change waits for a tap, with exactly what the AI wants to do.")
                Promise(symbol: "key", tint: Palette.success, title: "Your keys stay on this phone",
                        text: "Access to your mail, chats and code is kept encrypted here. The server only passes requests along.")
                Promise(symbol: "clock", tint: Palette.search, title: "Trust on your terms",
                        text: "Grant a few minutes of access, or let Autopilot learn what you always allow.")
                Promise(symbol: "lock", tint: Palette.warning, title: "One tap to stop everything",
                        text: "Lockdown denies every request at once, until you lift it.")
            }
            .padding(.vertical, 4)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .padding(.top, 10)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupWelcome")
    }
}

private struct Promise: View {
    var symbol: String
    var tint: Color
    var title: String
    var text: String

    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            IconTile(symbol: symbol, tint: tint, size: 40)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
                Text(text).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(16)
    }
}

// MARK: 2. Notifications

private struct NotificationsPage: View {
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    private var access: NotificationAccess { .shared }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 1 · Notifications", symbol: "bell", tint: Palette.accent, title: "Requests come to you",
                       text: "When an AI asks for something, this phone rings, even with Reins closed. You approve or deny right from the notification.")
            // What one looks like.
            VStack(alignment: .leading, spacing: 4) {
                Text("Reins · now").font(RFont.sans(12.5, .medium)).foregroundStyle(Palette.secondary)
                Text("Claude wants to send an email").font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text)
                // Verbatim: a string literal is Markdown, which would turn the address into a link.
                Text(verbatim: "To anna@example.com · \"Draft for Friday\"").font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(1)
                // In the order the notification shows them.
                HStack(spacing: 8) {
                    StatusPill(text: "Deny", tint: Palette.danger)
                    StatusPill(text: "Approve", tint: Palette.success)
                }
                .padding(.top, 6)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .padding(.vertical, 8)
            if access.state == .on {
                Label("Notifications are on. You're all set here.", systemImage: "checkmark.circle.fill")
                    .font(RFont.sans(15, .medium))
                    .foregroundStyle(Palette.text)
                    .accessibilityIdentifier("notificationsOn")
            } else {
                Text(access.state.needsAttention && access.state != .notAsked
                    ? "\(access.state.summary). Turn them on in the Settings app, under Reins."
                    : "Without notifications an AI waits for an answer that never comes, and its request times out. Reins only notifies you about requests, grants and your own devices.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                // Refused or quiet: only the Settings app can change it, so a way there.
                if access.state.needsAttention && access.state != .notAsked {
                    Button {
                        Task { await access.turnOn(openURL: openURL) }
                    } label: {
                        Label("Open Settings", systemImage: "gear")
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary))
                    .accessibilityIdentifier("openNotificationSettings")
                }
            }
            // Approving needs the passcode: better found out here than at the first request.
            ScreenLockBanner(padding: EdgeInsets(top: 10, leading: 0, bottom: 0, trailing: 0))
        }
        .task { await access.refresh() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await access.refresh() } }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupNotifications")
    }
}

// MARK: 3. Integrations

private struct IntegrationsPage: View {
    var onOpen: (String) -> Void
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 2 · Integrations", symbol: "square.grid.2x2", tint: Palette.read, title: "What your AIs can use",
                       text: "Connect the accounts you want your AIs to work with. They can only reach them through this phone, one approved request at a time.")
            if model.services.isEmpty {
                ProgressView().frame(maxWidth: .infinity).padding(24)
            } else {
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 10), GridItem(.flexible(), spacing: 10)], spacing: 10) {
                    ForEach(model.services, id: \.service) { s in
                        let count = s.accounts.count
                        IntegrationTile(
                            id: s.service, name: s.name,
                            status: !s.available ? "Not available" : count == 0 ? "Tap to connect" : count == 1 ? "1 account" : "\(count) accounts",
                            connected: count > 0, available: s.available
                        ) { onOpen(s.service) }
                    }
                    IntegrationTile(id: "mcp", name: "Any MCP server", status: "Add your own tools", connected: false, available: true) {
                        onOpen("mcp")
                    }
                }
                .padding(.top, 8)
            }
            Text("Nothing to connect right now? Add integrations any time from Integrations on the Activity tab.")
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.tertiary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4)
        }
        .task { await model.refreshPending() }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupIntegrations")
    }
}

private struct IntegrationTile: View {
    var id: String
    var name: String
    var status: String
    var connected: Bool
    var available: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top) {
                    ServiceAvatar(service: id, size: 40)
                    Spacer()
                    if connected {
                        Image(systemName: "checkmark.circle.fill").foregroundStyle(Palette.success).accessibilityHidden(true)
                    }
                }
                Text(name).font(RFont.sans(15, .semibold)).foregroundStyle(available ? Palette.text : Palette.tertiary).lineLimit(1)
                Text(status).font(RFont.sans(12.5)).foregroundStyle(connected ? Palette.success : Palette.tertiary).lineLimit(1)
            }
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(!available)
        .accessibilityIdentifier("setupService:\(id)")
    }
}

// MARK: 4. How much to ask

private struct RulesPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 3 · How much to ask", symbol: "checkmark.shield", tint: Palette.accent, title: "Fewer questions, same control",
                       text: "Reading is where most requests come from. Pick how a new AI starts; you can change it any time under Grants.")
            // The card brings its own side margin; the page's other content already has one.
            StartingRuleChooser().padding(.horizontal, -16).padding(.top, 8)
        }
        // Someone who just installed Reins starts with the recommended rule; a choice made before stays.
        .task {
            if (try? await model.core.startingPolicy()) == nil { model.chooseStartingPolicy(.readsForADay) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupRules")
    }
}

// MARK: 5. Autopilot

private struct AutopilotPage: View {
    @Environment(AppModel.self) private var model

    /// Where a new account starts, or (the tour taken again) the mode it is in now.
    private var startLine: String {
        guard let mode = model.autopilot?.mode, mode != .manual else { return "You start in Manual." }
        return "Autopilot is in \(AutopilotText.name(mode)) now."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 4 · Autopilot", symbol: "cpu", tint: Palette.accent, title: "A private model that learns your rules",
                       text: "Optionally, a small model on this phone learns from your answers and suggests, or takes, the easy decisions. Requests are judged right here; nothing is sent off to be decided.")
            VStack(spacing: 0) {
                ForEach(AutopilotText.modes, id: \.self) { mode in
                    HStack(spacing: 14) {
                        IconTile(symbol: SettingsText.modeSymbol(mode), tint: SettingsText.modeTint(mode), size: 36)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(AutopilotText.name(mode)).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text)
                            Text(AutopilotText.line(mode)).font(RFont.sans(13)).foregroundStyle(Palette.secondary).lineLimit(2)
                        }
                        Spacer(minLength: 0)
                        if AutopilotText.needsModel(mode) { StatusPill(text: "Model", tint: Palette.accent) }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 11)
                }
            }
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .padding(.top, 8)
            Text("\(startLine) Change modes, and download the model (about 370 MB), any time from Autopilot on the Activity tab.")
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupAutopilot")
    }
}

// MARK: 6. Your computer

/// Connecting a computer, with whether this phone is ready to approve.
private struct ComputerStep: View {
    @Environment(AppModel.self) private var model
    /// The connections there were when the step showed: a new one is the computer just connected.
    @State private var known: Set<String>?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 5 · Your computer", symbol: "desktopcomputer", tint: Palette.pair, title: "Connect your computer")
            Text("Run `reins login` on your computer (or open the Reins desktop app) and scan the QR code it shows.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            PhoneStatus().padding(.vertical, 4)
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
        }
        // The connections the account had once they are read, not the empty list from before.
        .task {
            await model.refreshConnections()
            if known == nil { known = Set(model.connections.map(\.id)) }
        }
    }

    private var added: ConnectionView? {
        guard let known else { return nil }
        return model.connections.last { !known.contains($0.id) }
    }
}

// MARK: 7. Your AI app

/// "Connect Claude.ai or ChatGPT": the server's /mcp address to add as a custom connector.
private struct AiStep: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "Step 6 · Your AI app", symbol: "sparkles", tint: Palette.accent, title: "Connect Claude.ai or ChatGPT")
            Text("In Claude.ai or ChatGPT, add a custom connector with this address. This phone then asks you to approve the connection.")
                .font(RFont.sans(15.5))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 10) {
                Text(model.mcpAddress)
                    .font(RFont.mono(15))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("mcpAddress")
                Button(copied ? "Copied" : "Copy") {
                    UIPasteboard.general.string = model.mcpAddress
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
        }
    }
}

// MARK: 8. Done

private struct DonePage: View {
    @Environment(AppModel.self) private var model
    private var access: NotificationAccess { .shared }
    @State private var screenLock = true

    var body: some View {
        let accounts = model.services.filter { $0.service != "vault" }.reduce(0) { $0 + $1.accounts.count }
        let modelState = model.autopilot?.model.state
        VStack(alignment: .leading, spacing: 12) {
            StepHeader(eyebrow: "All set", symbol: "checkmark.seal", tint: Palette.success, title: "You're in control",
                       text: "Your AIs can now do real work for you, and nothing happens without your say. Here's where things stand:")
            VStack(spacing: 0) {
                Checklist(symbol: "bell", title: "Notifications",
                          text: access.state == .on ? "On" : "\(access.state.summary): turn them on from Activity", ok: access.state == .on)
                Checklist(symbol: "lock", title: "Screen lock",
                          text: screenLock ? "On: approvals ask for it" : "Off: approving needs a passcode", ok: screenLock)
                Checklist(symbol: "square.grid.2x2", title: "Integrations",
                          text: accounts == 0 ? "None yet: add them from Integrations" : accounts == 1 ? "1 account connected" : "\(accounts) accounts connected",
                          ok: accounts > 0)
                Checklist(symbol: "cpu", title: "Private model",
                          text: modelState == .installed ? "Installed on this phone" : modelState == .downloading ? "Downloading in the background" : "Not downloaded: Manual mode needs none",
                          ok: modelState == .installed)
                Checklist(symbol: "desktopcomputer", title: "Computers and AI apps",
                          text: model.connections.isEmpty ? "None yet: connect one from Settings" : "\(model.connections.count) connected",
                          ok: !model.connections.isEmpty)
            }
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .padding(.top, 8)
            Text("Everything here can be changed later in Settings, where you can also take this tour again.")
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .task {
            screenLock = ScreenLock.isSet() || model.demo
            await access.refresh()
            await model.refreshConnections()
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupDone")
    }
}

private struct Checklist: View {
    var symbol: String
    var title: String
    var text: String
    var ok: Bool

    var body: some View {
        HStack(spacing: 14) {
            IconTile(symbol: symbol, tint: ok ? Palette.success : Palette.tertiary, size: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text)
                Text(text).font(RFont.sans(13)).foregroundStyle(Palette.secondary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            Image(systemName: ok ? "checkmark.circle.fill" : "circle").foregroundStyle(ok ? Palette.success : Palette.tertiary).accessibilityHidden(true)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 11)
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
