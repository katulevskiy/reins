#if DEBUG
import SwiftUI
import WidgetKit

extension LiveActivityController {
    /// `-demoBypass`: a 15-minute bypass right after launch, so the Bypass Live Activity can be looked at.
    func debugLaunch(model: AppModel) {
        guard ProcessInfo.processInfo.arguments.contains("-demoBypass") else { return }
        Task { @MainActor in
            for _ in 0..<50 where model.session == .loading { try? await Task.sleep(for: .milliseconds(100)) }
            try? await Task.sleep(for: .seconds(1))
            try? await model.core.setAutopilotMode(connectionId: nil, mode: .bypass, minutes: 15)
            await model.refreshAutopilot()
        }
    }
}

/// Every widget and Live Activity drawn with the same views the widget extension uses, at their real sizes, from
/// the snapshot the app just wrote (launch with `-demo -widgetGallery <page>`; pages: waiting, autopilot, states,
/// lock, island). Adding widgets to the Home Screen cannot be scripted, so screenshots of this stand in for them.
struct WidgetGallery: View {
    @Environment(AppModel.self) private var model
    @State private var snapshot = Snapshot.load()
    @State private var now = Glance.now()

    static var requested: Bool { ProcessInfo.processInfo.arguments.contains("-widgetGallery") }

    static var page: String {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-widgetGallery"), i + 1 < args.count, !args[i + 1].hasPrefix("-") else { return "waiting" }
        return args[i + 1]
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                switch Self.page {
                case "autopilot": autopilot
                case "states": states
                case "lock": lock
                case "island": island
                default: waiting
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 8)
            .frame(maxWidth: .infinity)
        }
        .background(Palette.background)
        .task {
            while !Task.isCancelled {
                snapshot = Snapshot.load()
                now = Glance.now()
                try? await Task.sleep(for: .seconds(1))
            }
        }
    }

    // MARK: Pages

    @ViewBuilder private var waiting: some View {
        caption("Waiting · small, medium, large")
        HStack(spacing: 20) {
            plate(.systemSmall) { WaitingSmallView(snapshot: snapshot, now: now) }
            plate(.systemSmall) { AutopilotWidgetView(snapshot: snapshot, now: now, medium: false) }
        }
        plate(.systemMedium) { WaitingListView(snapshot: snapshot, now: now, rows: 3) }
        plate(.systemLarge) { WaitingListView(snapshot: snapshot, now: now, rows: 7) }
    }

    @ViewBuilder private var autopilot: some View {
        let bypass = with { $0.autopilotMode = "bypass"; $0.bypassUntil = now + 14 * 60 + 5; $0.anyBypassUntil = $0.bypassUntil }
        let lockdown = with { $0.autopilotMode = "lockdown"; $0.baseMode = "assisted"; $0.bypassUntil = nil; $0.anyBypassUntil = nil }
        caption("Autopilot · now, in a bypass, in Lockdown")
        plate(.systemMedium) { AutopilotWidgetView(snapshot: snapshot, now: now, medium: true) }
        HStack(spacing: 20) {
            plate(.systemSmall) { AutopilotWidgetView(snapshot: bypass, now: now, medium: false) }
            plate(.systemSmall) { AutopilotWidgetView(snapshot: lockdown, now: now, medium: false) }
        }
        plate(.systemMedium) { AutopilotWidgetView(snapshot: bypass, now: now, medium: true) }
        caption("Latest activity")
        plate(.systemMedium) { LatestActivityView(snapshot: snapshot, now: now, rows: 3) }
    }

    @ViewBuilder private var states: some View {
        let empty = with { $0.pending = [] }
        let out = Snapshot()
        caption("Nothing waits, signed out")
        HStack(spacing: 20) {
            plate(.systemSmall) { WaitingSmallView(snapshot: empty, now: now) }
            plate(.systemSmall) { WaitingSmallView(snapshot: out, now: now) }
        }
        plate(.systemMedium) { WaitingListView(snapshot: empty, now: now, rows: 3) }
        plate(.systemMedium) { AutopilotWidgetView(snapshot: out, now: now, medium: true) }
        caption("Accented rendering")
        HStack(spacing: 20) {
            plate(.systemSmall, accented: true) { WaitingSmallView(snapshot: snapshot, now: now) }
            plate(.systemSmall, accented: true) { AutopilotWidgetView(snapshot: snapshot, now: now, medium: false) }
        }
    }

    @ViewBuilder private var lock: some View {
        caption("Lock Screen widgets")
        HStack(spacing: 14) {
            accessory(width: 76, height: 76) { WaitingCircularView(snapshot: snapshot, now: now) }
            accessory(width: 172, height: 76) { WaitingRectangularView(snapshot: snapshot, now: now) }
        }
        accessory(width: 300, height: 26) { WaitingInlineView(snapshot: snapshot, now: now) }
        caption("Live Activities on the Lock Screen")
        if let state = approvalState {
            liveCard { ApprovalLiveViews(state: state, stale: false).lockScreen }
        }
        liveCard { BypassLiveViews(state: bypassState, stale: false).lockScreen }
        liveCard { ModelDownloadLiveViews(state: .init(downloaded: 141_000_000, total: 370_000_000, finished: false, failed: false)).lockScreen }
    }

    @ViewBuilder private var island: some View {
        caption("Dynamic Island · compact and minimal")
        if let state = approvalState {
            let v = ApprovalLiveViews(state: state, stale: false)
            compact { v.compactLeading } trailing: { v.compactTrailing }
            let b = BypassLiveViews(state: bypassState, stale: false)
            compact { b.compactLeading } trailing: { b.compactTrailing }
            HStack(spacing: 14) {
                minimal { v.minimal }
                minimal { b.minimal }
                minimal { ModelDownloadLiveViews(state: downloadState).minimal }
            }
            caption("Expanded")
            expanded(leading: v.expandedLeading, trailing: v.expandedTrailing, center: v.expandedCenter, bottom: v.expandedBottom)
            expanded(leading: b.expandedLeading, trailing: b.expandedTrailing, center: b.expandedCenter, bottom: b.expandedBottom)
            let d = ModelDownloadLiveViews(state: downloadState)
            expanded(leading: d.expandedLeading, trailing: d.expandedTrailing, center: d.expandedCenter, bottom: d.expandedBottom)
        }
    }

    // MARK: Sample states

    private func with(_ change: (inout Snapshot) -> Void) -> Snapshot {
        var s = snapshot
        change(&s)
        return s
    }

    private var approvalState: ApprovalActivityAttributes.ContentState? {
        Glance.approvalState(snapshot.pending, now: now) ?? Glance.approvalState(Glance.sample(now: now).pending, now: now)
    }

    private var bypassState: BypassActivityAttributes.ContentState {
        let until = Date(timeIntervalSince1970: TimeInterval(now + 14 * 60 + 5))
        return .init(until: until, startedAt: until.addingTimeInterval(-15 * 60), scope: "every AI", approvedCount: 3)
    }

    private var downloadState: ModelDownloadActivityAttributes.ContentState {
        .init(downloaded: 141_000_000, total: 370_000_000, finished: false, failed: false)
    }

    // MARK: Frames

    private func caption(_ text: String) -> some View {
        Text(text).font(RFont.fixedSans(12, .semibold)).foregroundStyle(Palette.secondary).padding(.top, 6)
    }

    /// A Home Screen widget at the 6.9" iPhone's size, on its plate.
    private func plate<V: View>(_ family: WidgetFamily, accented: Bool = false, @ViewBuilder content: () -> V) -> some View {
        let size: CGSize = switch family {
        case .systemSmall: CGSize(width: 170, height: 170)
        case .systemMedium: CGSize(width: 364, height: 170)
        default: CGSize(width: 364, height: 382)
        }
        return content()
            .padding(16)
            .frame(width: size.width, height: size.height)
            .environment(\.widgetRenderingMode, accented ? .accented : .fullColor)
            .environment(\.colorScheme, accented ? .dark : colorSchemeFromTraits)
            .background(
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .fill(accented ? AnyShapeStyle(LinearGradient(colors: [Color(red: 0.12, green: 0.2, blue: 0.4), Color(red: 0.2, green: 0.12, blue: 0.35)], startPoint: .top, endPoint: .bottom)) : AnyShapeStyle(GlanceStyle.background))
            )
            .saturation(accented ? 0 : 1)
            .shadow(color: .black.opacity(0.06), radius: 8, y: 2)
    }

    @Environment(\.colorScheme) private var colorSchemeFromTraits

    /// A Lock Screen accessory: vibrant, white on the wallpaper.
    private func accessory<V: View>(width: CGFloat, height: CGFloat, @ViewBuilder content: () -> V) -> some View {
        content()
            .environment(\.widgetRenderingMode, .vibrant)
            .environment(\.colorScheme, .dark)
            .foregroundStyle(.white)
            .frame(width: width, height: height)
            .padding(8)
            .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(Color(red: 0.18, green: 0.2, blue: 0.3)))
    }

    private func liveCard<V: View>(@ViewBuilder content: () -> V) -> some View {
        content()
            .frame(width: 400)
            .background(RoundedRectangle(cornerRadius: 24, style: .continuous).fill(GlanceStyle.liveBackground))
            .shadow(color: .black.opacity(0.08), radius: 10, y: 2)
    }

    private func compact<L: View, T: View>(@ViewBuilder leading: () -> L, @ViewBuilder trailing: () -> T) -> some View {
        HStack(spacing: 0) {
            leading().frame(width: 40, alignment: .leading)
            Spacer(minLength: 126)
            trailing().frame(width: 52, alignment: .trailing)
        }
        .padding(.horizontal, 14)
        .frame(width: 250, height: 37)
        .background(Capsule().fill(.black))
        .environment(\.colorScheme, .dark)
        .frame(maxWidth: .infinity)
    }

    private func minimal<V: View>(@ViewBuilder content: () -> V) -> some View {
        content()
            .frame(width: 24, height: 24)
            .frame(width: 37, height: 37)
            .background(Circle().fill(.black))
            .environment(\.colorScheme, .dark)
    }

    private func expanded<L: View, T: View, C: View, B: View>(leading: L, trailing: T, center: C, bottom: B) -> some View {
        VStack(spacing: 0) {
            HStack(alignment: .top, spacing: 8) {
                leading
                center
                trailing
            }
            bottom
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 14)
        .frame(width: 408)
        .background(RoundedRectangle(cornerRadius: 44, style: .continuous).fill(.black))
        .environment(\.colorScheme, .dark)
        .frame(maxWidth: .infinity)
    }
}
#endif
