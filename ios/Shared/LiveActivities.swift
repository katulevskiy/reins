import ActivityKit
import Foundation

/// Requests waiting for the user: on the Lock Screen and in the Dynamic Island while the answer window is open.
/// Started by the app (see `LiveActivityController`), drawn by the widget extension.
struct ApprovalActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        /// The newest waiting item.
        var itemId: String
        var kind: Snapshot.Item.Kind
        var title: String
        var subtitle: String
        var connection: String
        var service: String
        /// How many items wait in all.
        var count: Int
        /// When the newest item's answer window closes (the countdown).
        var expiresAt: Date
        /// When it arrived (the countdown ring's start).
        var createdAt: Date
        var suggestion: String?
    }
}

/// A bypass that runs: what it covers and when it ends, with Stop.
struct BypassActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var until: Date
        var startedAt: Date
        /// "Every AI" or a connection's label.
        var scope: String
        /// Requests approved during this bypass so far.
        var approvedCount: Int
    }
}

/// Autopilot's model coming down (about 370 MB).
struct ModelDownloadActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var downloaded: Int64
        var total: Int64
        var finished: Bool
        var failed: Bool
    }
}
