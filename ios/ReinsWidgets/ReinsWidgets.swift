import SwiftUI
import WidgetKit

/// Placeholder bundle until the widgets, controls and Live Activities land.
@main
struct ReinsWidgetBundle: WidgetBundle {
    var body: some Widget {
        WaitingWidget()
    }
}

struct WaitingWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "waiting", provider: Provider()) { entry in
            Text("\(entry.snapshot.waiting().count) waiting")
                .font(RFont.fixedSans(15, .semibold))
                .containerBackground(Palette.background, for: .widget)
        }
        .configurationDisplayName("Waiting")
        .description("Requests that wait for you.")
    }

    struct Entry: TimelineEntry {
        var date: Date
        var snapshot: Snapshot
    }

    struct Provider: TimelineProvider {
        func placeholder(in context: Context) -> Entry { Entry(date: .now, snapshot: Snapshot()) }
        func getSnapshot(in context: Context, completion: @escaping (Entry) -> Void) { completion(Entry(date: .now, snapshot: .load())) }
        func getTimeline(in context: Context, completion: @escaping (Timeline<Entry>) -> Void) {
            completion(Timeline(entries: [Entry(date: .now, snapshot: .load())], policy: .never))
        }
    }
}
