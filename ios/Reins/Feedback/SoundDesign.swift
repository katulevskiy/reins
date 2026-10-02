import Foundation

/// One sound cue as the engine plays it: which file, how loud relative to the set, how important it is when sounds
/// compete, and which in-app switch governs it (the Android app's `CueSpec`). Files are `Resources/Sounds/<file>.wav`,
/// mono 16-bit 48 kHz, mastered with `CueTable.assetBoost` of headroom and trimmed to start sounding within 1 ms (Zeron's
/// sound set). Two cues may share a file at different levels.
struct CueSpec: Equatable {
    let cue: Cue
    /// File name without extension.
    let file: String
    let category: CueCategory
    /// Level trim against the rest of the set, 0...1 (before the user's volume).
    let gain: Float
    /// 0 interface, 1 action, 2 event, 3 alert. Higher wins when voices are scarce.
    let priority: Int
    /// The same cue is never stacked within this many ms.
    var minGapMs: Int64 = 60
    /// Approximate length, used to count voices still sounding.
    var approxMs: Int64 = 120
}

enum CueTable {
    /// Every `fx_*.wav` is mastered this much hotter (+6 dB) than the default level, to give the volume slider its
    /// headroom: a player's volume cannot exceed 1.0, so "twice as loud at 100%" lives in the file and the default
    /// volume plays it at half (see `volume`).
    static let assetBoost: Float = 2

    /// Player volume (0...1) for `spec` at user gain `userGain` (0...`FeedbackSettings.maxGain`).
    static func volume(_ spec: CueSpec, userGain: Float) -> Float {
        min(max(userGain * spec.gain / assetBoost, 0), 1)
    }

    static func spec(_ cue: Cue) -> CueSpec { specs[cue] ?? build(cue) }

    private static let specs: [Cue: CueSpec] = Dictionary(uniqueKeysWithValues: Cue.allCases.map { ($0, build($0)) })

    private static func build(_ cue: Cue) -> CueSpec {
        switch cue {
        // The chimes, 2-4 dB above the interface set. The notifications play the same files.
        case .request: CueSpec(cue: cue, file: "fx_chime_request", category: .requests, gain: 1, priority: 2, minGapMs: 400, approxMs: 600)
        case .attention: CueSpec(cue: cue, file: "fx_chime_attention", category: .alerts, gain: 1, priority: 3, minGapMs: 400, approxMs: 650)
        case .done: CueSpec(cue: cue, file: "fx_chime_done", category: .interface, gain: 0.8, priority: 2, minGapMs: 400, approxMs: 520)

        // Longer action cues (about 2 dB hotter than the interface set, so trimmed).
        case .send: CueSpec(cue: cue, file: "fx_send", category: .interface, gain: 0.8, priority: 1, minGapMs: 120, approxMs: 160)
        case .uploadReady: CueSpec(cue: cue, file: "fx_upload_ready", category: .interface, gain: 0.8, priority: 2, minGapMs: 250, approxMs: 300)
        case .reconnected: CueSpec(cue: cue, file: "fx_reconnected", category: .interface, gain: 0.8, priority: 2, minGapMs: 400, approxMs: 450)
        case .undo: CueSpec(cue: cue, file: "fx_undo", category: .interface, gain: 0.8, priority: 1, minGapMs: 120, approxMs: 250)

        // Interface cues: subliminal, matched in loudness.
        case .tap: CueSpec(cue: cue, file: "fx_tap", category: .interface, gain: 1, priority: 0, minGapMs: 60, approxMs: 60)
        case .select: CueSpec(cue: cue, file: "fx_select", category: .interface, gain: 1, priority: 0, minGapMs: 60, approxMs: 80)
        case .toggleOn: CueSpec(cue: cue, file: "fx_toggle_on", category: .interface, gain: 1, priority: 1, minGapMs: 80, approxMs: 90)
        case .toggleOff: CueSpec(cue: cue, file: "fx_toggle_off", category: .interface, gain: 1, priority: 1, minGapMs: 80, approxMs: 90)
        case .open: CueSpec(cue: cue, file: "fx_open", category: .interface, gain: 1, priority: 1, minGapMs: 120, approxMs: 120)
        case .close: CueSpec(cue: cue, file: "fx_close", category: .interface, gain: 1, priority: 1, minGapMs: 120, approxMs: 120)
        case .detent: CueSpec(cue: cue, file: "fx_detent", category: .interface, gain: 1, priority: 0, minGapMs: 60, approxMs: 60)
        case .delete: CueSpec(cue: cue, file: "fx_delete", category: .interface, gain: 1, priority: 1, minGapMs: 120, approxMs: 120)
        case .copy: CueSpec(cue: cue, file: "fx_copy", category: .interface, gain: 1, priority: 1, minGapMs: 120, approxMs: 120)
        case .error: CueSpec(cue: cue, file: "fx_error", category: .interface, gain: 1, priority: 3, minGapMs: 250, approxMs: 180)
        case .refresh: CueSpec(cue: cue, file: "fx_refresh", category: .interface, gain: 1, priority: 1, minGapMs: 200, approxMs: 120)

        // Autopilot modes answer the switch, so they are interface sounds. Surge swells for half a second and ranks as
        // an action (a later tap does not cut it).
        case .surge: CueSpec(cue: cue, file: "fx_surge", category: .interface, gain: 1, priority: 1, minGapMs: 400, approxMs: 490)
        case .zip: CueSpec(cue: cue, file: "fx_zip", category: .interface, gain: 1, priority: 0, minGapMs: 100, approxMs: 85)
        case .fastOn: CueSpec(cue: cue, file: "fx_fast_on", category: .interface, gain: 0.7, priority: 1, minGapMs: 300, approxMs: 195)
        case .fastOff: CueSpec(cue: cue, file: "fx_fast_off", category: .interface, gain: 0.8, priority: 1, minGapMs: 200, approxMs: 100)
        case .lockdown: CueSpec(cue: cue, file: "fx_chime_attention", category: .interface, gain: 0.8, priority: 2, minGapMs: 400, approxMs: 650)

        // What Autopilot decides by itself: the user's own approve and deny, far quieter, and the lightest priority.
        case .autoApproved: CueSpec(cue: cue, file: "fx_send", category: .autopilot, gain: 0.3, priority: 0, minGapMs: 250, approxMs: 160)
        case .autoDenied: CueSpec(cue: cue, file: "fx_close", category: .autopilot, gain: 0.5, priority: 0, minGapMs: 250, approxMs: 120)
        }
    }

    static var all: [CueSpec] { Cue.allCases.map(spec) }

    /// Each file once, for loading.
    static var files: [String] {
        var seen = Set<String>()
        return all.map(\.file).filter { seen.insert($0).inserted }
    }
}

/// The detent cue climbs a major-pentatonic ladder (the family's key), played by resampling one 880 Hz tick: step 0
/// sits a fourth below the reference and each step walks up the scale, so dragging a slider "plays" it. Rates stay
/// inside 0.5...2.0 (the Android SoundPool's range, kept so both apps sound alike); beyond the top the ladder holds.
enum DetentLadder {
    private static let degrees = [0, 2, 4, 7, 9]
    static let baseSemitones = -7
    static let maxSemitones = 12
    static let minRate: Float = 0.5
    static let maxRate: Float = 2.0

    static func semitones(_ step: Int) -> Int {
        let n = degrees.count
        let octave = Int((Double(step) / Double(n)).rounded(.down))
        let degree = ((step % n) + n) % n
        return min(max(baseSemitones + 12 * octave + degrees[degree], -12), maxSemitones)
    }

    static func rate(_ step: Int) -> Float {
        min(max(Float(pow(2.0, Double(semitones(step)) / 12.0)), minRate), maxRate)
    }
}
