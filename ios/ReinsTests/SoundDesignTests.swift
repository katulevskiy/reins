import XCTest
@testable import Reins

/// The cue table, the event vocabulary, the volume curve, and the files they name (the Android app's
/// `SoundDesignTest` and `FeedbackSettingsTest`).
final class SoundDesignTests: XCTestCase {
    private var shipped: Set<String> {
        let urls = (Bundle.main.urls(forResourcesWithExtension: "wav", subdirectory: nil) ?? [])
            + (Bundle.main.urls(forResourcesWithExtension: "wav", subdirectory: "Sounds") ?? [])
        return Set(urls.map { $0.deletingPathExtension().lastPathComponent }.filter { $0.hasPrefix("fx_") })
    }

    func testEveryCueHasASpecAndItsFileIsInTheApp() {
        XCTAssertEqual(CueTable.all.count, Cue.allCases.count)
        for spec in CueTable.all {
            XCTAssertNotNil(spec.file.range(of: "^fx_[a-z_]+$", options: .regularExpression), spec.file)
            XCTAssertTrue((0.1...1).contains(spec.gain))
            XCTAssertTrue(shipped.contains(spec.file), "\(spec.cue) -> \(spec.file)")
        }
    }

    func testOnlyTheQuietEchoesAndLockdownBorrowAnotherCuesFile() {
        let groups = Dictionary(grouping: CueTable.all, by: \.file).filter { $0.value.count > 1 }
        let shared = Set(groups.values.flatMap { $0.map(\.cue) })
        XCTAssertEqual(shared, [.send, .autoApproved, .close, .autoDenied, .attention, .lockdown])
        XCTAssertEqual(Set(CueTable.all.map(\.file)).count, CueTable.files.count)
    }

    func testNothingUnusedShips() {
        XCTAssertEqual(Set(CueTable.files), shipped)
    }

    func testTheChimesGovernByTheirOwnSwitches() {
        XCTAssertEqual(CueTable.spec(.request).file, "fx_chime_request")
        XCTAssertEqual(CueTable.spec(.attention).file, "fx_chime_attention")
        XCTAssertEqual(CueTable.spec(.request).category, .requests)
        XCTAssertEqual(CueTable.spec(.attention).category, .alerts)
        XCTAssertEqual(CueTable.spec(.autoApproved).category, .autopilot)
        XCTAssertEqual(CueTable.spec(.autoDenied).category, .autopilot)
    }

    func testWhatAutopilotDecidesIsQuieterAndLighterThanWhatTheUserDoes() {
        XCTAssertLessThan(CueTable.spec(.autoApproved).gain, CueTable.spec(.send).gain / 2)
        XCTAssertLessThan(CueTable.spec(.autoDenied).gain, CueTable.spec(.close).gain)
        XCTAssertEqual(CueTable.spec(.autoApproved).priority, 0)
        XCTAssertEqual(CueTable.spec(.autoDenied).priority, 0)
    }

    func testEveryEventHasASoundOrAHaptic() {
        for e in FeedbackEvent.allCases { XCTAssertTrue(e.haptic != nil || e.cue != nil, "\(e)") }
        XCTAssertEqual(FeedbackEvent.autopilotModeChanged(moreAutonomy: true), .autopilotOn)
        XCTAssertEqual(FeedbackEvent.autopilotModeChanged(moreAutonomy: false), .autopilotOff)
        XCTAssertEqual(FeedbackEvent.toggle(true), .toggleOn)
        XCTAssertEqual(FeedbackEvent.expand(false), .close)
    }

    func testTheMainMomentsSoundAsDesigned() {
        XCTAssertEqual(FeedbackEvent.requestArrived.haptic, .attention)
        XCTAssertEqual(FeedbackEvent.requestArrived.cue, .request)
        XCTAssertEqual(FeedbackEvent.approved.haptic, .confirm)
        XCTAssertEqual(FeedbackEvent.approved.cue, .send)
        XCTAssertEqual(FeedbackEvent.denied.haptic, .deny)
        XCTAssertEqual(FeedbackEvent.denied.cue, .close)
        XCTAssertEqual(FeedbackEvent.grantCreated.cue, .done)
        XCTAssertEqual(FeedbackEvent.revoked.haptic, .heavy)
        XCTAssertEqual(FeedbackEvent.uploadApproved.cue, .uploadReady)
        XCTAssertEqual(FeedbackEvent.bypassOn.haptic, .surge)
        XCTAssertEqual(FeedbackEvent.bypassOn.cue, .surge)
    }

    func testTheDetentClimbsAndStaysInRange() {
        var last = -100
        for step in 0...4 {
            let st = DetentLadder.semitones(step)
            XCTAssertGreaterThan(st, last, "step \(step)")
            last = st
        }
        for step in -5...60 { XCTAssertTrue((DetentLadder.minRate...DetentLadder.maxRate).contains(DetentLadder.rate(step))) }
        XCTAssertGreaterThan(DetentLadder.rate(5), DetentLadder.rate(4), "the next octave continues upward")
        XCTAssertEqual(DetentLadder.rate(40), DetentLadder.rate(80), "the top holds")
        XCTAssertEqual(DetentLadder.semitones(0), -7)
        XCTAssertEqual(DetentLadder.semitones(-1), -10, "below the ladder it walks down the scale")
    }

    // MARK: Settings and gain

    func testEverythingDefaultsOnAtHalfVolume() {
        let d = FeedbackSettings()
        XCTAssertTrue(d.master && d.sounds && d.interfaceSounds && d.requestSounds && d.autopilotSounds && d.alertSounds && d.haptics)
        XCTAssertEqual(d.strength, .standard)
        XCTAssertEqual(d.volume, 0.5)
        XCTAssertEqual(d.gain, 1, accuracy: 1e-6)
    }

    func testCategorySwitchesAreIndependentUnderTheMaster() {
        var s = FeedbackSettings()
        s.requestSounds = false
        XCTAssertFalse(s.allows(.requests))
        XCTAssertTrue(s.allows(.alerts) && s.allows(.autopilot) && s.allows(.interface))
        var noSounds = FeedbackSettings()
        noSounds.sounds = false
        XCTAssertFalse(noSounds.allows(.interface))
        var off = FeedbackSettings()
        off.master = false
        for category in CueCategory.allCases { XCTAssertFalse(off.allows(category)) }
        XCTAssertFalse(off.hapticsOn)
    }

    func testTheFullSliderIsSixDecibelsLouderThanTheDefault() {
        let max = FeedbackSettings.gain(for: 1)
        XCTAssertEqual(max, FeedbackSettings.maxGain, accuracy: 1e-6)
        XCTAssertEqual(20 * log10(Double(max)), 6.02, accuracy: 0.01)
        XCTAssertEqual(FeedbackSettings.gain(for: 0), 0)
        XCTAssertEqual(FeedbackSettings.gain(for: 7), FeedbackSettings.maxGain, accuracy: 1e-6)
    }

    func testTheVolumeCurveIsMonotoneContinuousAndSquaredBelowTheDefault() {
        var last: Float = -1
        for i in 0...100 {
            let g = FeedbackSettings.gain(for: Float(i) / 100)
            XCTAssertTrue(g > last || i == 0, "\(i)%")
            last = g
        }
        XCTAssertEqual(FeedbackSettings.gain(for: 0.25), 0.25, accuracy: 1e-6)
        XCTAssertEqual(FeedbackSettings.gain(for: 0.5), FeedbackSettings.gain(for: 0.5001), accuracy: 1e-3)
        let step = 20 * log10(Double(FeedbackSettings.gain(for: 0.75)) / Double(FeedbackSettings.gain(for: 0.5)))
        XCTAssertEqual(step, 3.01, accuracy: 0.01)
    }

    func testPlayerVolumeNeverExceedsOneAndTheDefaultPlaysTheFilesAtHalf() {
        let tap = CueTable.spec(.tap)
        XCTAssertEqual(CueTable.volume(tap, userGain: FeedbackSettings().gain), 0.5, accuracy: 1e-6)
        XCTAssertEqual(CueTable.volume(tap, userGain: FeedbackSettings.gain(for: 1)), 1, accuracy: 1e-6)
        for s in CueTable.all {
            for i in 0...100 { XCTAssertLessThanOrEqual(CueTable.volume(s, userGain: FeedbackSettings.gain(for: Float(i) / 100)), 1) }
        }
        let fast = CueTable.spec(.fastOn)
        XCTAssertEqual(CueTable.volume(fast, userGain: FeedbackSettings().gain), 0.5 * fast.gain, accuracy: 1e-6, "a trim is kept")
    }

    func testTheStoreSavesWhereTheExtensionReadsAndClampsTheVolume() throws {
        let suite = "reins.tests.feedback.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = FeedbackStore(defaults: defaults)
        XCTAssertEqual(store.current, FeedbackSettings())
        store.update {
            $0.master = false
            $0.strength = .strong
            $0.volume = 9
        }
        XCTAssertEqual(store.settings.volume, 1)
        let reread = FeedbackSettings.load(from: defaults)
        XCTAssertFalse(reread.master)
        XCTAssertEqual(reread.strength, .strong)
        XCTAssertEqual(FeedbackStore(defaults: defaults).current, store.current)
        store.update { $0.volume = .nan }
        XCTAssertEqual(store.current.volume, FeedbackSettings.defaultVolume)
    }
}
