import Foundation
import WidgetKit

/// Asks WidgetKit to reload the widgets and controls, at most once every `spacing`. Each reload wakes the widget
/// extension, so a burst of snapshot changes (a refresh, an answer, the next refresh) is folded into one: the first
/// request goes at once, later ones within the spacing make a single reload at its end.
final class WidgetReloads: @unchecked Sendable {
    static let shared = WidgetReloads()

    private let spacing: TimeInterval
    private let queue: DispatchQueue
    private let reload: @Sendable () -> Void
    private let lock = NSLock()
    private var last = Date.distantPast
    private var pending = false

    init(
        spacing: TimeInterval = 2,
        queue: DispatchQueue = DispatchQueue(label: "com.reins2fa.app.widget-reloads"),
        reload: @escaping @Sendable () -> Void = {
            WidgetCenter.shared.reloadAllTimelines()
            ControlCenter.shared.reloadAllControls()
        }
    ) {
        self.spacing = spacing
        self.queue = queue
        self.reload = reload
    }

    func request() {
        lock.lock()
        let wait = last.addingTimeInterval(spacing).timeIntervalSinceNow
        if wait <= 0 {
            last = Date()
            lock.unlock()
            reload()
            return
        }
        guard !pending else {
            lock.unlock()
            return
        }
        pending = true
        lock.unlock()
        queue.asyncAfter(deadline: .now() + wait) { [self] in
            lock.lock()
            pending = false
            last = Date()
            lock.unlock()
            reload()
        }
    }
}
