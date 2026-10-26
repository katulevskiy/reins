import LocalAuthentication
import SwiftUI

/// A request that waits for the user, full screen (the Android app's ApprovalScreen): the field at the top says who
/// asks, how long it waits, what in one sentence, every warning and Autopilot's score; the facts below (long lists folded into
/// pages), and the dock at the bottom with how long to remember it and Deny / Approve. Approving asks for Face ID,
/// Touch ID or the passcode first; denying never does.
struct ApprovalSheet: View {
    var requestId: String
    @Environment(AppModel.self) private var model
    @State private var vm: ApprovalModel

    init(requestId: String) {
        self.requestId = requestId
        _vm = State(initialValue: ApprovalModel(requestId: requestId))
    }

    /// A screen already holding its request (previews).
    init(model: ApprovalModel) {
        requestId = model.requestId
        _vm = State(initialValue: model)
    }

    var body: some View {
        Group {
            if let view = vm.view {
                ApprovalScreen(vm: vm, view: view)
            } else {
                DecisionLoading(error: vm.loading ? nil : vm.error)
            }
        }
        .task { await vm.load(model) }
        .onChange(of: vm.finished) { _, done in if done { model.closeSheet() } }
    }
}

/// A page pushed inside the approval screen; the dock stays under every one.
enum ApprovalPage: Hashable {
    case commits(ref: Int)
    case commit(ref: Int, index: Int)
    case files(ref: Int)
    /// Long text folded out of the main page (a search, a tool's arguments).
    case text(title: String, body: String, mono: Bool, id: String)
}

/// What slides up over the approval screen.
enum ApprovalOverlay: String, Identifiable {
    case remember, autopilot, grant
    var id: String { rawValue }
}

/// The approval screen's navigation: the pages pushed and the sheet up.
@Observable
@MainActor
final class ApprovalNav {
    var path: [ApprovalPage] = []
    var overlay: ApprovalOverlay?
    /// The risk's reason did not fit the field's two lines, so it is the first fact and the field says only its word.
    var riskInFacts = false
}

private struct ApprovalScreen: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model
    @Environment(\.hinge) private var hinge
    @Environment(\.dynamicTypeSize) private var typeSize
    @State private var nav = ApprovalNav()
    @State private var autopilot = AutopilotModel()
    /// The request was in the waiting list while the screen was open, so its leaving means it was answered elsewhere
    /// or the server dropped it.
    @State private var wasListed = false

    private var gone: Bool { wasListed && !vm.busy && !vm.finished && !model.pending.contains { $0.id == view.requestId } }

    var body: some View {
        GeometryReader { geo in
            // On a bent iPhone Duo the pages take one half and the recap with the dock the other, so the crease runs
            // between them.
            let split = hinge == .bent && max(geo.size.width, geo.size.height) >= 640
            if split {
                let sideBySide = geo.size.width >= geo.size.height
                let layout = sideBySide ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
                layout {
                    pages(top: geo.safeAreaInsets.top)
                        .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                    VStack(spacing: 16) {
                        Spacer(minLength: 0)
                        Text(AskFormat.waiting(view, core: model.pending.first { $0.id == view.requestId }?.label).text)
                            .font(RFont.sans(22, .semibold))
                            .foregroundStyle(Palette.ink)
                            .lineLimit(3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 20)
                        dock
                    }
                    .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                    .background(Palette.paperRaised.opacity(0.5))
                }
            } else {
                // On an island phone the field's top row takes the island's ears, so the status bar steps aside.
                pages(top: geo.safeAreaInsets.top).safeAreaInset(edge: .bottom, spacing: 0) { dock }
                    .statusBarHidden(FieldEars.of(top: geo.safeAreaInsets.top, accessibility: typeSize.isAccessibilitySize).hidesStatusBar)
            }
        }
        .background(Palette.paper.ignoresSafeArea())
        .environment(nav)
        .sheet(item: $nav.overlay) { overlay in
            Group {
                switch overlay {
                case .remember: RememberSheet(vm: vm, view: view)
                case .autopilot:
                    if let s = vm.suggestion {
                        AutopilotSheet(suggestion: s) {
                            nav.overlay = nil
                            model.closeSheet()
                            model.open(.trust)
                        }
                    }
                case .grant: GrantSheet(vm: vm, view: view)
                }
            }
            .environment(model)
            .environment(\.feedback, model.feedback)
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(32)
        }
        .onAppear {
            autopilot.bind(model)
            #if DEBUG
            // Screenshots: `-approvalPage commits|commit` opens a drill-in, `-approvalOverlay remember|autopilot|grant` a sheet.
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-approvalPage"), i + 1 < args.count {
                nav.path = args[i + 1] == "commit" ? [.commit(ref: 0, index: 0)] : [.commits(ref: 0)]
            }
            if let i = args.firstIndex(of: "-approvalOverlay"), i + 1 < args.count {
                let overlay = ApprovalOverlay(rawValue: args[i + 1])
                Task { try? await Task.sleep(for: .milliseconds(600)); nav.overlay = overlay }
            }
            #endif
            if model.pending.contains(where: { $0.id == view.requestId }) { wasListed = true }
        }
        .onChange(of: model.pending.map(\.id)) { _, ids in if ids.contains(view.requestId) { wasListed = true } }
        .onChange(of: autopilot.modelReady) { _, ready in
            if ready { Task { await vm.reloadSuggestion(model) } }
        }
        .accessibilityContainer("approvalSheet")
    }

    /// `top`: the status bar's height, which the field runs under (the main page's scroll ignores it and the field
    /// keeps its content below).
    private func pages(top: CGFloat) -> some View {
        NavigationStack(path: $nav.path) {
            ScrollView {
                MainPage(vm: vm, view: view, autopilot: autopilot, gone: gone, top: top).padding(.bottom, 20)
            }
            .ignoresSafeArea(edges: .top)
            .scrollEdgeEffectHidden(true, for: .top)
            .scrollDismissesKeyboard(.interactively)
            .toolbar(.hidden, for: .navigationBar)
            // White status bar text on the field.
            .toolbarColorScheme(.dark, for: .navigationBar)
            .background(Palette.paper.ignoresSafeArea())
            .navigationDestination(for: ApprovalPage.self) { page in
                ScrollView {
                    page.content(view)
                        .padding(.bottom, 20)
                }
                .background(Palette.paper.ignoresSafeArea())
                .fieldBand(view, title: Self.title(view, page), ask: AskFormat.waiting(view, core: model.pending.first { $0.id == view.requestId }?.label))
            }
        }
    }

    @ViewBuilder private var dock: some View {
        if gone {
            Button("Close") {
                model.closeSheet()
                model.feedback.defaultTap()
            }
            .buttonStyle(.glass)
            .font(RFont.sans(17, .semibold))
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
        } else {
            ApprovalDock(vm: vm, view: view)
        }
    }

    static func title(_ view: ApprovalView, _ page: ApprovalPage) -> String { page.title(in: view) }
}

extension ApprovalPage {
    /// The page's own title in the bar: "8 commits", a commit's short id, "12 files".
    func title(in view: ApprovalView) -> String {
        let refs = view.git?.refs ?? []
        switch self {
        case let .commits(ref): return refs.indices.contains(ref) ? ApprovalWords.commits(Int(refs[ref].commitCount)) : ""
        case let .commit(ref, index):
            guard refs.indices.contains(ref), refs[ref].commits.indices.contains(index) else { return "" }
            return untrusted(refs[ref].commits[index].shortSha)
        case let .files(ref): return refs.indices.contains(ref) ? ApprovalWords.files(Int(refs[ref].filesChanged)) : ""
        case let .text(title, _, _, _): return title
        }
    }

    /// The page itself (the approval screen and an Activity entry push the same ones).
    @ViewBuilder func content(_ view: ApprovalView) -> some View {
        switch self {
        case let .commits(ref): CommitsPage(view: view, refIndex: ref)
        case let .commit(ref, index): CommitPage(view: view, refIndex: ref, index: index)
        case let .files(ref): FilesPage(view: view, refIndex: ref)
        case let .text(_, text, mono, id): TextPage(text: text, mono: mono, id: id)
        }
    }
}

extension View {
    /// A drill-in's bar as the field's band (Next-Approve 02, 03): the field's colour, the page's title over the ask in
    /// one line (its target in 600), and the timer. The system bar stays, so the back button and the edge swipe work as
    /// anywhere.
    func fieldBand(_ view: ApprovalView, title: String, ask: AskLine) -> some View {
        navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(ApprovalWords.tone(view).field.fill, for: .navigationBar)
            .toolbarBackgroundVisibility(.visible, for: .navigationBar)
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbar {
                ToolbarItem(placement: .principal) {
                    VStack(spacing: 1) {
                        Text(verbatim: title).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.onField)
                        AskRunsText(runs: ask.runs, size: 13).foregroundStyle(Palette.onField.opacity(0.85))
                    }
                    .lineLimit(1)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(title), \(ask.text)")
                    .accessibilityIdentifier("askBand")
                }
                ToolbarItem(placement: .topBarTrailing) { FieldTimer(view: view) }
                    .sharedBackgroundVisibility(.hidden)
            }
    }
}

/// The text a (?) opens, in a popover bubble.
struct HelpText: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(14))
            .foregroundStyle(Palette.ink)
            .fixedSize(horizontal: false, vertical: true)
            .frame(idealWidth: 290, maxWidth: 320, alignment: .leading)
            .padding(15)
            .presentationCompactAdaptation(.popover)
            .accessibilityIdentifier("helpBubble")
    }
}

/// A round (?) of its own, never inside another tappable thing; it opens `text` in a popover.
struct HelpDot: View {
    var id: String
    var text: String
    var size: CGFloat = 32
    /// On a field: the white wash and a white mark.
    var onField = false
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            open = true
            feedback.play(.open)
        } label: {
            Image(systemName: "questionmark.circle")
                .font(.system(size: size * 0.56, weight: .medium))
                .foregroundStyle(onField ? Palette.onField : open ? Palette.ink : Palette.ink2)
                .frame(width: size, height: size)
                .background(onField ? Palette.onFieldWash : open ? Palette.ink.opacity(0.18) : Palette.paperSunken, in: Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("More about this")
        .accessibilityIdentifier("help:\(id)")
        .popover(isPresented: $open) { HelpText(text: text) }
    }
}

/// The phone's biometry, for the Approve button's symbol: Face ID, Touch ID, Optic ID, else a lock (the passcode).
enum Biometry {
    static let symbol: String = {
        let context = LAContext()
        var error: NSError?
        _ = context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &error)
        switch context.biometryType {
        case .faceID: return "faceid"
        case .touchID: return "touchid"
        case .opticID: return "opticid"
        case .none: return "lock.fill"
        @unknown default: return "lock.fill"
        }
    }()
}

#if DEBUG
#Preview("Search") {
    PreviewHost { ApprovalSheet(requestId: "req1") }
}
#endif

#if DEBUG
#Preview("Push") {
    PreviewHost { ApprovalSheet(requestId: "req20") }
}
#endif

#if DEBUG
#Preview("Permission") {
    PreviewHost { ApprovalSheet(requestId: "req3") }
}
#endif
