import ActivityKit
import SwiftUI
import WidgetKit

// The Live Activities' configurations: the Lock Screen view and the Dynamic Island's regions, all drawn by the views
// in Shared/Widgets/GlanceViews.swift. The app starts, updates and ends them (`LiveActivityController`).

/// Requests waiting: the newest with its countdown, Deny and Review.
struct ApprovalLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ApprovalActivityAttributes.self) { context in
            let v = ApprovalLiveViews(state: context.state, stale: context.isStale)
            v.lockScreen
                .activityBackgroundTint(v.background)
                .activitySystemActionForegroundColor(context.isStale ? Palette.ink : Palette.onField)
                .widgetURL(v.link)
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
            .keylineTint(v.field.fill)
        }
    }
}

/// A bypass that runs: a red field, the time left, Stop. An Overnight the same, in its violet.
struct BypassLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: BypassActivityAttributes.self) { context in
            let v = BypassLiveViews(state: context.state, stale: context.isStale)
            v.lockScreen
                .activityBackgroundTint(v.background)
                .activitySystemActionForegroundColor(v.ended ? Palette.ink : Palette.onField)
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
            .keylineTint(v.tint)
        }
    }
}

/// Autopilot's model coming down.
struct ModelDownloadLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ModelDownloadActivityAttributes.self) { context in
            ModelDownloadLiveViews(state: context.state).lockScreen
                .activityBackgroundTint(GlanceStyle.liveBackground)
                .activitySystemActionForegroundColor(Palette.violet)
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
            .keylineTint(Palette.violet)
        }
    }
}
