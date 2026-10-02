import ActivityKit
import SwiftUI
import WidgetKit

// The Live Activities' configurations: the Lock Screen view and the Dynamic Island's regions, all drawn by the views
// in Shared/Widgets/GlanceViews.swift. The app starts, updates and ends them (`LiveActivityController`).

/// Requests waiting: the newest with its countdown, Deny and Review.
struct ApprovalLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ApprovalActivityAttributes.self) { context in
            ApprovalLiveViews(state: context.state, stale: context.isStale).lockScreen
                .activityBackgroundTint(GlanceStyle.liveBackground)
                .activitySystemActionForegroundColor(Palette.accent)
                .widgetURL(DeepLink.item(kind: context.state.kind, id: context.state.itemId).url)
        } dynamicIsland: { context in
            let v = ApprovalLiveViews(state: context.state, stale: context.isStale)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { v.expandedLeading }
                DynamicIslandExpandedRegion(.trailing) { v.expandedTrailing }
                DynamicIslandExpandedRegion(.center) { v.expandedCenter }
                DynamicIslandExpandedRegion(.bottom) { if !context.isStale { v.expandedBottom } }
            } compactLeading: {
                v.compactLeading
            } compactTrailing: {
                v.compactTrailing
            } minimal: {
                v.minimal
            }
            .widgetURL(v.link)
            .keylineTint(Palette.accent)
        }
    }
}

/// A bypass that runs: red, the countdown to its end, Stop.
struct BypassLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: BypassActivityAttributes.self) { context in
            BypassLiveViews(state: context.state, stale: context.isStale).lockScreen
                .activityBackgroundTint(GlanceStyle.liveBackground)
                .activitySystemActionForegroundColor(Palette.danger)
                .widgetURL(DeepLink.autopilot.url)
        } dynamicIsland: { context in
            let v = BypassLiveViews(state: context.state, stale: context.isStale)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { v.expandedLeading }
                DynamicIslandExpandedRegion(.trailing) { v.expandedTrailing }
                DynamicIslandExpandedRegion(.center) { v.expandedCenter }
                DynamicIslandExpandedRegion(.bottom) { if !v.ended { v.expandedBottom } }
            } compactLeading: {
                v.compactLeading
            } compactTrailing: {
                v.compactTrailing
            } minimal: {
                v.minimal
            }
            .widgetURL(DeepLink.autopilot.url)
            .keylineTint(Palette.danger)
        }
    }
}

/// Autopilot's model coming down.
struct ModelDownloadLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ModelDownloadActivityAttributes.self) { context in
            ModelDownloadLiveViews(state: context.state).lockScreen
                .activityBackgroundTint(GlanceStyle.liveBackground)
                .activitySystemActionForegroundColor(Palette.accent)
                .widgetURL(DeepLink.autopilot.url)
        } dynamicIsland: { context in
            let v = ModelDownloadLiveViews(state: context.state)
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { v.expandedLeading }
                DynamicIslandExpandedRegion(.trailing) { v.expandedTrailing }
                DynamicIslandExpandedRegion(.center) { v.expandedCenter }
                DynamicIslandExpandedRegion(.bottom) { v.expandedBottom }
            } compactLeading: {
                v.compactLeading
            } compactTrailing: {
                v.compactTrailing
            } minimal: {
                v.minimal
            }
            .widgetURL(DeepLink.autopilot.url)
            .keylineTint(Palette.accent)
        }
    }
}
