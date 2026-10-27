import SwiftUI
import UIKit

/// The setup after creating an account or signing in, once per account, and again from Settings > Take the tour (the
/// Tour board; the Android app's `SetupScreen`): five pages, one idea each, always skippable. Every ask waits for you;
/// requests come to this phone (notifications, the screen lock); what the AIs can use (the services, a computer, an
/// AI app); how much to ask (Manual or Auto, the model, how a new AI starts); and where things stand. A service opens
/// over the setup and closes back to the same page.
struct OnboardingScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
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
            TourProgress(page: page.rawValue, count: Page.allCases.count, onSkip: page == .done ? nil : { model.finishOnboarding() })
            GeometryReader { geo in
                ScrollView {
                    VStack(spacing: 0) {
                        content
                            .frame(maxWidth: 480)
                            .padding(.top, 18)
                            .id(page)
                            .transition(reduceMotion
                                ? .opacity
                                : .asymmetric(insertion: .move(edge: .trailing), removal: .move(edge: .leading)).combined(with: .opacity))
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
        .animation(reduceMotion ? .easeInOut(duration: 0.15) : .smooth(duration: 0.3), value: page)
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

/// The page's buttons: Next alone on the welcome, Not now and Allow while notifications were never asked for, Start on
/// the last page, Back and Next between.
private struct SetupButtons: View {
    var page: OnboardingScreen.Page
    var onBack: () -> Void
    var onNext: () -> Void
    var onDone: () -> Void
    private var access: NotificationAccess { .shared }

    var body: some View {
        HStack(spacing: 10) {
            switch page {
            case .welcome:
                Button(action: onNext) {
                    Label { Text("Next") } icon: { GlyphIcon(.arrowForward, size: 18) }
                }
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
                Button("Start", action: onDone)
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
        .frame(maxWidth: 480)
        .padding(.horizontal, 16)
        .padding(.top, 10)
        .padding(.bottom, 8)
    }
}

/// A line of what is missing, in its colour, under a page's picture (a warning the setup must not hide).
private struct SetupNote: View {
    var glyph: Glyph
    var text: String
    var tint: Color = Palette.warning

    var body: some View {
        Label { Text(text) } icon: { GlyphIcon(glyph, size: 15) }
            .labelStyle(SignalLabelStyle())
            .font(RFont.sans(14))
            .foregroundStyle(tint)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 20)
    }
}

// MARK: 1. Welcome

/// Every ask waits for you, shown as the stack it will be: a push waiting, two more fanned behind it.
private struct WelcomePage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HeroText(title: "Welcome", line: "Every ask waits for you.")
            SkyTile(tint: Palette.fieldPush, height: 400) {
                ZStack {
                    fan(.read, angle: -7, x: -6, y: -10)
                    fan(.delete, angle: 6, x: 8, y: -4)
                    SampleCard()
                        .shadow(color: .black.opacity(0.3), radius: 20, y: 18)
                }
                .frame(width: 326)
            }
            .padding(.horizontal, 16)
            .padding(.top, 30)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setupWelcome")
    }

    private func fan(_ kind: FieldKind, angle: Double, x: CGFloat, y: CGFloat) -> some View {
        RoundedRectangle(cornerRadius: 26, style: .continuous)
            .fill(kind.fill)
            .overlay { Grain().clipShape(RoundedRectangle(cornerRadius: 26, style: .continuous)) }
            .frame(height: 200)
            .padding(.horizontal, 14)
            .rotationEffect(.degrees(angle))
            .offset(x: x, y: y)
            .shadow(color: .black.opacity(0.3), radius: 15, y: 16)
            .accessibilityHidden(true)
    }
}

/// A sample waiting card (not a real request): Push 8 commits to octo/app, Autopilot's pick and Review.
private struct SampleCard: View {
    var body: some View {
        WaitCardShell(
            kind: .push, label: "GitHub · Push",
            ask: AskText("Push 8 commits to", target: "octo/app", role: .cardAsk)
        ) {
            RequestIcon(spec: AskIcon(mark: .service("github"), action: .gitBranch, kind: .push), size: 36, ring: FieldKind.push.fill)
        } timer: {
            HStack(spacing: 8) {
                BlobAvatar(seed: "M2A", size: 28)
                ArcRing(fraction: 0.8, color: .white, track: .white.opacity(0.35), lineWidth: 2.5).frame(width: 18, height: 18)
                Text(verbatim: "9:40").font(RFont.mono(14, .medium)).foregroundStyle(.white)
            }
        } actions: {
            Text(verbatim: "feature/passkeys").font(RFont.mono(14)).foregroundStyle(.white.opacity(0.85)).padding(.top, -6)
            WaitCardButtons(pick: .allow(97), onPick: {}, onReview: {})
                .allowsHitTesting(false)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("A sample request: push 8 commits to octo/app")
    }
}

// MARK: 2. Requests come to you

/// Asks reach this phone first: a lock screen with one notification on a grained sky. What is missing (notifications
/// refused, no passcode) says so under it.
private struct NotificationsPage: View {
    @Environment(AppModel.self) private var model
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.openURL) private var openURL
    @State private var screenLock = true
    private var access: NotificationAccess { .shared }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HeroText(title: "Requests", line: "Asks reach this phone first.")
            SkyTile(tint: Palette.fieldGrant, height: 380, gradient: LinearGradient(
                colors: [Color(red: 91 / 255, green: 67 / 255, blue: 232 / 255), Palette.fieldAccounts], startPoint: .topLeading, endPoint: .bottomTrailing
            )) {
                VStack(spacing: 22) {
                    VStack(spacing: 2) {
                        GlyphIcon(.lock, size: 18)
                        Text(verbatim: "9:41").font(RFont.fixedSans(76, .semibold)).tracking(-2.3)
                    }
                    .foregroundStyle(.white)
                    SampleNotification()
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 30)
            VStack(alignment: .leading, spacing: 10) {
                if access.state == .on {
                    SetupNote(glyph: .check, text: "Notifications are on", tint: Palette.success).accessibilityIdentifier("notificationsOn")
                } else if access.state.needsAttention && access.state != .notAsked {
                    SetupNote(glyph: .bell, text: "\(access.state.summary). Without them an AI waits for an answer that never comes.")
                    Button("Open Settings") {
                        Task { await access.turnOn(openURL: openURL) }
                    }
                    .buttonStyle(SignalButtonStyle(.outline, size: .small))
                    .padding(.horizontal, 20)
                    .accessibilityIdentifier("openNotificationSettings")
                }
                if screenLock {
                    Color.clear.frame(height: 0).accessibilityIdentifier("screenLockOn")
                } else {
                    SetupNote(glyph: .faceId, text: "No passcode. Approving needs one: Settings, Face ID & Passcode.")
                        .accessibilityIdentifier("screenLockOff")
                }
            }
            .padding(.top, 14)
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

/// The notification they will get, drawn as it shows on the lock screen.
private struct SampleNotification: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image("ReinsIcon")
                    .resizable()
                    .frame(width: 22, height: 22)
                    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    .accessibilityHidden(true)
                Text("Reins").font(RFont.fixedSans(13, .semibold)).foregroundStyle(Color(red: 95 / 255, green: 94 / 255, blue: 107 / 255))
                Spacer()
                Text("now").font(RFont.fixedSans(13)).foregroundStyle(Color(red: 107 / 255, green: 106 / 255, blue: 120 / 255))
            }
            HStack(spacing: 10) {
                RequestIcon(spec: AskIcon(mark: .service("gmail"), action: .send, kind: .push), size: 36, ring: .white)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Claude wants to send an email").font(RFont.fixedSans(15.5, .semibold)).foregroundStyle(Palette.onFieldInk)
                    // Verbatim: a string literal is Markdown, which would turn the address into a link.
                    Text(verbatim: "anna@example.com").font(RFont.fixedMono(13)).foregroundStyle(Color(red: 95 / 255, green: 94 / 255, blue: 107 / 255))
                }
            }
        }
        .padding(14)
        .frame(width: 318)
        .background(.white.opacity(0.88), in: RoundedRectangle(cornerRadius: 24, style: .continuous))
        .shadow(color: .black.opacity(0.3), radius: 18, y: 18)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("A sample notification: Claude wants to send an email")
    }
}

// MARK: 3. What your AIs can use

/// The services as big tiles (a tile opens its service, then returns), a computer to scan, and the address for an AI
/// app: they are all connections.
private struct ConnectionsPage: View {
    var onOpen: (String) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    /// The connections there were when the page showed: a new one is the computer just connected.
    @State private var known: Set<String>?
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HeroText(title: "Connect", line: nil)
            if model.services.isEmpty {
                ProgressView().frame(maxWidth: .infinity).padding(24)
            } else {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
                    // The first six, as the board draws them; every service stays a tap away on Connections.
                    ForEach(model.services.prefix(6), id: \.service) { s in
                        TourServiceTile(id: s.service, name: s.name, count: s.accounts.count, available: s.available) { onOpen(s.service) }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 30)
            }
            RaisedGroup {
                HStack(spacing: 12) {
                    GlyphTile(.laptop)
                    Text("Computer").textRole(.row).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
                    // The sheet's Open cue is the sound (the style plays the default tap).
                    Button {
                        model.openSheet(.connectComputer)
                    } label: {
                        Label { Text("Scan") } icon: { GlyphIcon(.qrCode, size: 18) }
                    }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false, height: 40))
                    .disabled(!model.approvalDevice)
                    .accessibilityLabel("Scan QR code")
                    .accessibilityIdentifier("scanQR")
                }
                .padding(.leading, 14)
                .padding(.trailing, 8)
                .frame(minHeight: 56)
                HStack(spacing: 12) {
                    GlyphTile(.globe)
                    Text(verbatim: SettingsText.host(model.mcpAddress))
                        .font(RFont.mono(13.5))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                        .environment(\.layoutDirection, .leftToRight)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel(model.mcpAddress)
                        .accessibilityIdentifier("mcpAddress")
                    QuietIconButton(copied ? .check : .copy, label: copied ? "Copied" : "Copy") {
                        copyPlain(model.mcpAddress, feedback: feedback)
                        copied = true
                    }
                    .accessibilityIdentifier("copyMcp")
                }
                .padding(.leading, 14)
                .padding(.trailing, 8)
                .frame(minHeight: 56)
            }
            .padding(.top, 14)
            VStack(alignment: .leading, spacing: 8) {
                PhoneStatus()
                if let added {
                    SetupNote(glyph: .check, text: "\(untrustedName(added.label, limit: 40)) is connected.", tint: Palette.success)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("computerConnected")
                }
            }
            .padding(.top, 12)
            Link(destination: URL(string: "https://reins2fa.com/download")!) {
                Text("Get the desktop app")
                    .textRole(.sub)
                    .foregroundStyle(Palette.ink2)
                    .underline()
                    .frame(maxWidth: .infinity, minHeight: 44)
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

    private var added: ConnectionView? {
        guard let known else { return nil }
        return model.connections.last { !known.contains($0.id) }
    }
}

/// A service as a big tile: its logo, then a green check and how many accounts, or "Add".
private struct TourServiceTile: View {
    var id: String
    var name: String
    var count: Int
    var available: Bool
    var action: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        Button(action: action) {
            VStack(alignment: .leading, spacing: 0) {
                MarkPlate(size: 42) { ServiceAvatar(service: id, size: 42) }
                Spacer(minLength: 8)
                HStack(spacing: 4) {
                    if count > 0 {
                        GlyphIcon(.check, size: 14)
                        Text("\(count)")
                    } else if available {
                        GlyphIcon(.plus, size: 15)
                        Text("Add")
                    } else {
                        Text("Not here")
                    }
                }
                .font(RFont.sans(14, .semibold))
                .foregroundStyle(count > 0 ? Palette.success : Palette.ink2)
            }
            .padding(14)
            .frame(maxWidth: .infinity, minHeight: 108, alignment: .leading)
            .background(Palette.paperRaised, in: shape)
            .overlay { if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) } }
            .contentShape(shape)
        }
        .buttonStyle(PressDimsStyle())
        .disabled(!available)
        .accessibilityLabel("\(name), \(count > 0 ? (count == 1 ? "1 account" : "\(count) accounts") : available ? "add" : "not available")")
        .accessibilityIdentifier("setupService:\(id)")
    }
}

// MARK: 4. How much to ask

/// Manual or Auto as two big tiles, the model under them, and how a new AI starts.
private struct RulesPage: View {
    @Environment(AppModel.self) private var model
    @State private var ap = AutopilotModel()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HeroText(title: "Asking", line: nil)
            HStack(alignment: .top, spacing: 10) {
                ModeBigTile(mode: .manual, chosen: model.autopilot?.mode == .manual) { Task { await ap.setMode(.manual) } }
                ModeBigTile(mode: .auto, chosen: model.autopilot?.mode == .auto) { Task { await ap.setMode(.auto) } }
            }
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, 16)
            .padding(.top, 30)
            RaisedGroup { modelRow }
                .padding(.top, 12)
            SectionHeader("New AIs", glyph: .link, quiet: true).padding(.top, 2)
            FlatGroup {
                StartRow(title: "Ask every time", glyph: .shieldWaiting, policy: .askEveryTime)
                StartRow(title: "Headlines for 1 hour", glyph: .mail, policy: .headlinesFirstHour)
            }
            GroupFooter("A small model on this phone learns from your answers. Requests are judged right here; nothing is sent off to be decided. Change modes any time on the Trust tab.\n\nHeadlines for 1 hour: a new AI sees only sender, subject and date for its first hour; opening anything still asks.")
                .padding(.horizontal, 20)
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

    /// The model: its size and Get, or how its download goes.
    private var modelRow: some View {
        let m = ap.model
        return HStack(spacing: 12) {
            GlyphTile(.chip)
            Text("Model").textRole(.row).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
            if m?.state == .installed || ap.modelReady {
                Label { Text("Ready") } icon: { GlyphIcon(.check, size: 14) }
                    .labelStyle(SignalLabelStyle())
                    .font(RFont.sans(14, .semibold))
                    .foregroundStyle(Palette.success)
            } else if m?.state == .downloading, let m {
                let percent = m.sizeBytes > 0 ? Int(Double(m.downloadedBytes) / Double(m.sizeBytes) * 100) : 0
                Text("\(percent)%").font(RFont.sans(15)).foregroundStyle(Palette.ink2).monospacedDigit()
            } else if ap.downloadJob == .waiting {
                Text(ap.waitingForWifi ? "Waits for Wi-Fi" : "Waits for a network").font(RFont.sans(14)).foregroundStyle(Palette.ink2)
            } else {
                Label { Text("370 MB") } icon: { GlyphIcon(.cloud, size: 15) }
                    .labelStyle(SignalLabelStyle())
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.ink2)
                Button("Get") { ap.download() }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false, height: 40))
                    .accessibilityLabel("Get the model, about 370 MB, on Wi-Fi")
                    .accessibilityIdentifier("setupGetModel")
            }
        }
        .padding(.leading, 14)
        .padding(.trailing, 8)
        .frame(minHeight: 56)
    }
}

/// How a new AI starts: a row with a radio.
private struct StartRow: View {
    var title: String
    var glyph: Glyph
    var policy: StartingPolicy
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        let on = (model.startingPolicy ?? .askEveryTime) == policy
        Button {
            guard !on else { return }
            feedback.play(.selection)
            model.chooseStartingPolicy(policy)
        } label: {
            TileRow(title, glyph: glyph, chevron: false) { RadioDot(selected: on) }
                .padding(.horizontal, -14)
        }
        .buttonStyle(RowPressStyle())
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier("startingRule:\(policy == .askEveryTime ? "ask" : "headlines")")
    }
}

/// Manual (raised; a violet ring and a check once chosen) or Auto (its violet field with the grain) as a big tile.
private struct ModeBigTile: View {
    var mode: ModeKind
    var chosen: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 26, style: .continuous)
        let field = mode == .auto
        Button {
            guard !chosen else { return }
            feedback.play(.selection)
            action()
        } label: {
            VStack(alignment: .leading, spacing: 0) {
                RoundedRectangle(cornerRadius: 15, style: .continuous)
                    .fill(field ? .white.opacity(0.2) : Palette.paperSunken)
                    .frame(width: 48, height: 48)
                    .overlay { ModeGlyph(mode, size: 22).foregroundStyle(field ? .white : Palette.ink) }
                Spacer(minLength: 12)
                Text(mode.name)
                    .font(RFont.display(23.5))
                    .tracking(RFont.displayTracking(23.5))
                    .foregroundStyle(field ? .white : Palette.ink)
                Text(mode.effect)
                    .font(RFont.sans(13.5))
                    .foregroundStyle(field ? .white.opacity(0.85) : Palette.inkMeta)
                    .padding(.top, 4)
            }
            .padding(16)
            .frame(maxWidth: .infinity, minHeight: 176, alignment: .leading)
            .background {
                if field {
                    FieldSurface(kind: .grant).clipShape(shape)
                } else {
                    shape.fill(Palette.paperRaised)
                }
            }
            .overlay { if chosen { shape.strokeBorder(field ? .white : Palette.violet, lineWidth: 2.5) } }
            .overlay(alignment: .topTrailing) {
                if chosen {
                    GlyphIcon(.check, size: 14)
                        .foregroundStyle(field ? Palette.violetFill : .white)
                        .frame(width: 24, height: 24)
                        .background(field ? .white : Palette.violetFill, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .padding(14)
                }
            }
            .contentShape(shape)
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel("\(mode.name), \(mode.effect)")
        .accessibilityAddTraits(chosen ? .isSelected : [])
        .accessibilityIdentifier("setupMode:\(mode.rawValue)")
    }
}

// MARK: 5. All set

/// One big check, and what is ready as chips (a missing one shows a minus).
private struct DonePage: View {
    @Environment(AppModel.self) private var model
    private var access: NotificationAccess { .shared }
    @State private var screenLock = true

    var body: some View {
        let accounts = model.services.filter { $0.service != "vault" }.reduce(0) { $0 + $1.accounts.count }
        let modelState = model.autopilot?.model.state
        VStack(alignment: .leading, spacing: 0) {
            HeroText(title: "All set", line: nil)
            ZStack {
                Palette.success
                Grain()
                GlyphIcon(.check, size: 80).foregroundStyle(.white)
            }
                .clipShape(Circle())
                .frame(width: 168, height: 168)
                .shadow(color: Palette.success.opacity(0.5), radius: 25, y: 24)
                .frame(maxWidth: .infinity)
                .padding(.top, 40)
                .accessibilityHidden(true)
            CenteredFlow(spacing: 8) {
                SetupChip(glyph: .bell, text: "Notifications", ok: access.state == .on)
                SetupChip(glyph: .faceId, text: "Face ID", ok: screenLock)
                SetupChip(glyph: .database, text: accounts == 1 ? "1 service" : "\(accounts) services", ok: accounts > 0)
                SetupChip(glyph: .chip, text: "Model", ok: modelState == .installed)
                SetupChip(glyph: .laptop, text: model.connections.count == 1 ? "1 app" : "\(model.connections.count) apps", ok: !model.connections.isEmpty)
            }
            .padding(.horizontal, 20)
            .padding(.top, 34)
            if access.state != .on {
                SetupNote(glyph: .bell, text: "Turn notifications on from Activity, or asks wait unseen.").padding(.top, 16)
            }
            if !screenLock {
                SetupNote(glyph: .faceId, text: "Approving needs a passcode.").padding(.top, 8)
            }
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

/// Whether this phone became the approval device: still on it, done, or why not (with a retry).
private struct PhoneStatus: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false

    var body: some View {
        if model.approvalDevice {
            SetupNote(glyph: .shieldCheck, text: "This phone approves what your AIs ask.", tint: Palette.success)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("phoneReady")
        } else if let error = model.registrationError, !busy {
            VStack(alignment: .leading, spacing: 8) {
                FormBanner(text: error).padding(.horizontal, 16)
                Button("Try again", action: retry)
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                    .padding(.horizontal, 16)
                    .accessibilityIdentifier("registerAgain")
            }
        } else {
            HStack(spacing: 10) {
                ProgressView()
                Text("Setting up this phone for approvals").textRole(.sub).foregroundStyle(Palette.inkMeta)
            }
            .padding(.horizontal, 20)
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
