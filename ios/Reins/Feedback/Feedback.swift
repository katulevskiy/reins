import SwiftUI

/// The touch vocabulary (the Android app's `Haptic`). Names describe the moment, not the vibration; the haptic
/// engine decides how each one is felt (Core Haptics patterns, or nothing on devices without a Taptic Engine).
enum Haptic: CaseIterable {
    /// The faintest detent: slider steps.
    case tick
    /// A choice was made: a chip, tab or list row.
    case select
    /// A switch turned on / off: a rising pair, a falling pair.
    case toggleOn, toggleOff
    /// A user action was committed: approve, save, refresh.
    case confirm
    /// Something finished well: a permission given, an AI connected, a file released.
    case success
    /// Something needs the user: a request arrived.
    case attention
    /// Something failed.
    case error
    /// A weighty, destructive step: revoke, delete, disconnect.
    case heavy
    /// A small pop that decays: Autopilot answered for you.
    case pop
    /// A refusal: a firm click that falls away (the door shuts).
    case deny
    /// Every safeguard switched off: a rising surge ending in a crack.
    case surge
    /// Back to safety: a very short, sharp double tick.
    case zip
    /// Autopilot switched up: a ragged crackle that builds to a click.
    case lightning
}

/// Sound cues, `Resources/Sounds/fx_<name>.wav` (the Android app's `Cue`, Zeron's sound set). The chimes are also the
/// notification sounds.
enum Cue: CaseIterable {
    // Chimes.
    case request, attention, done
    // Longer action cues.
    case send, uploadReady, reconnected, undo
    // Interface.
    case tap, select, toggleOn, toggleOff, open, close
    /// A slider detent; `step` climbs the pentatonic scale.
    case detent
    case delete, copy, error, refresh
    // Autopilot.
    /// Bypass on: a bright swell with sparkle.
    case surge
    /// Bypass off: a quick airy flick, falling.
    case zip
    /// A mode with more autonomy: a crackle with a bloom.
    case fastOn
    /// A mode with less autonomy: the charge draining away.
    case fastOff
    /// Autopilot approved something: the approve cue, far quieter.
    case autoApproved
    /// Autopilot denied something: the deny cue, far quieter.
    case autoDenied
    /// Lockdown: the attention chime, as an answer to the switch.
    case lockdown
}

/// Reins's moments, each a haptic and a sound designed as a pair (either may be absent). Screens and the model play
/// these: `feedback.play(.approved)`.
enum FeedbackEvent: CaseIterable {
    case tap, selection, toggleOn, toggleOff, detent, open, close
    case requestArrived, approved, denied, grantCreated, connected, uploadApproved, revoked, error, alert
    case refresh, copied, undo
    case autoApproved, autoDenied, autopilotOn, autopilotOff, bypassOn, bypassOff, lockdownOn, lockdownOff

    var haptic: Haptic? {
        switch self {
        case .tap: .select
        case .selection: .select
        case .toggleOn: .toggleOn
        case .toggleOff: .toggleOff
        case .detent: .tick
        case .open, .close: nil
        case .requestArrived: .attention
        case .approved: .confirm
        case .denied: .deny
        case .grantCreated, .connected, .uploadApproved: .success
        case .revoked: .heavy
        case .error: .error
        case .alert: .attention
        case .refresh, .copied: .confirm
        case .undo: .select
        case .autoApproved: .pop
        case .autoDenied: .tick
        case .autopilotOn: .lightning
        case .autopilotOff: .toggleOff
        case .bypassOn: .surge
        case .bypassOff: .zip
        case .lockdownOn: .heavy
        case .lockdownOff: .toggleOn
        }
    }

    var cue: Cue? {
        switch self {
        case .tap: .tap
        case .selection: .select
        case .toggleOn: .toggleOn
        case .toggleOff: .toggleOff
        case .detent: .detent
        case .open: .open
        case .close: .close
        case .requestArrived: .request
        case .approved: .send
        case .denied: .close
        case .grantCreated: .done
        case .connected: .reconnected
        case .uploadApproved: .uploadReady
        case .revoked: .delete
        case .error: .error
        case .alert: .attention
        case .refresh: .refresh
        case .copied: .copy
        case .undo: .undo
        case .autoApproved: .autoApproved
        case .autoDenied: .autoDenied
        case .autopilotOn: .fastOn
        case .autopilotOff: .fastOff
        case .bypassOn: .surge
        case .bypassOff: .zip
        case .lockdownOn: .lockdown
        case .lockdownOff: .toggleOn
        }
    }

    /// `autopilotOn` when the new mode allows more than the old one, else `autopilotOff`.
    static func autopilotModeChanged(moreAutonomy: Bool) -> FeedbackEvent { moreAutonomy ? .autopilotOn : .autopilotOff }

    static func toggle(_ on: Bool) -> FeedbackEvent { on ? .toggleOn : .toggleOff }

    /// A section expanding or collapsing.
    static func expand(_ open: Bool) -> FeedbackEvent { open ? .open : .close }
}

protocol Feedback: AnyObject {
    func haptic(_ haptic: Haptic)
    /// `step` only matters to cues that climb a scale (`.detent`).
    func cue(_ cue: Cue, step: Int)
    /// Plays `cue` unless the same cue sounded within the last moment (a page closing right after the action that
    /// closed it).
    func cueUnlessRecent(_ cue: Cue)
    /// A press was released inside its target: the tap, unless the action it ran played its own haptic or cue.
    func defaultTap()
    /// A sheet or dialog appeared: the Open cue, unless a request's chime just announced what it shows.
    func sheetOpened()
    /// A choice was made whose own sound comes later (after the core answers): the close that follows stays silent.
    func quietClose()
}

extension Feedback {
    func defaultTap() { play(.tap) }

    func sheetOpened() { cue(.open) }

    func quietClose() {}

    func cue(_ cue: Cue) { self.cue(cue, step: 0) }

    /// Sound first: it is the channel perceived late.
    func play(haptic: Haptic?, cue: Cue?, step: Int = 0) {
        if let cue { self.cue(cue, step: step) }
        if let haptic { self.haptic(haptic) }
    }

    func play(_ event: FeedbackEvent, step: Int = 0) {
        play(haptic: event.haptic, cue: event.cue, step: step)
    }
}

/// Does nothing: previews, tests, and the extensions.
final class NoFeedback: Feedback {
    static let shared = NoFeedback()
    func haptic(_ haptic: Haptic) {}
    func cue(_ cue: Cue, step: Int) {}
    func cueUnlessRecent(_ cue: Cue) {}
}

/// A button look that answers every press with the default tap (the Android app's `pressable`): the shared button
/// styles are made of one, so their actions only play what is particular to them. The press runs the action first,
/// then `Feedback.defaultTap`, which gives way to whatever the action played itself.
///
/// A primitive style around a `Button` of the look, not a tap gesture on the label: a gesture there stops the button's
/// own action.
struct DefaultTapStyle<Look: ButtonStyle>: PrimitiveButtonStyle {
    var look: Look

    func makeBody(configuration: Configuration) -> some View {
        DefaultTapButton(configuration: configuration, look: look)
    }
}

private struct DefaultTapButton<Look: ButtonStyle>: View {
    var configuration: PrimitiveButtonStyleConfiguration
    var look: Look
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button(role: configuration.role) {
            configuration.trigger()
            feedback.defaultTap()
        } label: {
            configuration.label
        }
        .buttonStyle(look)
    }
}

extension View {
    /// For a sheet, alert or dialog shown while `shown`: the Open cue as it appears, and the Close cue as it goes,
    /// unless the choice that closed it just sounded (the Android app's `DialogFeedback`). `opens: false` where the
    /// presented content plays its own Open.
    func presentationFeedback(_ shown: Bool, opens: Bool = true) -> some View {
        modifier(PresentationFeedback(shown: shown, opens: opens))
    }
}

extension View {
    /// For a navigation stack `depth` pages deep: a page pushed opens with the Open cue, going back closes with the
    /// Close cue, quiet when the action that left just sounded (the Android app's `PageFeedback`). Where one view
    /// shows several stacks in turn (the detail column), `stack` says which: changing to another is not a page.
    func pageFeedback(_ depth: Int, in stack: AnyHashable = 0) -> some View {
        modifier(PageFeedback(position: PagePosition(stack: stack, depth: depth)))
    }
}

private struct PagePosition: Equatable {
    var stack: AnyHashable
    var depth: Int
}

private struct PageFeedback: ViewModifier {
    var position: PagePosition
    @Environment(\.feedback) private var feedback

    func body(content: Content) -> some View {
        content.onChange(of: position) { old, new in
            guard old.stack == new.stack else { return }
            if new.depth > old.depth {
                feedback.cue(.open)
            } else if new.depth < old.depth {
                feedback.cueUnlessRecent(.close)
            }
        }
    }
}

private struct PresentationFeedback: ViewModifier {
    var shown: Bool
    var opens: Bool
    @Environment(\.feedback) private var feedback

    func body(content: Content) -> some View {
        // On the change itself rather than `onDismiss`: a close button, a swipe and a choice all end it here, before
        // the dismissal's animation, so a choice's own sound is still recent.
        content.onChange(of: shown) { _, now in
            if now {
                if opens { feedback.sheetOpened() }
            } else {
                feedback.cueUnlessRecent(.close)
            }
        }
    }
}

private struct FeedbackKey: EnvironmentKey {
    static let defaultValue: Feedback = NoFeedback.shared
}

extension EnvironmentValues {
    var feedback: Feedback {
        get { self[FeedbackKey.self] }
        set { self[FeedbackKey.self] = newValue }
    }
}
