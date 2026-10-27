import SwiftUI

/// The signed-in app. A compact width (an iPhone, a folded iPhone Duo, iPad Slide Over) gets Signal's floating tab bar
/// with a navigation stack per tab; a regular width (iPad, an unfolded iPhone Duo) gets a sidebar, the section's
/// list and a detail column. Both read the same navigation state (`AppModel.section` / `paths`), so folding or
/// resizing keeps what is open.
struct MainShell: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var model = model
        HingeReader { posture in
            Group {
                if sizeClass == .regular {
                    SplitShell()
                } else {
                    TabShell()
                }
            }
            // A request is decided full screen; pairings, uploads and the rest are sheets.
            .fullScreenCover(item: Binding(
                get: { model.sheet?.isApproval == true ? model.sheet : nil },
                set: { if $0 == nil, model.sheet?.isApproval == true { model.closeSheet() } }
            )) { target in
                // Presented content does not inherit the presenter's environment reliably: hand the fold over.
                SheetContent(target: target, regular: sizeClass == .regular)
                    .environment(\.hinge, posture)
            }
            .sheet(item: Binding(
                get: { model.sheet?.isApproval == false ? model.sheet : nil },
                set: { if $0 == nil, model.sheet?.isApproval == false { model.closeSheet() } }
            )) { target in
                SheetContent(target: target, regular: sizeClass == .regular)
                    .environment(\.hinge, posture)
            }
            // Each item's sheet plays its own Open (`SheetContent`); the close is the shell's.
            .presentationFeedback(model.sheet != nil, opens: false)
        }
        .overlay(alignment: .top) {
            if let notice = model.notice {
                Toast(text: notice) { model.notice = nil }
                    .padding(.top, 8)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.smooth, value: model.notice)
        .onChange(of: scenePhase, initial: true) { _, phase in
            model.setActive(phase == .active)
        }
    }
}

// MARK: Compact

/// Four navigation stacks, one per place, under Signal's tab bar (an inverse capsule floating over the content, which
/// scrolls on beneath it), shown at a place's root and gone on the pages pushed over it, as the boards draw them. A
/// stack is made the first time its tab is chosen and kept after, so going back to a tab finds it as it was.
private struct TabShell: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var visited: Set<AppSection> = []
    /// The tab bar's measured height (an estimate until it has been laid out).
    @State private var barHeight: CGFloat = SignalTabBar<AppSection>.estimatedHeight
    /// The keyboard is up (Activity's search): the tab bar steps aside rather than ride on it over the results.
    @State private var keyboardUp = false

    var body: some View {
        @Bindable var model = model
        ZStack {
            ForEach(AppSection.allCases) { section in
                if visited.contains(section) || section == model.section {
                    let shown = section == model.section
                    NavigationStack(path: Binding(get: { model.path(section) }, set: { model.paths[section] = $0 })) {
                        // The one place the room under the tab bar is given: every root page scrolls its last row
                        // clear of the bar. Pushed pages have no tab bar, so they get none.
                        SectionRoot(section: section)
                            .tabBarClearance(keyboardUp ? 0 : barHeight + Self.barGap)
                            .navigationDestination(for: Route.self) { RouteView(route: $0) }
                    }
                    .pageFeedback(model.path(section).count)
                    .opacity(shown ? 1 : 0)
                    .allowsHitTesting(shown)
                    .accessibilityHidden(!shown)
                }
            }
        }
        .overlay(alignment: .bottom) {
            // At a place's root only: a pushed page has its own bottom (a dock, "This was wrong") and its back button.
            // It floats over the page, which scrolls on beneath it; the page's room for it is `tabBarClearance`. Never on
            // the keyboard: while it is up the results take its inset and the bar comes back when it goes.
            if model.path(model.section).isEmpty && !keyboardUp {
                SignalTabBar(items: items, selection: Binding(get: { model.section }, set: { model.select($0) }))
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { barHeight = $0 }
                    .padding(.bottom, Self.barGap)
                    .transition(reduceMotion ? .opacity : .move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(Motion.of(Motion.easeOut(Motion.drillIn), reduce: reduceMotion), value: model.path(model.section).isEmpty)
        .animation(Motion.of(Motion.easeOut(Motion.drillIn), reduce: reduceMotion), value: keyboardUp)
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in keyboardUp = true }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in keyboardUp = false }
        .onChange(of: model.section, initial: true) { _, section in visited.insert(section) }
    }

    private var items: [SignalTabBar<AppSection>.Item] {
        AppSection.allCases.map { section in
            SignalTabBar.Item(
                tab: section, title: section.title, glyph: section.glyph, identifier: section.identifier,
                dot: section == .trust ? runningMode?.fill : nil, count: section == .activity ? model.pending.count : 0
            )
        }
    }

    /// Overnight, Lockdown or Bypass runs: a dot in its colour on Trust.
    private var runningMode: ModeKind? {
        guard let mode = model.autopilot?.mode else { return nil }
        let kind = ModeKind(mode)
        return kind.calm ? nil : kind
    }

    /// The tab bar floats this far above the bottom safe area.
    private static let barGap: CGFloat = 4
}

// MARK: Regular

private struct SplitShell: View {
    @Environment(AppModel.self) private var model
    @Environment(\.hinge) private var hinge
    @State private var columns: NavigationSplitViewVisibility = .all

    var body: some View {
        @Bindable var model = model
        NavigationSplitView(columnVisibility: $columns) {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 280)
        } content: {
            SectionRoot(section: model.section)
                .id(model.section)
                .navigationSplitViewColumnWidth(min: 380, ideal: hinge == .bent ? 400 : 440, max: 540)
        } detail: {
            DetailColumn()
                .pageFeedback(model.path(model.section).count, in: model.section)
        }
        .navigationSplitViewStyle(.balanced)
    }
}

private struct Sidebar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(selection: Binding(get: { model.section }, set: { if let s = $0 { model.select(s) } })) {
            ForEach(AppSection.allCases) { section in
                NavigationLink(value: section) {
                    Label {
                        HStack {
                            Text(section.title).font(RFont.sans(16, .medium))
                            Spacer()
                            if section == .activity { CountBadge(count: model.pending.count) }
                        }
                    } icon: {
                        GlyphIcon(section.glyph, size: 20)
                    }
                }
                .accessibilityIdentifier(section.identifier)
            }
        }
        .navigationTitle("Reins")
        .onChange(of: model.section) { model.feedback.play(.selection) }
    }
}

/// The detail column: the first route of the section's path at the root, the rest pushed.
private struct DetailColumn: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let section = model.section
        let path = model.path(section)
        NavigationStack(path: Binding(
            get: { Array(path.dropFirst()) },
            set: { rest in
                if let first = model.path(section).first { model.paths[section] = [first] + rest }
            }
        )) {
            Group {
                if let first = path.first {
                    RouteView(route: first)
                } else {
                    DetailPlaceholder(section: section)
                }
            }
            .navigationDestination(for: Route.self) { RouteView(route: $0) }
        }
        .id("\(section.rawValue):\(path.first.map(String.init(describing:)) ?? "")")
    }
}

private struct DetailPlaceholder: View {
    var section: AppSection

    var body: some View {
        let (symbol, text): (String, String) = switch section {
        case .activity: ("waveform.path.ecg", "Pick a request or an entry to see everything about it.")
        case .trust: ("checkmark.shield", "Pick a grant or a profile to see what it allows and what Autopilot learned.")
        case .connections: ("link", "Pick an AI, a computer or a service to see what it can reach.")
        case .settings: ("gearshape", "Pick a setting.")
        }
        EmptyState(symbol: symbol, title: section.title, message: text)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .pageBackground()
    }
}

// MARK: Content

/// A section's root screen.
struct SectionRoot: View {
    var section: AppSection

    var body: some View {
        switch section {
        case .activity: ActivityScreen()
        case .trust: TrustScreen()
        case .connections: ConnectionsScreen()
        case .settings: SettingsScreen()
        }
    }
}

/// The screen for a route.
struct RouteView: View {
    var route: Route

    var body: some View {
        switch route {
        case .sounds: SoundsScreen()
        case .vaultPasskeys: VaultPasskeysScreen()
        case .appearance: AppearanceScreen()
        case .wallpaper: WallpaperScreen()
        case .devices: DevicesScreen()
        case .circles: CirclesScreen()
        case .vault: VaultScreen()
        case .vaultAdd: VaultAddScreen()
        case let .vaultItem(id): VaultItemScreen(id: id)
        case let .vaultEdit(id, newItem): VaultEditScreen(id: id, newItem: newItem)
        case let .autopilotProfile(id): ProfileScreen(profileId: id)
        case let .tryIt(id): TryItScreen(profileId: id)
        case let .connection(id): ConnectionDetailScreen(connectionId: id)
        case .gmail: GmailScreen()
        case .permissions: PermissionsHomeScreen()
        case let .accountPermissions(service, account, temporary): AccountPermissionsScreen(service: service, account: account, temporary: temporary)
        case let .service(id): ServiceScreen(serviceId: id)
        case .mcpAdd: McpAddScreen()
        case let .mcpServer(id): McpServerScreen(serverId: id)
        case let .activityDetail(id): ActivityDetailScreen(entryId: id)
        case let .email(entryId, index): EmailScreen(entryId: entryId, index: index)
        case let .grantDetail(id): GrantDetailScreen(grantId: id)
        case .expiredGrants: ExpiredGrantsScreen()
        case .newGrant: NewGrantScreen()
        }
    }
}

/// The sheet for an item that waits: an approval full screen; the others tall on a phone (the content stays visible
/// above them), a form sheet on iPad.
struct SheetContent: View {
    var target: SheetTarget
    /// The shell's width class (inside a form sheet the sheet's own is compact): a regular width gets a plain form
    /// sheet, since a fractional detent there pushes the sheet's bottom, and its buttons, off the screen.
    var regular: Bool
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            switch target {
            case let .approval(id): ApprovalSheet(requestId: id)
            case let .pairing(id): PairingSheet(pairingId: id)
            case let .upload(id): UploadSheet(blobId: id)
            case let .join(id): JoinRequestSheet(joinId: id)
            case .connectComputer: ConnectComputerSheet(typing: model.connectStartsTyped)
            }
        }
        .environment(model)
        .environment(\.feedback, model.feedback)
        .modifier(SheetShape(regular: regular, fullScreen: target.isApproval, ownDetents: target.ownDetents))
        // Quiet when the request's chime just announced it (a request that pops up by itself).
        .onAppear { model.feedback.sheetOpened() }
    }
}

/// Detents on a phone; a plain form sheet, with no detents, on a regular width.
private struct SheetShape: ViewModifier {
    var regular: Bool
    /// A full-screen cover (an approval) takes no detents.
    var fullScreen = false
    /// Pairing, joining and connecting a computer size themselves (the Connect board's sheets).
    var ownDetents = false

    func body(content: Content) -> some View {
        if fullScreen {
            content
        } else if ownDetents && !regular {
            content
        } else if regular {
            content.presentationSizing(.form)
        } else {
            content
                .presentationDetents([.fraction(0.86), .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(32)
        }
    }
}
