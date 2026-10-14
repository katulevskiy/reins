import LocalAuthentication
import SwiftUI

/// A request that waits for the user, full screen (the Android app's ApprovalScreen): who asks and how long it waits at
/// the top, a hero that says what in one sentence with Autopilot's score, the facts below (long lists folded into
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
}

private struct ApprovalScreen: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model
    @Environment(\.hinge) private var hinge
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
                    pages
                        .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                    VStack(spacing: 16) {
                        Spacer(minLength: 0)
                        Text(ApprovalWords.sentence(view).map(\.text).joined())
                            .font(RFont.sans(22, .semibold))
                            .foregroundStyle(Palette.text)
                            .lineLimit(3)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 20)
                        dock
                    }
                    .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                    .background(Palette.elevated.opacity(0.5))
                }
            } else {
                pages.safeAreaInset(edge: .bottom, spacing: 0) { dock }
            }
        }
        .background(Palette.background.ignoresSafeArea())
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
                            model.select(.autopilot)
                        }
                    }
                case .grant: GrantSheet(vm: vm, view: view)
                }
            }
            .environment(model)
            .environment(\.feedback, model.feedback)
            .presentationDragIndicator(.visible)
            .presentationCornerRadius(34)
        }
        .onAppear {
            autopilot.bind(model)
            if model.pending.contains(where: { $0.id == view.requestId }) { wasListed = true }
        }
        .onChange(of: model.pending.map(\.id)) { _, ids in if ids.contains(view.requestId) { wasListed = true } }
        .onChange(of: autopilot.modelReady) { _, ready in
            if ready { Task { await vm.reloadSuggestion(model) } }
        }
        .accessibilityContainer("approvalSheet")
    }

    private var pages: some View {
        NavigationStack(path: $nav.path) {
            ScrollView {
                MainPage(vm: vm, view: view, autopilot: autopilot, gone: gone).padding(.bottom, 20)
            }
            .scrollDismissesKeyboard(.interactively)
            .safeAreaInset(edge: .top, spacing: 0) { AgentBar(view: view) }
            .toolbar(.hidden, for: .navigationBar)
            .background(Palette.background.ignoresSafeArea())
            .navigationDestination(for: ApprovalPage.self) { page in
                ScrollView {
                    Group {
                        switch page {
                        case let .commits(ref): CommitsPage(view: view, refIndex: ref)
                        case let .commit(ref, index): CommitPage(view: view, refIndex: ref, index: index)
                        case let .files(ref): FilesPage(view: view, refIndex: ref)
                        case let .text(_, text, mono, id): TextPage(text: text, mono: mono, id: id)
                        }
                    }
                    .padding(.bottom, 20)
                }
                .background(Palette.background.ignoresSafeArea())
                .navigationTitle(Self.title(view, page))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) { ApprovalTimerPill(view: view) }
                }
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

    /// The page's own title in the bar: "8 commits", a commit's short id, "12 files".
    static func title(_ view: ApprovalView, _ page: ApprovalPage) -> String {
        let refs = view.git?.refs ?? []
        switch page {
        case let .commits(ref): return refs.indices.contains(ref) ? ApprovalWords.commits(Int(refs[ref].commitCount)) : ""
        case let .commit(ref, index):
            guard refs.indices.contains(ref), refs[ref].commits.indices.contains(index) else { return "" }
            return refs[ref].commits[index].shortSha
        case let .files(ref): return refs.indices.contains(ref) ? ApprovalWords.files(Int(refs[ref].filesChanged)) : ""
        case let .text(title, _, _, _): return title
        }
    }
}

/// Close, the agent ("Claude Code · laptop") and the time it still waits.
private struct AgentBar: View {
    var view: ApprovalView
    @Environment(AppModel.self) private var model

    var body: some View {
        let host = findConnection(model.connections, id: view.connectionId, label: view.connectionLabel).map { untrusted($0.clientHost) }
        HStack(spacing: 10) {
            GlassIconButton(symbol: "xmark", size: 36, label: "Close") { model.closeSheet() }
                .accessibilityIdentifier("closeSheet")
            HStack(spacing: 9) {
                ConnectionIcon(connectionId: view.connectionId, label: view.connectionLabel, size: 28)
                (Text(untrusted(view.connectionLabel)).foregroundStyle(Palette.text)
                    + Text(host.map { $0.isEmpty ? "" : " · \($0)" } ?? "").font(RFont.sans(15, .medium)).foregroundStyle(Palette.tertiary))
                    .font(RFont.sans(15, .semibold))
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            ApprovalTimerPill(view: view)
        }
        .padding(.horizontal, 16)
        .padding(.top, 6)
        .padding(.bottom, 10)
        .background(Palette.background)
    }
}

/// The time the AI still waits: a ring that empties and the seconds, red in the last 15; "Stopped" once it gave up
/// (its (?) says approving still works).
struct ApprovalTimerPill: View {
    var view: ApprovalView
    @State private var lateHelp = false

    var body: some View {
        UrgencyClock(createdAt: view.createdAt, waitUntil: view.waitUntil) { urgency in
            if let u = urgency {
                if u.stale {
                    Button {
                        lateHelp = true
                    } label: {
                        Label("Stopped", systemImage: "timer")
                            .font(RFont.sans(14, .semibold))
                            .foregroundStyle(Palette.secondary)
                            .padding(.horizontal, 11)
                            .padding(.vertical, 7)
                            .background(Palette.controlFill, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("lateBanner")
                    .popover(isPresented: $lateHelp) {
                        HelpText(text: "\(untrusted(view.connectionLabel)) stopped waiting. You can still approve; then ask it to try again.")
                    }
                } else {
                    HStack(spacing: 7) {
                        ZStack {
                            Circle().stroke(u.urgent ? Palette.danger.opacity(0.22) : Palette.controlFill, lineWidth: 2.5)
                            Circle()
                                .trim(from: 0, to: u.fraction)
                                .stroke(u.urgent ? Palette.danger : Palette.secondary, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
                                .rotationEffect(.degrees(-90))
                        }
                        .frame(width: 16, height: 16)
                        Text(ApprovalWords.seconds(u.remainingSeconds))
                            .font(RFont.sans(14, .semibold))
                            .monospacedDigit()
                            .foregroundStyle(u.urgent ? Palette.danger : Palette.secondary)
                    }
                    .padding(.leading, 9)
                    .padding(.trailing, 11)
                    .padding(.vertical, 7)
                    .background(u.urgent ? Palette.danger.opacity(0.13) : Palette.controlFill, in: Capsule())
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("waitLine")
                }
            }
        }
    }
}

/// The text a (?) opens, in a popover bubble.
struct HelpText: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(14))
            .foregroundStyle(Palette.text)
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
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            open = true
            feedback.play(.open)
        } label: {
            Image(systemName: "questionmark.circle")
                .font(.system(size: size * 0.56, weight: .medium))
                .foregroundStyle(open ? Palette.text : Palette.secondary)
                .frame(width: size, height: size)
                .background(open ? Palette.text.opacity(0.18) : Palette.controlFill, in: Circle())
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
