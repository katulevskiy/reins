import SwiftUI
import WidgetKit

/// Everything the widget extension draws: Home Screen and Lock Screen widgets, Live Activities, Control Center
/// controls. None of it runs the core: widgets read the snapshot the app writes, and buttons run App Intents in the
/// app's process (see `ReinsIntents.swift`).
@main
struct ReinsWidgetBundle: WidgetBundle {
    var body: some Widget {
        WaitingWidget()
        AutopilotWidget()
        LatestActivityWidget()
        ApprovalLiveActivity()
        BypassLiveActivity()
        ModelDownloadLiveActivity()
        LockdownControl()
        WaitingControl()
        StopBypassControl()
    }
}

// MARK: Timeline

struct SnapshotEntry: TimelineEntry {
    var date: Date
    var snapshot: Snapshot

    var now: Int64 { Int64(date.timeIntervalSince1970) }
}

/// One entry now, and one at every moment something runs out by itself (an answer window, a bypass). New requests
/// and answers come with a new snapshot: the app (and the notification extension) reload every timeline then.
struct SnapshotProvider: TimelineProvider {
    func placeholder(in context: Context) -> SnapshotEntry {
        SnapshotEntry(date: .now, snapshot: Glance.sample(now: Glance.now()))
    }

    func getSnapshot(in context: Context, completion: @escaping (SnapshotEntry) -> Void) {
        let saved = Snapshot.load()
        // The widget picker shows a filled example rather than "Sign in" on a phone that has not signed in yet.
        let snapshot = context.isPreview && !saved.signedIn ? Glance.sample(now: Glance.now()) : saved
        completion(SnapshotEntry(date: .now, snapshot: snapshot))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<SnapshotEntry>) -> Void) {
        let snapshot = Snapshot.load()
        let entries = Glance.timelineDates(snapshot, now: Glance.now()).map {
            SnapshotEntry(date: Date(timeIntervalSince1970: TimeInterval($0)), snapshot: snapshot)
        }
        completion(Timeline(entries: entries, policy: .never))
    }
}

extension View {
    /// The widget plate on the Home Screen; Lock Screen accessories have none of their own.
    @ViewBuilder func glanceBackground(_ family: WidgetFamily) -> some View {
        switch family {
        case .accessoryCircular, .accessoryRectangular, .accessoryInline:
            containerBackground(Color.clear, for: .widget)
        default:
            containerBackground(GlanceStyle.background, for: .widget)
        }
    }
}

// MARK: Waiting

struct WaitingWidget: Widget {
    static let kind = "waiting"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Self.kind, provider: SnapshotProvider()) { entry in
            WaitingWidgetView(entry: entry)
        }
        .configurationDisplayName("Waiting")
        .description("Requests that wait for you, with the time left to answer.")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge, .accessoryCircular, .accessoryRectangular, .accessoryInline])
    }
}

struct WaitingWidgetView: View {
    var entry: SnapshotEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        Group {
            switch family {
            case .systemSmall: WaitingSmallView(snapshot: entry.snapshot, now: entry.now)
            case .systemLarge, .systemExtraLarge: WaitingListView(snapshot: entry.snapshot, now: entry.now, rows: 7)
            case .accessoryCircular: WaitingCircularView(snapshot: entry.snapshot, now: entry.now)
            case .accessoryRectangular: WaitingRectangularView(snapshot: entry.snapshot, now: entry.now)
            case .accessoryInline: WaitingInlineView(snapshot: entry.snapshot, now: entry.now)
            default: WaitingListView(snapshot: entry.snapshot, now: entry.now, rows: 3)
            }
        }
        .widgetURL(Glance.waitingLink(entry.snapshot, now: entry.now))
        .glanceBackground(family)
    }
}

// MARK: Autopilot

struct AutopilotWidget: Widget {
    static let kind = "autopilot"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Self.kind, provider: SnapshotProvider()) { entry in
            AutopilotWidgetEntryView(entry: entry)
        }
        .configurationDisplayName("Autopilot")
        .description("The mode in force, with Lockdown and Stop bypass at hand.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}

struct AutopilotWidgetEntryView: View {
    var entry: SnapshotEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        AutopilotWidgetView(snapshot: entry.snapshot, now: entry.now, medium: family != .systemSmall)
            .widgetURL(DeepLink.autopilot.url)
            .glanceBackground(family)
    }
}

// MARK: Latest activity

struct LatestActivityWidget: Widget {
    static let kind = "latest"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: Self.kind, provider: SnapshotProvider()) { entry in
            LatestActivityEntryView(entry: entry)
        }
        .configurationDisplayName("Latest activity")
        .description("What your AIs did last, and whether it went through.")
        .supportedFamilies([.systemMedium, .systemLarge])
    }
}

struct LatestActivityEntryView: View {
    var entry: SnapshotEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        LatestActivityView(snapshot: entry.snapshot, now: entry.now, rows: family == .systemLarge ? 6 : 3)
            .widgetURL(DeepLink.home.url)
            .glanceBackground(family)
    }
}
