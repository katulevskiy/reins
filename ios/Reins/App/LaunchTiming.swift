#if DEBUG
import Darwin
import Foundation
import os

/// Debug builds log how long launch takes, counted from the process starting: to the first `body` of the window's
/// content, to its first frame, and to the core being open. Each moment is logged once, and is also a signpost event
/// for Instruments.
enum LaunchTiming {
    private static let log = Logger(subsystem: "com.reins2fa.app", category: "launch")
    private static let signposter = OSSignposter(subsystem: "com.reins2fa.app", category: "launch")
    private static let marked = OSAllocatedUnfairLock<Set<String>>(initialState: [])

    static func mark(_ moment: StaticString) {
        let name = "\(moment)"
        guard marked.withLock({ $0.insert(name).inserted }) else { return }
        let ms = sinceProcessStart() * 1000
        signposter.emitEvent(moment)
        log.notice("\(name, privacy: .public): \(ms, format: .fixed(precision: 1)) ms after process start")
    }

    /// Seconds since the kernel started this process (before `main`, dyld and static initialisers).
    private static func sinceProcessStart() -> TimeInterval {
        var info = kinfo_proc()
        var size = MemoryLayout<kinfo_proc>.stride
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_PID, getpid()]
        guard sysctl(&mib, u_int(mib.count), &info, &size, nil, 0) == 0 else { return 0 }
        let start = info.kp_proc.p_un.__p_starttime
        return Date().timeIntervalSince1970 - (TimeInterval(start.tv_sec) + TimeInterval(start.tv_usec) / 1_000_000)
    }
}
#endif
