package dev.rewarden.android.feedback

import androidx.compose.runtime.staticCompositionLocalOf

/**
 * The touch vocabulary. Names describe the *moment*, not the vibration: [HapticTable] decides how each one is felt on
 * this device (platform effects, composition primitives, or nothing at all).
 */
enum class Haptic {
    /** The faintest detent: slider steps. */
    Tick,

    /** A choice was made: a chip, tab or list row. */
    Select,

    /** A switch or tick box turned on / off: a rising pair, a falling pair. */
    ToggleOn,
    ToggleOff,

    /** A user action was committed: approve, save, refresh. */
    Confirm,

    /** Something finished well: a permission given, an AI connected, a file released. */
    Success,

    /** Something needs the user: a request arrived. */
    Attention,

    /** Something failed. */
    Error,

    /** A weighty, destructive step: revoke, delete, disconnect. */
    Heavy,

    /** A small pop that decays: Autopilot answered for you. */
    Pop,

    /** A refusal: a firm click that falls away (the door shuts). */
    Deny,

    /** Every safeguard switched off: a rising surge ending in a crack. */
    Surge,

    /** Back to safety: a very short, sharp double tick. */
    Zip,

    /** Autopilot switched up: a ragged crackle that builds to a click. */
    Lightning,
}

/**
 * Sound cues, files in `res/raw` (see [CueTable]). The chimes are the desktop's notification family; the rest are
 * short interface cues in the same "rounded pressure pulse" style, all in C major pentatonic.
 */
enum class Cue {
    // Chimes (also the notification channels' sounds).
    Request,
    Attention,
    Done,

    // Longer action cues.
    Send,
    UploadReady,
    Reconnected,
    Undo,

    // Interface.
    Tap,
    Select,
    ToggleOn,
    ToggleOff,
    Open,
    Close,

    /** A slider detent; `step` climbs the pentatonic scale (see [DetentLadder]). */
    Detent,
    Delete,
    Copy,
    Error,
    Refresh,

    // Autopilot.
    /** Bypass on: a bright swell with sparkle. */
    Surge,

    /** Bypass off: a quick airy flick, falling. */
    Zip,

    /** A mode with more autonomy: a crackle with a bloom. */
    FastOn,

    /** A mode with less autonomy: the charge draining away. */
    FastOff,

    /** Autopilot approved something: the approve cue, far quieter. */
    AutoApproved,

    /** Autopilot denied something: the deny cue, far quieter. */
    AutoDenied,

    /** Lockdown: the attention chime, as an answer to the switch. */
    Lockdown,
}

/**
 * Rewarden's moments, each a haptic and a sound designed as a pair (either may be absent). This is what screens and
 * view models play: `feedback.play(Event.Approved)`.
 */
enum class Event(val haptic: Haptic?, val cue: Cue?) {
    /** The default answer of any plain control (see [TapFeedback.defaultTap]). */
    Tap(Haptic.Select, Cue.Tap),

    /** A chip, a picker option, a tab. */
    Selection(Haptic.Select, Cue.Select),
    ToggleOn(Haptic.ToggleOn, Cue.ToggleOn),
    ToggleOff(Haptic.ToggleOff, Cue.ToggleOff),

    /** A slider step; pass the step index so the detents climb the scale. */
    Detent(Haptic.Tick, Cue.Detent),
    /** A sheet, dialog or section opens (the tap's haptic stays). */
    Open(null, Cue.Open),

    /** A sheet, dialog or section is dismissed. */
    Close(null, Cue.Close),

    /** A request, pairing or upload arrived while the app is open (in the background its notification chimes). */
    RequestArrived(Haptic.Attention, Cue.Request),

    /** Approved once. */
    Approved(Haptic.Confirm, Cue.Send),

    /** Denied: a request, a pairing or an upload. */
    Denied(Haptic.Deny, Cue.Close),

    /** A standing permission was given: allow for a while, allow all, a new or resumed grant. */
    GrantCreated(Haptic.Success, Cue.Done),

    /** Something was connected: an AI (pairing approved), an MCP server, an account. */
    Connected(Haptic.Success, Cue.Reconnected),

    /** An uploaded file was approved and can be downloaded. */
    UploadApproved(Haptic.Success, Cue.UploadReady),

    /** Revoked or removed: a grant, a connection, an account, an MCP server. */
    Revoked(Haptic.Heavy, Cue.Delete),
    Error(Haptic.Error, Cue.Error),

    /** Something important happened that is not an error (this phone lost its approval role). */
    Alert(Haptic.Attention, Cue.Attention),
    Refresh(Haptic.Confirm, Cue.Refresh),
    Copied(Haptic.Confirm, Cue.Copy),
    Undo(Haptic.Select, Cue.Undo),

    // Autopilot. Automatic answers are subtle; switching modes answers the hand that switched.
    /** Autopilot approved a request on its own: felt more than heard. */
    AutoApproved(Haptic.Pop, Cue.AutoApproved),

    /** Autopilot denied a request on its own. */
    AutoDenied(Haptic.Tick, Cue.AutoDenied),

    /** Autopilot moved to a mode with more autonomy. */
    AutopilotOn(Haptic.Lightning, Cue.FastOn),

    /** Autopilot moved to a mode with less autonomy (or off). */
    AutopilotOff(Haptic.ToggleOff, Cue.FastOff),

    /** Bypass: requests are no longer asked for. Meant to be noticed. */
    BypassOn(Haptic.Surge, Cue.Surge),
    BypassOff(Haptic.Zip, Cue.Zip),

    /** Lockdown: everything is refused. */
    LockdownOn(Haptic.Heavy, Cue.Lockdown),
    LockdownOff(Haptic.ToggleOn, Cue.ToggleOn),
    ;

    companion object {
        /** [AutopilotOn] when the new mode allows more than the old one, else [AutopilotOff]. */
        fun autopilotModeChanged(moreAutonomy: Boolean): Event = if (moreAutonomy) AutopilotOn else AutopilotOff

        fun toggle(on: Boolean): Event = if (on) ToggleOn else ToggleOff

        /** A section expanding or collapsing. */
        fun expand(open: Boolean): Event = if (open) Open else Close
    }
}

interface Feedback {
    fun haptic(haptic: Haptic)

    /** [step] only matters to cues that climb a scale ([Cue.Detent]). */
    fun cue(cue: Cue, step: Int = 0)
}

/** Play whichever of [haptic] / [cue] is non-null. */
fun Feedback.play(haptic: Haptic?, cue: Cue?, step: Int = 0) {
    cue?.let { cue(it, step) } // sound first: it is the channel perceived late (see SoundBank)
    haptic?.let(::haptic)
}

/** The moment's haptic and sound together; [step] is for [Event.Detent]. */
fun Feedback.play(event: Event, step: Int = 0) = play(event.haptic, event.cue, step)

/** Does nothing: previews, tests, and the default until the app provides the real one. */
object NoFeedback : Feedback {
    override fun haptic(haptic: Haptic) = Unit
    override fun cue(cue: Cue, step: Int) = Unit
}

val LocalFeedback = staticCompositionLocalOf<Feedback> { NoFeedback }
