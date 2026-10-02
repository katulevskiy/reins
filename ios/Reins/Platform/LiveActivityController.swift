import ActivityKit
import OSLog
import UIKit

/// Keeps the Live Activities in step with the model: one for the requests that wait, one while a bypass runs (the
/// iOS side of the Android app's ongoing bypass notification, with Stop). Installed once at launch; it wraps the
/// model's `onPendingChanged` / `onAutopilotChanged` hooks, keeping whatever else was set on them.
///
/// The system only lets an app start a Live Activity while it is in front; updating and ending work from the
/// background too (a push, a background refresh). An activity's `staleDate` is when its window closes, so one the
/// app could not end in time shows itself as over.
@MainActor
final class LiveActivityController {
    static let shared = LiveActivityController()

    private weak var model: AppModel?
    /// Activity changes run one after another, so two quick refreshes never start two activities.
    private var queue: Task<Void, Never>?
    /// While the app runs: the next moment something runs out (an answer window, a bypass).
    private var timer: Task<Void, Never>?

    func install(model: AppModel) {
        self.model = model
        let pending = model.onPendingChanged
        model.onPendingChanged = { [weak self] list in
            pending?(list)
            self?.sync()
        }
        let autopilot = model.onAutopilotChanged
        model.onAutopilotChanged = { [weak self] settings in
            autopilot?(settings)
            self?.sync()
        }
        #if DEBUG
        debugLaunch(model: model)
        #endif
    }

    static var enabled: Bool { ActivityAuthorizationInfo().areActivitiesEnabled }

    /// Starting needs the app in front; `inactive` counts (the moment the scene comes up or Control Center is open).
    static var canStart: Bool { enabled && UIApplication.shared.applicationState != .background }

    /// Re-reads the model and brings both activities in line.
    func sync() {
        guard let model else { return }
        let now = Glance.now()
        let signedIn: Bool = if case .signedIn = model.session { true } else { false }
        let approval = signedIn ? Glance.approvalState(model.pending.map(\.snapshotItem), now: now) : nil
        let previous = Activity<BypassActivityAttributes>.activities.first?.content.state
        let bypass: BypassActivityAttributes.ContentState? = if signedIn, let s = model.autopilot {
            Glance.bypassState(
                global: s.bypassUntil,
                connections: s.connections.map { c in (untrusted(model.connection(c.connectionId)?.label ?? "An AI"), c.bypassUntil) },
                previous: previous,
                approvedSince: { start in Self.approvedDuringBypass(model.activity, since: start) },
                now: now
            )
        } else {
            nil
        }
        Self.log.debug("sync: requests \(approval?.count ?? 0), bypass until \(bypass?.until.description ?? "-", privacy: .public), mode \(model.autopilot.map { "\($0.mode)" } ?? "-", privacy: .public)")
        enqueue {
            await Self.apply(approval)
            await Self.apply(bypass)
        }
        schedule(now: now, approval: approval, bypass: bypass)
    }

    /// Requests a bypass approved since it started (the count on the activity).
    nonisolated static func approvedDuringBypass(_ activity: [ActivityEntry], since start: Date) -> Int {
        let from = Int64(start.timeIntervalSince1970)
        return activity.filter { $0.decidedBy == "bypass" && $0.at >= from && !["denied", "failed", "expired", "error"].contains($0.outcome) }.count
    }

    private func enqueue(_ work: @escaping @MainActor () async -> Void) {
        let previous = queue
        queue = Task { @MainActor in
            await previous?.value
            await work()
        }
    }

    /// Wakes up when the shown request's window closes or the bypass ends, while the app still runs.
    private func schedule(now: Int64, approval: ApprovalActivityAttributes.ContentState?, bypass: BypassActivityAttributes.ContentState?) {
        timer?.cancel()
        let next = [approval?.expiresAt, bypass?.until].compactMap { $0 }.min()
        guard let next else { return }
        let delay = max(0.5, next.timeIntervalSince1970 - TimeInterval(now) + 0.5)
        timer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled, let self else { return }
            if let b = bypass, b.until.timeIntervalSince1970 <= Date().timeIntervalSince1970 + 1 {
                // The bypass is over: read Autopilot again, which syncs.
                await self.model?.refreshAutopilot()
            } else {
                self.sync()
            }
        }
    }

    // MARK: Applying

    private static func apply(_ state: ApprovalActivityAttributes.ContentState?) async {
        await apply(state, attributes: ApprovalActivityAttributes(), stale: state?.expiresAt, relevance: 100)
    }

    private static func apply(_ state: BypassActivityAttributes.ContentState?) async {
        await apply(state, attributes: BypassActivityAttributes(), stale: state?.until, relevance: 90)
    }

    /// Ends the activities of a kind when `state` is nil, else updates the one that runs (ending any extra) or starts one.
    private static func apply<A: ActivityAttributes>(_ state: A.ContentState?, attributes: A, stale: Date?, relevance: Double) async where A.ContentState: Equatable {
        let running = Activity<A>.activities.filter { $0.activityState == .active || $0.activityState == .stale }
        guard let state else {
            for activity in running { await activity.end(nil, dismissalPolicy: .immediate) }
            return
        }
        let content = ActivityContent(state: state, staleDate: stale, relevanceScore: relevance)
        if let current = running.first {
            if current.content.state != state || current.activityState == .stale { await current.update(content) }
            for extra in running.dropFirst() { await extra.end(nil, dismissalPolicy: .immediate) }
        } else if canStart {
            do {
                _ = try Activity.request(attributes: attributes, content: content, pushType: nil)
            } catch {
                log.error("Live Activity \(String(describing: A.self), privacy: .public) not started: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    private static let log = Logger(subsystem: "dev.rewarden.ios", category: "LiveActivities")
}

/// Autopilot's model download on the Lock Screen and in the Dynamic Island, for whoever runs the download (the
/// Autopilot screen). Call `start` when it begins (with the app in front), `update` as bytes arrive (cheap to call
/// often: it only passes on whole-percent changes), and `end` when it finished or failed.
@MainActor
enum ModelDownloadActivity {
    private static var lastPercent = -1

    static func start(total: Int64) {
        lastPercent = -1
        update(downloaded: 0, total: total)
    }

    static func update(downloaded: Int64, total: Int64) {
        let state = ModelDownloadActivityAttributes.ContentState(downloaded: downloaded, total: total, finished: false, failed: false)
        let percent = Int(Glance.downloadFraction(state) * 100)
        guard percent != lastPercent else { return }
        lastPercent = percent
        let content = ActivityContent(state: state, staleDate: nil)
        Task {
            if let current = running.first {
                await current.update(content)
            } else if LiveActivityController.canStart {
                _ = try? Activity.request(attributes: ModelDownloadActivityAttributes(), content: content, pushType: nil)
            }
        }
    }

    /// The last state stays a little on the Lock Screen: "ready" for a minute, a failure for ten.
    static func end(failed: Bool = false) {
        lastPercent = -1
        let last = running.first?.content.state
        let state = ModelDownloadActivityAttributes.ContentState(
            downloaded: last?.downloaded ?? 0,
            total: last?.total ?? 0,
            finished: !failed,
            failed: failed
        )
        let content = ActivityContent(state: state, staleDate: nil)
        Task {
            for activity in running {
                await activity.end(content, dismissalPolicy: .after(Date().addingTimeInterval(failed ? 600 : 60)))
            }
        }
    }

    private static var running: [Activity<ModelDownloadActivityAttributes>] {
        Activity<ModelDownloadActivityAttributes>.activities.filter { $0.activityState == .active || $0.activityState == .stale }
    }
}
