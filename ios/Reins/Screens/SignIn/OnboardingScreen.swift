import SwiftUI
import UIKit

/// The setup after creating an account or signing in, once per account, and again from Settings > Take the tour
/// (iOS-Tour.png; the Android app's `SetupScreen`): five pages, always skippable. What Reins is; requests coming to this
/// phone (notifications, the screen lock); what the AIs can use (the services, a computer, an AI app); how much to ask
/// (how a new AI starts, Autopilot and its model); and where things stand. An integration opens over the setup and
/// closes back to the same page.
struct OnboardingScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.feedback) private var feedback
    @State private var page = Page.first
    /// The integration opened from the connections page.
    @State private var service: ServiceSheet?

    enum Page: Int, CaseIterable {
        case welcome, notifications, connections, rules, done

        /// The welcome; in a debug build, `-tourPage <n>` opens another (screenshots).
        static var first: Page {
            #if DEBUG
            Page(rawValue: UserDefaults.standard.integer(forKey: "tourPage")) ?? .welcome
            #else
            .welcome
            #endif
        }
    }

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
                            .transition(.asymmetric(insertion: .move(edge: .trailing), removal: .move(edge: .leading)).combined(with: .opacity))
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
        .animation(.smooth(duration: 0.3), value: page)
        // Swipe sideways between pages, as well as the buttons. Only a sideways movement begins it (SidewaysPan), so the
        // page under it scrolls from the first points of a touch.
        .gesture(SidewaysPan { drag, cancelled in
            guard !cancelled, abs(drag.translation.width) > abs(drag.translation.height) * 1.5, abs(drag.translation.width) > 60 else { return }
            go(drag.translation.width < 0 ? 1 : -1)
        })
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
        case .connections: ConnectionsPage { service = ServiceSheet(id: $0) }
        case .rules: RulesPage()
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

/// A bar per page, each filled as you arrive, and "Skip" (the tour lives on under Settings).
private struct SetupTopBar: View {
    var page: OnboardingScreen.Page
    var onSkip: () -> Void

    var body: some View {
        HStack(spacing: 14) {
            HStack(spacing: 5) {
                ForEach(OnboardingScreen.Page.allCases, id: \.rawValue) { p in
                    Capsule()
                        .fill(p.rawValue <= page.rawValue ? Palette.ink : Palette.rule)
                        .frame(height: 3)
                }
            }
            .animation(.easeOut(duration: 0.3), value: page)
            .accessibilityElement()
            .accessibilityLabel("Step \(page.rawValue + 1) of \(OnboardingScreen.Page.allCases.count)")
            .accessibilityIdentifier("setupProgress")
            if page != .done {
                Button("Skip", action: onSkip)
                    .font(TextRole.button.font())
                    .foregroundStyle(Palette.ink2)
                    .accessibilityLabel("Skip setup")
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
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("onboardingNext")
            case .notifications where access.state == .notAsked:
                Button("Not now", action: onNext)
                    .buttonStyle(SignalButtonStyle(.outline))
                    .accessibilityIdentifier("onboardingNext")
                Button("Allow") {
                    Task {
                        await NotificationRouter.shared.requestPermission()
                        onNext()
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .accessibilityIdentifier("allowNotifications")
            case .done:
                Button("Start using Reins", action: onDone)
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("onboardingDone")
            default:
                Button("Back", action: onBack)
                    .buttonStyle(SignalButtonStyle(.outline))
                    .accessibilityIdentifier("onboardingBack")
                Button("Next", action: onNext)
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .accessibilityIdentifier("onboardingNext")
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 12)
    }
}

/// A page's title in the serif (its last words in italic) and what it is about.
private struct PageTitle: View {
    var lead: String
    var target: String
    var text: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            AskText([AskText.Run(text: lead), AskText.Run(text: target, target: true)], role: .ask, lineLimit: nil)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
            if let text {
                Text(text)
                    .textRole(.body)
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// A row on a hairline: a glyph, a title over a meta line, and what is on its right.
private struct TourRow<Trailing: View>: View {
    var glyph: Glyph
    var title: String
    var text: String
    var tint: Color = Palette.ink2
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            GlyphIcon(glyph, size: 20)
                .foregroundStyle(tint)
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).textRole(.row).foregroundStyle(Palette.ink)
                Text(text).textRole(.meta).foregroundStyle(Palette.inkMeta).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            trailing
        }
        .padding(.vertical, 13)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
    }
}

/// A small eyebrow over a group of rows.
private struct Eyebrow: View {
    var text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text.uppercased())
            .textRole(.label)
            .foregroundStyle(Palette.inkMeta)
            .padding(.top, 14)
            .accessibilityAddTraits(.isHeader)
    }
}

// MARK: 1. Welcome

private struct WelcomePage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Eyebrow("Welcome to Reins").padding(.top, -14)
            PageTitle(lead: "Nothing happens ", target: "behind your back")
            VStack(spacing: 0) {
                TourRow(glyph: .shieldWaiting, title: "Every ask waits for you",
                        text: "Each read, send or change shows exactly what the AI wants.") { EmptyView() }
                TourRow(glyph: .key, title: "Your keys stay on this phone",
                        text: "The server passes requests along and sees nothing inside.") { EmptyView() }
                TourRow(glyph: .timer, title: "Trust on your terms",
                        text: "Allow for an hour, or let Autopilot learn what you always allow.") { EmptyView() }
                TourRow(glyph: .lock, title: "One tap stops everything",
                        text: "Lockdown denies every request at once, until you lift it.") { EmptyView() }
            }
            .padding(.top, 6)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupWelcome")
    }
}

// MARK: 2. Requests come to you

private struct NotificationsPage: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @State private var screenLock = true
    private var access: NotificationAccess { .shared }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            PageTitle(lead: "Requests come ", target: "to you",
                      text: "When an AI asks, this phone rings, even with Reins closed. Answer right from the notification.")
            SampleNotification().padding(.vertical, 8)
            VStack(spacing: 0) {
                TourRow(glyph: .bell, title: "Notifications", text: "Requests, grants and your devices only") {
                    if access.state == .on {
                        StatusWord(text: "On", ok: true).accessibilityIdentifier("notificationsOn")
                    } else {
                        StatusWord(text: access.state == .notAsked ? "Not yet" : "Off", ok: false)
                    }
                }
                TourRow(glyph: .faceId, title: "Face ID", text: screenLock ? "Every approval asks for it" : "No passcode. Approving needs one: Settings, Face ID & Passcode.",
                        tint: screenLock ? Palette.ink2 : Palette.warning) {
                    StatusWord(text: screenLock ? "On" : "Off", ok: screenLock)
                }
                .accessibilityIdentifier(screenLock ? "screenLockOn" : "screenLockOff")
            }
            if access.state != .on {
                Text(access.state.needsAttention && access.state != .notAsked
                    ? "\(access.state.summary). Turn them on in the Settings app, under Reins."
                    : "Without notifications an AI waits for an answer that never comes, and its request times out.")
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
                    .fixedSize(horizontal: false, vertical: true)
                // Refused or quiet: only the Settings app can change it, so a way there.
                if access.state.needsAttention && access.state != .notAsked {
                    Button("Open Settings") {
                        Task { await access.turnOn(openURL: openURL) }
                    }
                    .buttonStyle(SignalButtonStyle(.outline))
                    .accessibilityIdentifier("openNotificationSettings")
                }
            }
        }
        .task {
            screenLock = ScreenLock.isSet() || model.demo
            await access.refresh()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                screenLock = ScreenLock.isSet() || model.demo
                Task { await access.refresh() }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupNotifications")
    }
}

/// "On" (a check) or what is missing, on the right of a row.
private struct StatusWord: View {
    var text: String
    var ok: Bool

    var body: some View {
        HStack(spacing: 5) {
            if ok { GlyphIcon(.check, size: 14) }
            Text(text)
        }
        .font(TextRole.tag.font())
        .foregroundStyle(ok ? Palette.success : Palette.inkMeta)
        .fixedSize()
    }
}

/// The notification they will get, drawn as it shows on the lock screen.
private struct SampleNotification: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                GlyphIcon(.shieldCheck, size: 13)
                    .foregroundStyle(Palette.onInverse)
                    .frame(width: 20, height: 20)
                    .background(Palette.inverse, in: RoundedRectangle(cornerRadius: 5, style: .continuous))
                    .accessibilityHidden(true)
                Text("Reins").font(RFont.sans(13, .semibold)).foregroundStyle(Palette.ink2)
                Spacer()
                Text("now").font(RFont.sans(13)).foregroundStyle(Palette.inkMeta)
            }
            Text("Claude wants to send an email").font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.ink).padding(.top, 4)
            // Verbatim: a string literal is Markdown, which would turn the address into a link.
            Text(verbatim: "To anna@example.com · \u{201C}Draft for Friday\u{201D}").font(RFont.sans(14)).foregroundStyle(Palette.ink2).lineLimit(1)
            // In the order the notification shows them.
            HStack(spacing: 8) {
                Text("Deny").frame(maxWidth: .infinity)
                Text("Review").frame(maxWidth: .infinity)
            }
            .font(RFont.sans(14, .medium))
            .foregroundStyle(Palette.ink)
            .frame(height: 34)
            .background(Palette.paperSunken, in: Capsule())
            .padding(.top, 8)
        }
        .padding(14)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
        .shadow(color: .black.opacity(0.06), radius: 12, y: 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("A sample notification: Claude wants to send an email")
    }
}

// MARK: 3. What your AIs can use

/// The services, a computer and an AI app together: they are all connections.
private struct ConnectionsPage: View {
    var onOpen: (String) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// The connections there were when the page showed: a new one is the computer just connected.
    @State private var known: Set<String>?
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            PageTitle(lead: "What your AIs ", target: "can use",
                      text: "They reach these only through this phone, one approved request at a time.")
            if model.services.isEmpty {
                ProgressView().frame(maxWidth: .infinity).padding(24)
            } else {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 8) {
                    ForEach(model.services, id: \.service) { s in
                        let count = s.accounts.count
                        TourServiceTile(
                            id: s.service, name: s.name,
                            status: !s.available ? "Not available" : count == 0 ? "Connect" : count == 1 ? "1 account" : "\(count) accounts",
                            connected: count > 0, available: s.available
                        ) { onOpen(s.service) }
                    }
                    TourServiceTile(id: "mcp", name: "Any MCP", status: "Add yours", connected: false, available: true) {
                        onOpen("mcp")
                    }
                }
                .padding(.top, 8)
            }

            VStack(spacing: 0) {
                // The sheet's Open cue is the sound (the style plays the default tap).
                TourRow(glyph: .laptop, title: "Your computer", text: "Desktop app or reins login") {
                    Button("Scan") { model.openSheet(.connectComputer) }
                        .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                        .disabled(!model.approvalDevice)
                        .opacity(model.approvalDevice ? 1 : 0.45)
                        .accessibilityLabel("Scan QR code")
                        .accessibilityIdentifier("scanQR")
                }
                aiApp
            }
            .padding(.top, 10)
            PhoneStatus()
            if let added {
                FormBanner(text: "\(untrusted(added.label)) is connected.", kind: .info)
                    .accessibilityIdentifier("computerConnected")
            }
            Link(destination: URL(string: "https://reins2fa.com/download")!) {
                Text("No desktop app yet? Get it at reins2fa.com/download")
                    .textRole(.meta)
                    .foregroundStyle(Palette.ink2)
                    .underline()
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
            }
            .accessibilityIdentifier("downloadDesktop")
        }
        // The connections the account had once they are read, not the empty list from before.
        .task {
            await model.refreshPending()
            await model.refreshConnections()
            if known == nil { known = Set(model.connections.map(\.id)) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupIntegrations")
    }

    /// "Claude.ai or ChatGPT": the server's /mcp address to add as a custom connector, and Copy.
    private var aiApp: some View {
        HStack(alignment: .center, spacing: 14) {
            GlyphIcon(.globe, size: 20)
                .foregroundStyle(Palette.ink2)
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("Claude.ai or ChatGPT").textRole(.row).foregroundStyle(Palette.ink)
                Text(model.mcpAddress)
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("mcpAddress")
            }
            Spacer(minLength: 8)
            Button(copied ? "Copied" : "Copy") {
                UIPasteboard.general.string = model.mcpAddress
                feedback.play(.copied)
                copied = true
            }
            .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
            .accessibilityIdentifier("copyMcp")
        }
        .padding(.vertical, 13)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
    }

    private var added: ConnectionView? {
        guard let known else { return nil }
        return model.connections.last { !known.contains($0.id) }
    }
}

/// A service in the grid: its mark, its name, and "Connect" or its accounts (ticked once one lands).
private struct TourServiceTile: View {
    var id: String
    var name: String
    var status: String
    var connected: Bool
    var available: Bool
    var action: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top) {
                    ServiceAvatar(service: id, size: 30)
                    Spacer(minLength: 0)
                    if connected {
                        GlyphIcon(.check, size: 14).foregroundStyle(Palette.success).accessibilityHidden(true)
                    }
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(name).font(RFont.sans(14, .medium)).foregroundStyle(available ? Palette.ink : Palette.inkMeta).lineLimit(1)
                    Text(status).font(RFont.sans(12)).foregroundStyle(connected ? Palette.success : Palette.inkMeta).lineLimit(1)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Palette.paperSunken, in: shape)
            .contentShape(shape)
        }
        .buttonStyle(PressDimsStyle())
        .disabled(!available)
        .accessibilityIdentifier("setupService:\(id)")
    }
}

// MARK: 4. How much to ask

/// Two decisions, one page: how a new AI starts, and whether a model on this phone helps.
private struct RulesPage: View {
    @Environment(AppModel.self) private var model
    @State private var ap = AutopilotModel()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            PageTitle(lead: "How much ", target: "to ask")
            // The card brings its own side margin; the page's other content already has one.
            StartingRuleChooser().padding(.horizontal, -16).padding(.top, 8)
            Eyebrow("Autopilot · a private model")
            HStack(spacing: 8) {
                ModeTile(mode: .manual, line: "You decide, with a hint", chosen: model.autopilot?.mode == .manual) {
                    Task { await ap.setMode(.manual) }
                }
                ModeTile(mode: .auto, line: "Decides the easy ones", chosen: model.autopilot?.mode == .auto) {
                    Task { await ap.setMode(.auto) }
                }
            }
            modelRow
            Text("A small model on this phone learns from your answers. Requests are judged right here; nothing is sent off to be decided. Change modes any time on the Trust tab.")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
                .fixedSize(horizontal: false, vertical: true)
        }
        // Someone who just installed Reins starts with the recommended rule; a choice made before stays.
        .task {
            ap.bind(model)
            if (try? await model.core.startingPolicy()) == nil { model.chooseStartingPolicy(.askEveryTime) }
            await ap.refresh()
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupRules")
    }

    /// "Get the model now", or how its download goes.
    @ViewBuilder private var modelRow: some View {
        let m = ap.model
        if m?.state == .installed || ap.modelReady {
            TourRow(glyph: .chip, title: "The model", text: "On this phone") { StatusWord(text: "Ready", ok: true) }
        } else if m?.state == .downloading, let m {
            let percent = m.sizeBytes > 0 ? Int(Double(m.downloadedBytes) / Double(m.sizeBytes) * 100) : 0
            TourRow(glyph: .chip, title: "The model", text: "Downloading in the background · \(percent)%") { EmptyView() }
        } else if ap.downloadJob == .waiting {
            TourRow(glyph: .chip, title: "The model", text: ap.waitingForWifi ? "Waits for Wi-Fi" : "Waits for a network") { EmptyView() }
        } else {
            TourRow(glyph: .chip, title: "Get the model now", text: "About 370 MB, on Wi-Fi, runs on this phone") {
                Button("Get") { ap.download() }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                    .accessibilityIdentifier("setupGetModel")
            }
        }
    }
}

/// Manual or Auto as a tile, its name in the serif as on the Trust tab.
private struct ModeTile: View {
    var mode: AutopilotMode
    var line: String
    var chosen: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)
        Button {
            guard !chosen else { return }
            feedback.play(.selection)
            action()
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                Text(AutopilotText.name(mode)).font(RFont.serif(28)).foregroundStyle(Palette.ink)
                Text(line).textRole(.meta).foregroundStyle(Palette.ink2).fixedSize(horizontal: false, vertical: true)
            }
            .padding(14)
            .frame(maxWidth: .infinity, minHeight: 96, alignment: .topLeading)
            .background(chosen ? Palette.paperRaised : Palette.paperSunken, in: shape)
            .overlay(shape.strokeBorder(chosen ? Palette.ink : Color.clear, lineWidth: 1.5))
            .contentShape(shape)
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityAddTraits(chosen ? .isSelected : [])
        .accessibilityIdentifier("setupMode:\(AutopilotText.key(mode))")
    }
}

// MARK: 5. You're in control

private struct DonePage: View {
    @Environment(AppModel.self) private var model
    private var access: NotificationAccess { .shared }
    @State private var screenLock = true

    var body: some View {
        let accounts = model.services.filter { $0.service != "vault" }.reduce(0) { $0 + $1.accounts.count }
        let modelState = model.autopilot?.model.state
        VStack(alignment: .leading, spacing: 12) {
            PageTitle(lead: "You're ", target: "in control",
                      text: "Your AIs can now do real work for you, and nothing happens without your say.")
            VStack(spacing: 0) {
                Checklist(glyph: .bell, title: "Notifications",
                          text: access.state == .on ? "On" : "\(access.state.summary): turn them on from Activity", ok: access.state == .on)
                Checklist(glyph: .faceId, title: "Screen lock",
                          text: screenLock ? "On: approvals ask for it" : "Off: approving needs a passcode", ok: screenLock)
                Checklist(glyph: .database, title: "Services",
                          text: accounts == 0 ? "None yet: add them from Connections" : accounts == 1 ? "1 account connected" : "\(accounts) accounts connected",
                          ok: accounts > 0)
                Checklist(glyph: .chip, title: "Private model",
                          text: modelState == .installed ? "Installed on this phone" : modelState == .downloading ? "Downloading in the background" : "Not downloaded: Manual needs none",
                          ok: modelState == .installed)
                Checklist(glyph: .laptop, title: "Computers and AI apps",
                          text: model.connections.isEmpty ? "None yet: connect one from Connections" : "\(model.connections.count) connected",
                          ok: !model.connections.isEmpty)
            }
            .padding(.top, 6)
            Text("Everything here can be changed later. The tour lives on under Settings.")
                .textRole(.meta)
                .foregroundStyle(Palette.inkMeta)
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
    var glyph: Glyph
    var title: String
    var text: String
    var ok: Bool

    var body: some View {
        TourRow(glyph: glyph, title: title, text: text) {
            GlyphIcon(ok ? .circleCheck : .minus, size: 18)
                .foregroundStyle(ok ? Palette.success : Palette.inkMeta)
                .accessibilityHidden(true)
        }
        .accessibilityElement(children: .combine)
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
                    .font(TextRole.button.font())
                    .foregroundStyle(Palette.ink)
                    .accessibilityIdentifier("registerAgain")
            }
        } else {
            HStack(spacing: 10) {
                ProgressView()
                Text("Setting up this phone for approvals")
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
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
