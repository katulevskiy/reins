import Foundation

/// Which asks reach the owner. The core's list (`pending`) holds only what waits for the owner: a bypass, Lockdown,
/// Overnight, Autopilot in Auto and standing grants decide first, and what they decide never enters it. On top of that
/// list, a request a mode or Autopilot decided meanwhile (`AppNotifier.autoDecided`) closes its ask if one is open, and
/// stays out of the list even when a read made a moment before the decision still names it.
enum AskGate {
    /// How many automatic decisions are remembered against a list read before them (only the last moments matter).
    static let remembered = 64

    /// The owner's list as read from the core, without what was decided automatically since.
    static func waiting(_ read: [PendingItem], decided: [String]) -> [PendingItem] {
        guard !decided.isEmpty else { return read }
        let gone = Set(decided)
        return read.filter { !gone.contains($0.id) }
    }

    /// The item to pop up by itself: the first one that waits for the owner and has not popped up before.
    static func next(_ waiting: [PendingItem], presented: Set<String>) -> PendingItem? {
        waiting.first { !presented.contains($0.id) }
    }

    /// Whether the sheet open on `open` is the ask a mode or Autopilot just decided (`decided`): it closes.
    static func closes(_ open: SheetTarget?, decided: String) -> Bool {
        guard let open else { return false }
        return open.id == decided
    }

    /// `decided` with `id` added, the oldest forgotten past `remembered`.
    static func remember(_ id: String, in decided: [String]) -> [String] {
        Array((decided.filter { $0 != id } + [id]).suffix(remembered))
    }
}
