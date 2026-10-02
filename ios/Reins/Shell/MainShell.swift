import SwiftUI

/// The signed-in app. A compact width (an iPhone, a folded iPhone Duo, iPad Slide Over) gets a Liquid Glass tab bar
/// with a navigation stack per tab; a regular width (iPad, an unfolded iPhone Duo) gets a sidebar, the section's
/// list and a detail column. Both read the same navigation state (`AppModel.section` / `paths`), so folding or
/// resizing keeps what is open.
struct MainShell: View {
    @Environment(AppModel.self) private var model
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        @Bindable var model = model
        HingeReader { _ in
            Group {
                if sizeClass == .regular {
                    SplitShell()
                } else {
                    TabShell()
                }
            }
        }
        .sheet(item: $model.sheet, onDismiss: { model.feedback.cueUnlessRecent(.close) }) { target in
            SheetContent(target: target, regular: sizeClass == .regular)
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
        .onOpenURL { url in
            guard let link = DeepLink(url: url) else { return }
            Task { await model.handle(link) }
        }
    }
}

// MARK: Compact

private struct TabShell: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        TabView(selection: Binding(get: { model.section }, set: { model.select($0) })) {
            ForEach(AppSection.allCases) { section in
                Tab(section.title, systemImage: section.symbol, value: section) {
                    NavigationStack(path: Binding(get: { model.path(section) }, set: { model.paths[section] = $0 })) {
                        SectionRoot(section: section)
                            .navigationDestination(for: Route.self) { RouteView(route: $0) }
                    }
                }
                .badge(badge(section))
            }
        }
        .tabBarMinimizeBehavior(.onScrollDown)
        .onChange(of: model.section) { model.feedback.play(.selection) }
    }

    private func badge(_ section: AppSection) -> Int {
        switch section {
        case .activity: model.pending.count
        case .grants: model.activeGrants
        default: 0
        }
    }
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
                .navigationSplitViewColumnWidth(min: 220, ideal: 250, max: 300)
        } content: {
            SectionRoot(section: model.section)
                .id(model.section)
                .navigationSplitViewColumnWidth(min: 340, ideal: hinge == .bent ? 400 : 420, max: 520)
        } detail: {
            DetailColumn()
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
                        Image(systemName: section.symbol)
                    }
                }
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
        case .grants: ("key.horizontal", "Pick a grant to see what it allows and how it was used.")
        case .autopilot: ("bolt.shield", "Pick a profile to see what Autopilot learned.")
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
        case .grants: GrantsScreen()
        case .autopilot: AutopilotScreen()
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
        case .autopilot: AutopilotScreen()
        case let .autopilotProfile(id): ProfileScreen(profileId: id)
        case let .tryIt(id): TryItScreen(profileId: id)
        case let .connection(id): ConnectionDetailScreen(connectionId: id)
        case .integrations: IntegrationsScreen()
        case .gmail: GmailScreen()
        case let .service(id): ServiceScreen(serviceId: id)
        case .mcpAdd: McpAddScreen()
        case let .mcpServer(id): McpServerScreen(serverId: id)
        case let .activityDetail(id): ActivityDetailScreen(entryId: id)
        case let .email(entryId, index): EmailScreen(entryId: entryId, index: index)
        case let .grantDetail(id): GrantDetailScreen(grantId: id)
        case .newGrant: NewGrantScreen()
        }
    }
}

/// The sheet for an item that waits: tall on a phone (the content stays visible above it), a form sheet on iPad.
private struct SheetContent: View {
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
            }
        }
        .environment(model)
        .environment(\.feedback, model.feedback)
        .modifier(SheetShape(regular: regular))
        .onAppear { model.feedback.cue(.open) }
    }
}

/// Detents on a phone; a plain form sheet, with no detents, on a regular width.
private struct SheetShape: ViewModifier {
    var regular: Bool

    func body(content: Content) -> some View {
        if regular {
            content.presentationSizing(.form)
        } else {
            content
                .presentationDetents([.fraction(0.86), .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(32)
        }
    }
}
