import Foundation

/// Which in-app switch governs a sound.
enum CueCategory: String, Codable, CaseIterable {
    /// Taps, toggles, sheets, approve and deny: the answer to the user's own hand.
    case interface
    /// A request, pairing or upload is waiting (in-app and its notification).
    case requests
    /// What Autopilot decides by itself.
    case autopilot
    /// Grants ending, this phone losing its role: the attention chime (in-app and its notification).
    case alerts
}

/// How hard haptics hit: scales Core Haptics intensities.
enum HapticStrength: String, Codable, CaseIterable {
    case subtle, standard, strong

    var label: String { rawValue.capitalized }

    var scale: Float {
        switch self {
        case .subtle: 0.5
        case .standard: 1
        case .strong: 1.5
        }
    }
}

/// The user's choices for sound and haptics (the Android app's `FeedbackSettings`), shared with the notification
/// extension so notifications follow them too. Everything defaults on.
///
/// `master` ("Sounds & haptics") is absolute: off silences every sound and every vibration, in the app and in the
/// notifications it posts.
struct FeedbackSettings: Codable, Equatable {
    var master = true
    var sounds = true
    var interfaceSounds = true
    var requestSounds = true
    var autopilotSounds = true
    var alertSounds = true
    /// 0...1 slider; the audible gain follows `gain`.
    var volume: Float = Self.defaultVolume
    var haptics = true
    var strength: HapticStrength = .standard

    static let defaultVolume: Float = 0.5
    /// At 100% every cue is twice as loud (+6 dB) as at the default 50%; the files carry the headroom.
    static let maxGain: Float = 2

    var soundsOn: Bool { master && sounds }
    var hapticsOn: Bool { master && haptics }

    func allows(_ category: CueCategory) -> Bool {
        guard soundsOn else { return false }
        switch category {
        case .interface: return interfaceSounds
        case .requests: return requestSounds
        case .autopilot: return autopilotSounds
        case .alerts: return alertSounds
        }
    }

    var gain: Float { Self.gain(for: volume) }

    /// The slider is linear, loudness is not. Up to the default the gain is the square of twice the slider (25% reads
    /// about -12 dB); above it, equal dB steps up to +6 dB at 100%. Both pieces meet at gain 1.
    static func gain(for slider: Float) -> Float {
        let v = 2 * min(max(slider, 0), 1)
        if v <= 1 { return v * v }
        return powf(maxGain, v - 1)
    }

    private static let key = "feedback.settings.v1"

    static func load(from defaults: UserDefaults = AppGroup.defaults) -> FeedbackSettings {
        guard let data = defaults.data(forKey: key), let s = try? JSONDecoder().decode(FeedbackSettings.self, from: data) else {
            return FeedbackSettings()
        }
        return s
    }

    func save(to defaults: UserDefaults = AppGroup.defaults) {
        if let data = try? JSONEncoder().encode(self) { defaults.set(data, forKey: Self.key) }
    }
}
