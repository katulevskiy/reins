import SwiftUI

// A running Overnight, Lockdown or Bypass collapses as Trust scrolls, in two springy steps (the Running board, C):
// full (the big clock, what still asks, Stop now) → short (the clock and Stop on one line) → mini (only the island's
// row: the clock in the left ear, Stop in the right). The full field is the page's first row and scrolls with it; the
// short and mini bars are pinned over the page, so the list's insets never change while it scrolls (a changing inset
// would feed back into the offset). Under Reduce Motion each step cross-fades instead of springing.

/// The three states of a running mode's field.
enum RunningStage: Int, Comparable {
    case full, short, mini

    static func < (a: RunningStage, b: RunningStage) -> Bool { a.rawValue < b.rawValue }

    /// The stage for a scroll offset (0 at rest, growing as the page moves up). `fieldBottom` is where the full field
    /// ends at rest, from the top of the screen; `shortHeight` and `miniHeight` are the pinned bars' heights. Short
    /// takes over when the full field's visible part is no taller than the short bar; mini once the page has moved on
    /// by the short bar's height and `linger` more, so the short bar is seen before it shrinks.
    nonisolated static func at(offset: CGFloat, fieldBottom: CGFloat, shortHeight: CGFloat, miniHeight: CGFloat, linger: CGFloat = 40) -> RunningStage {
        guard fieldBottom > 0 else { return .full }
        let toShort = fieldBottom - shortHeight
        let toMini = toShort + max(shortHeight - miniHeight, 0) + linger
        if offset >= toMini { return .mini }
        if offset >= toShort { return .short }
        return .full
    }

    /// The field's state as its accessibility value (VoiceOver reads it; UI tests check it on "runningField").
    var value: String {
        switch self {
        case .full: "open"
        case .short: "short"
        case .mini: "compact"
        }
    }
}

/// How far Trust has scrolled, and the full field's measured bottom. Only the pinned bars read the stage, so scrolling
/// redraws them, not the page.
@Observable @MainActor
final class TrustFold {
    private(set) var stage: RunningStage = .full
    /// Where the full field ends at rest, from the top of the screen.
    var fieldBottom: CGFloat = 0
    @ObservationIgnored var shortHeight: CGFloat = 118
    @ObservationIgnored var miniHeight: CGFloat = 60

    func scroll(to y: CGFloat) {
        let next = RunningStage.at(offset: y, fieldBottom: fieldBottom, shortHeight: shortHeight, miniHeight: miniHeight)
        if next != stage { stage = next }
    }

    func reset() { if stage != .full { stage = .full } }
}

/// Where the ears go (the SE and Largest text boards): beside the Dynamic Island on a phone that has one (a top inset
/// of 47 pt or more), else in a row under the status bar, which stays. Accessibility text sizes always take the row
/// under the status bar: the words no longer fit beside the island.
struct EarsPlacement: Equatable {
    /// The status bar's height (the top safe-area inset).
    var safeTop: CGFloat
    var accessibility: Bool

    nonisolated static let islandInset: CGFloat = 47

    var beside: Bool { safeTop >= Self.islandInset && !accessibility }
    /// Hide the status bar: the ears take its place.
    var hidesStatusBar: Bool { beside }
    /// The ears row's top, from the top of the screen.
    var top: CGFloat { beside ? 12 : safeTop + (accessibility ? 6 : 0) }
    /// In from the screen's edges: clear of the corner curve beside the island.
    var side: CGFloat { beside ? 30 : 16 }
    /// Never reaching the island.
    var maxWidth: CGFloat { beside ? 95 : accessibility ? 240 : 160 }
    var height: CGFloat { accessibility ? 44 : 36 }
    /// Where the field's own content starts.
    var contentTop: CGFloat { top + height + (beside ? 16 : 10) }
    var miniHeight: CGFloat { top + height + 12 }
    var shortHeight: CGFloat { contentTop + 54 }
}
