#if DEBUG
import SwiftUI
import WidgetKit

extension LiveActivityController {
    /// `-demoBypass`: a 15-minute bypass right after launch, so the Bypass Live Activity can be looked at;
    /// `-demoBypassUntilOff`: one that runs until turned off.
    func debugLaunch(model: AppModel) {
        let args = ProcessInfo.processInfo.arguments
        let untilOff = args.contains("-demoBypassUntilOff")
        guard untilOff || args.contains("-demoBypass") else { return }
        Task { @MainActor in
            for _ in 0..<50 where model.session == .loading { try? await Task.sleep(for: .milliseconds(100)) }
            try? await Task.sleep(for: .seconds(1))
            try? await model.core.startBypass(connectionId: nil, minutes: untilOff ? nil : 15)
            await model.refreshAutopilot()
        }
    }
}

/// Every widget and Live Activity drawn with the same views the widget extension uses, at their real sizes, laid out
/// as the Outside board's phones (launch with `-demo -widgetGallery <page>`; pages: lock, longpress, island, widgets,
/// bypass, controls, states). Adding widgets to the Home Screen cannot be scripted, so screenshots of this stand in for
/// them. What the system draws itself (a notification, its long-press menu, Control Center's tiles) is mocked plainly:
/// Reins supplies only their words.
struct WidgetGallery: View {
    @Environment(AppModel.self) private var model
    @State private var snapshot = WidgetGallery.load()
    @State private var now = Glance.now()

    /// The snapshot the app wrote, or with `-gallerySample` the board's own sample (three waiting, a little history).
    static func load() -> Snapshot {
        ProcessInfo.processInfo.arguments.contains("-gallerySample") ? Glance.sample(now: Glance.now()) : Snapshot.load()
    }

    static var requested: Bool { ProcessInfo.processInfo.arguments.contains("-widgetGallery") }

    static var page: String {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-widgetGallery"), i + 1 < args.count, !args[i + 1].hasPrefix("-") else { return "widgets" }
        return args[i + 1]
    }

    var body: some View {
        Group {
            switch Self.page {
            case "lock": lock
            case "longpress": longPress
            case "island": island
            case "bypass": bypass
            case "controls": controls
            case "states": ScrollView { states.padding(20) }.background(Palette.paper)
            default: widgets
            }
        }
        .statusBarHidden()
        .task {
            while !Task.isCancelled {
                snapshot = Self.load()
                now = Glance.now()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    // MARK: Pages (the Outside board)

    /// 01: a notification Autopilot sent, and the waiting request as a Live Activity.
    private var lock: some View {
        ZStack(alignment: .bottom) {
            LockWallpaper()
            VStack(spacing: 8) {
                MockNotification(title: "Claude: Read 3 emails", body: "Approved by Autopilot · 97%", when: "2m ago")
                if let state = approvalState {
                    let v = ApprovalLiveViews(state: state, stale: false)
                    liveCard(v.background) { v.lockScreen }
                }
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 96)
        }
        .environment(\.colorScheme, .dark)
    }

    /// 02: the system's long-press menu on a routine request: Deny and Approve (a risky one offers only Deny).
    private var longPress: some View {
        ZStack(alignment: .top) {
            LockWallpaper().blur(radius: 6)
            Color(red: 10 / 255, green: 10 / 255, blue: 16 / 255).opacity(0.55).ignoresSafeArea()
            VStack(spacing: 10) {
                MockNotification(title: "Claude: Read 3 emails", body: "me@gmail.com", when: "now")
                VStack(spacing: 0) {
                    menuRow("Deny", glyph: .close, red: true)
                    Rectangle().fill(Color.black.opacity(0.12)).frame(height: 1)
                    menuRow("Approve", glyph: .check, red: false)
                }
                .background(RoundedRectangle(cornerRadius: 16, style: .continuous).fill(Color(white: 0.96).opacity(0.88)))
            }
            .padding(.horizontal, 12)
            .padding(.top, 196)
        }
        .environment(\.colorScheme, .dark)
    }

    /// 03: the Dynamic Island, compact, minimal and expanded, then a bypass and the model download.
    private var island: some View {
        ZStack(alignment: .top) {
            Color(red: 42 / 255, green: 40 / 255, blue: 56 / 255).ignoresSafeArea()
            if let state = approvalState {
                let v = ApprovalLiveViews(state: state, stale: false)
                let b = BypassLiveViews(state: bypassState, stale: false)
                let d = ModelDownloadLiveViews(state: downloadState)
                VStack(spacing: 0) {
                    compact { v.compactLeading } trailing: { v.compactTrailing }
                    label("Compact").padding(.top, 10)
                    HStack(spacing: 8) {
                        Capsule().fill(.black).frame(width: 124, height: 36)
                        minimal { v.minimal }
                    }
                    .padding(.top, 116)
                    label("Minimal").padding(.top, 12)
                    expanded(leading: v.expandedLeading, trailing: v.expandedTrailing, center: v.expandedCenter, bottom: v.expandedBottom)
                        .padding(.top, 34)
                    label("Expanded").padding(.top, 70)
                    compact { b.compactLeading } trailing: { b.compactTrailing }.padding(.top, 72)
                    label("Bypass").padding(.top, 10)
                    compact { d.compactLeading } trailing: { d.compactTrailing }.padding(.top, 34)
                    label("Autopilot model").padding(.top, 10)
                }
                .padding(.top, -48)
            }
        }
        .environment(\.colorScheme, .dark)
    }

    /// 04: the Home Screen widgets.
    private var widgets: some View {
        ZStack(alignment: .top) {
            LockWallpaper()
            VStack(spacing: 22) {
                HStack(spacing: 22) {
                    plate(.systemSmall, field: smallField(snapshot)) { WaitingSmallView(snapshot: snapshot, now: now) }
                    plate(.systemSmall) { AutopilotWidgetView(snapshot: snapshot, now: now, medium: false) }
                }
                plate(.systemMedium) { LatestActivityView(snapshot: latestSnapshot, now: now, rows: 3) }
                plate(.systemMedium) { WaitingListView(snapshot: snapshot, now: now, rows: 3) }
            }
            .padding(.top, 12)
        }
    }

    /// 05: the Lock Screen accessories, a bypass and the model download.
    private var bypass: some View {
        ZStack(alignment: .top) {
            LockWallpaper()
            accessory(width: 300, height: 22) { WaitingInlineView(snapshot: snapshot, now: now) }
                .padding(.top, 38)
            HStack(spacing: 12) {
                accessory(width: 72, height: 72) { WaitingCircularView(snapshot: snapshot, now: now) }
                accessory(width: nil, height: 72) { WaitingRectangularView(snapshot: snapshot, now: now) }
            }
            .padding(.horizontal, 24)
            .padding(.top, 242)
            VStack(spacing: 8) {
                Spacer()
                let b = BypassLiveViews(state: bypassState, stale: false)
                liveCard(b.background) { b.lockScreen }
                liveCard(GlanceStyle.liveBackground) { ModelDownloadLiveViews(state: downloadState).lockScreen }
                    .environment(\.colorScheme, .light)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 96)
        }
        .environment(\.colorScheme, .dark)
    }

    /// 06: Control Center keeps the system's title and state lines; a control is coloured only while on. Then the Siri
    /// phrases of `ReinsShortcuts`.
    private var controls: some View {
        ZStack(alignment: .top) {
            Color(red: 42 / 255, green: 40 / 255, blue: 56 / 255).ignoresSafeArea()
            Color.black.opacity(0.35).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 0) {
                let mode = Glance.mode(controlSnapshot, now: now)
                let waiting = Glance.waiting(snapshot, now: now).count
                let bypassOn = Glance.bypassEnd(controlSnapshot, now: now) != nil
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)], spacing: 12) {
                    MockControl(symbol: ControlLook.lockdownSymbol, tint: mode == .lockdown ? Palette.fieldWarn : nil, title: "Lockdown", state: mode == .lockdown ? "On" : "Off")
                    MockControl(symbol: ControlLook.waitingSymbol, tint: waiting > 0 ? Palette.fieldPush : nil, title: Glance.countLine(waiting), state: "Requests")
                    MockControl(symbol: ControlLook.bypassSymbol, tint: bypassOn ? Palette.fieldDelete : nil, title: bypassOn ? "Stop bypass" : "No bypass", state: ControlLook.stopState(controlSnapshot, now: now))
                    MockControl(symbol: ControlLook.modeSymbol(mode), tint: ControlLook.modeTint(mode), title: mode.name, state: "Mode")
                }
                HStack(spacing: 6) {
                    FixedGlyph(.speaker, size: 15)
                    Text("Siri").font(RFont.fixedSans(13, .semibold))
                }
                .foregroundStyle(.white.opacity(0.7))
                .padding(.top, 30)
                .padding(.horizontal, 4)
                .padding(.bottom, 8)
                VStack(spacing: 0) {
                    ForEach(Array(Self.phrases.enumerated()), id: \.offset) { i, phrase in
                        HStack(spacing: 12) {
                            Group {
                                if let mode = phrase.mode { FixedModeGlyph(mode: mode, size: 17) } else { FixedGlyph(phrase.glyph, size: 17) }
                            }
                            .foregroundStyle(.white.opacity(0.7))
                            Text("“\(phrase.text)”").font(RFont.fixedSans(15, .medium)).foregroundStyle(.white)
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 48)
                        .overlay(alignment: .top) { if i > 0 { Rectangle().fill(.white.opacity(0.12)).frame(height: 1) } }
                    }
                }
                .background(RoundedRectangle(cornerRadius: 20, style: .continuous).fill(.white.opacity(0.1)))
            }
            .padding(.horizontal, 20)
            .padding(.top, 52)
        }
        .environment(\.colorScheme, .dark)
    }

    private static let phrases: [(glyph: Glyph, mode: ModeKind?, text: String)] = [
        (.block, .lockdown, "Lock down Reins"), (.lockOpen, nil, "Resume Reins"), (.autopilot, .auto, "Set Reins to Auto"),
        (.shieldAlert, .bypass, "Stop the Reins bypass"), (.tray, nil, "What is waiting in Reins"),
    ]

    /// Nothing waits, signed out, and the accented rendering.
    @ViewBuilder private var states: some View {
        let empty = with { $0.pending = [] }
        let out = Snapshot()
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 20) {
                plate(.systemSmall) { WaitingSmallView(snapshot: empty, now: now) }
                plate(.systemSmall) { WaitingSmallView(snapshot: out, now: now) }
            }
            plate(.systemMedium) { WaitingListView(snapshot: empty, now: now, rows: 3) }
            HStack(spacing: 20) {
                plate(.systemSmall, accented: true) { WaitingSmallView(snapshot: snapshot, now: now) }
                plate(.systemSmall, accented: true) { AutopilotWidgetView(snapshot: snapshot, now: now, medium: false) }
            }
            if let state = approvalState {
                let stale = ApprovalLiveViews(state: state, stale: true)
                liveCard(stale.background) { stale.lockScreen }
            }
            let off = BypassLiveViews(state: untilOffState, stale: false)
            liveCard(off.background) { off.lockScreen }
        }
    }

    // MARK: Sample states

    private func with(_ change: (inout Snapshot) -> Void) -> Snapshot {
        var s = snapshot
        change(&s)
        return s
    }

    /// The board's Latest: the sample's rows when the demo has none with services.
    private var latestSnapshot: Snapshot {
        snapshot.latest.contains { $0.service != nil } ? snapshot : with { $0.latest = Glance.sample(now: now).latest }
    }

    /// Controls as the board shows them: a bypass running.
    private var controlSnapshot: Snapshot {
        with { $0.autopilotMode = "bypass"; $0.bypassUntil = now + 47 * 60 + 12; $0.anyBypassUntil = $0.bypassUntil }
    }

    private var approvalState: ApprovalActivityAttributes.ContentState? {
        Glance.approvalState(snapshot.pending, now: now) ?? Glance.approvalState(Glance.sample(now: now).pending, now: now)
    }

    private var bypassState: BypassActivityAttributes.ContentState {
        let until = Date(timeIntervalSince1970: TimeInterval(now + 47 * 60 + 12))
        return .init(until: until, startedAt: until.addingTimeInterval(-60 * 60), scope: "every AI", approvedCount: 3)
    }

    /// A bypass that runs until turned off, on for two days.
    private var untilOffState: BypassActivityAttributes.ContentState {
        Glance.bypassState(
            global: AutopilotText.untilOff, connections: [], previous: nil, approvedSince: { _ in 41 }, now: now, since: now - 2 * 86_400
        ) ?? bypassState
    }

    private var downloadState: ModelDownloadActivityAttributes.ContentState {
        .init(downloaded: 126_000_000, total: 370_000_000, finished: false, failed: false)
    }

    // MARK: Frames

    private func label(_ text: String) -> some View {
        Text(text.uppercased()).font(RFont.fixedMono(10.5)).tracking(1.7).foregroundStyle(.white.opacity(0.55))
    }

    private func menuRow(_ title: String, glyph: Glyph, red: Bool) -> some View {
        HStack {
            Text(title).font(.system(size: 16))
            Spacer()
            FixedGlyph(glyph, size: 18)
        }
        .foregroundStyle(red ? Color(red: 185 / 255, green: 28 / 255, blue: 28 / 255) : Color(red: 22 / 255, green: 21 / 255, blue: 27 / 255))
        .padding(.horizontal, 16)
        .frame(minHeight: 48)
    }

    /// The small Waiting widget's field while something waits, as the widget picks it.
    private func smallField(_ s: Snapshot) -> FieldKind? {
        Glance.waiting(s, now: now).first.map(Glance.field)
    }

    /// A Home Screen widget at the 6.1" iPhone's size, on its plate (a field's colour and grain when `field` is set).
    private func plate<V: View>(_ family: WidgetFamily, accented: Bool = false, field: FieldKind? = nil, @ViewBuilder content: () -> V) -> some View {
        let size: CGSize = switch family {
        case .systemSmall: CGSize(width: 170, height: 170)
        case .systemMedium: CGSize(width: 362, height: 170)
        default: CGSize(width: 362, height: 382)
        }
        return content()
            .padding(14)
            .frame(width: size.width, height: size.height)
            .environment(\.widgetRenderingMode, accented ? .accented : .fullColor)
            .environment(\.colorScheme, accented ? .dark : colorSchemeFromTraits)
            .background {
                let shape = RoundedRectangle(cornerRadius: 22, style: .continuous)
                if let field, !accented {
                    GlanceField(kind: field).clipShape(shape)
                } else {
                    shape.fill(accented ? AnyShapeStyle(LinearGradient(colors: [Color(red: 0.12, green: 0.2, blue: 0.4), Color(red: 0.2, green: 0.12, blue: 0.35)], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(GlanceStyle.background))
                }
            }
            .environment(\.colorScheme, colorSchemeFromTraits)
    }

    @Environment(\.colorScheme) private var colorSchemeFromTraits

    /// A Lock Screen accessory: vibrant, white on the wallpaper.
    private func accessory<V: View>(width: CGFloat?, height: CGFloat, @ViewBuilder content: () -> V) -> some View {
        content()
            .environment(\.widgetRenderingMode, .vibrant)
            .environment(\.colorScheme, .dark)
            .foregroundStyle(.white)
            .frame(width: width, height: height)
            .frame(maxWidth: width == nil ? .infinity : nil)
    }

    private func liveCard<V: View>(_ background: Color, @ViewBuilder content: () -> V) -> some View {
        content()
            .frame(maxWidth: .infinity)
            .background(RoundedRectangle(cornerRadius: 24, style: .continuous).fill(background))
            .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
    }

    private func compact<L: View, T: View>(@ViewBuilder leading: () -> L, @ViewBuilder trailing: () -> T) -> some View {
        HStack(spacing: 0) {
            leading()
            Spacer(minLength: 20)
            trailing()
        }
        .padding(.horizontal, 14)
        .frame(width: 240, height: 36)
        .background(Capsule().fill(.black))
        .environment(\.colorScheme, .dark)
    }

    private func minimal<V: View>(@ViewBuilder content: () -> V) -> some View {
        content()
            .frame(width: 24, height: 24)
            .frame(width: 36, height: 36)
            .background(Circle().fill(.black))
            .environment(\.colorScheme, .dark)
    }

    private func expanded<L: View, T: View, C: View, B: View>(leading: L, trailing: T, center: C, bottom: B) -> some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                leading
                center
                Spacer(minLength: 0)
                trailing
            }
            bottom
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 18)
        .frame(maxWidth: .infinity)
        .background(RoundedRectangle(cornerRadius: 44, style: .continuous).fill(.black))
        .padding(.horizontal, 10)
        .environment(\.colorScheme, .dark)
    }
}

/// The Lock Screen's wallpaper on the board: night with the pond drawn faintly, and grain.
private struct LockWallpaper: View {
    var body: some View {
        ZStack(alignment: .topLeading) {
            Color(red: 29 / 255, green: 27 / 255, blue: 51 / 255)
            Image("pond")
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .foregroundStyle(.white.opacity(0.45))
                .frame(width: 370)
                .padding(.leading, 10)
                .padding(.top, 170)
            Grain()
        }
        .ignoresSafeArea()
    }
}

/// A notification as the system draws it: the app's icon, name and time; Reins supplies a plain title and body.
private struct MockNotification: View {
    var title: String
    var body_: String
    var when: String

    init(title: String, body: String, when: String) {
        self.title = title
        body_ = body
        self.when = when
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 8) {
                FixedGlyph(.shieldCheck, size: 13)
                    .foregroundStyle(.white)
                    .frame(width: 22, height: 22)
                    .background(RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color(red: 0.13, green: 0.1, blue: 0.3)))
                Text("Reins").font(.system(size: 13, weight: .semibold))
                Spacer()
                Text(when).font(.system(size: 12.5)).foregroundStyle(Color(red: 0.42, green: 0.42, blue: 0.47))
            }
            .padding(.bottom, 4)
            Text(title).font(.system(size: 15, weight: .semibold))
            Text(body_).font(.system(size: 14)).foregroundStyle(Color(red: 0.24, green: 0.23, blue: 0.27))
        }
        .foregroundStyle(Color(red: 22 / 255, green: 21 / 255, blue: 27 / 255))
        .padding(.horizontal, 14)
        .padding(.vertical, 13)
        .background(RoundedRectangle(cornerRadius: 24, style: .continuous).fill(Color(white: 0.96).opacity(0.88)))
    }
}

/// A Control Center tile as the system draws a control: its symbol in a disc (the tint only while on), title, state.
private struct MockControl: View {
    var symbol: String
    var tint: Color?
    var title: String
    var state: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Image(systemName: symbol)
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 30, height: 30)
                .background(Circle().fill(tint ?? .white.opacity(0.22)))
            Spacer(minLength: 0)
            Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(.white).lineLimit(1)
            Text(state).font(.system(size: 12, weight: .medium)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .padding(14)
        .frame(maxWidth: .infinity, minHeight: 92, maxHeight: 92, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 22, style: .continuous).fill(.white.opacity(0.14)))
    }
}
#endif
