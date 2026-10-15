import XCTest
@testable import Reins

/// Trust in Signal: the mood line, and which field and list a running mode paints Trust with.
final class TrustSignalTests: XCTestCase {
    private func settings(_ mode: AutopilotMode, bypassUntil: Int64? = nil, bypassSince: Int64? = nil, overnightUntil: Int64? = nil) -> AutopilotSettings {
        AutopilotSettings(
            mode: mode, baseMode: .manual, bypassUntil: bypassUntil, bypassSince: bypassSince, overnightUntil: overnightUntil, overnightSince: nil,
            overnightWhenRisky: .wait, defaultProfileId: "personal", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 0, downloadedBytes: 0, error: nil, runtimeReady: true, risk: true, update: false),
            connections: []
        )
    }

    func testTheMoodCountsWhatAutopilotDecidedToday() {
        // Mid-morning wherever the test runs, so a minute earlier is still today.
        let midnight = Int64(Calendar.current.startOfDay(for: Date(timeIntervalSince1970: 1_800_000_000)).timeIntervalSince1970)
        let now = midnight + 10 * 3_600
        let entries = [
            DemoData.entry(1, at: now - 60, action: "read", outcome: "released", detail: "", decidedBy: "autopilot"),
            DemoData.entry(2, at: now - 120, action: "read", outcome: "released", detail: "", decidedBy: "overnight"),
            DemoData.entry(3, at: now - 180, action: "read", outcome: "released", detail: ""),
            DemoData.entry(4, at: now - 3 * 86_400, action: "read", outcome: "released", detail: "", decidedBy: "autopilot"),
        ]
        XCTAssertEqual(TrustText.decided(entries, now: now), "Autopilot decided 2 things today.")
        XCTAssertEqual(TrustText.decided(Array(entries.prefix(1)), now: now), "Autopilot decided 1 thing today.")
        XCTAssertEqual(TrustText.decided([], now: now), "Autopilot decided nothing yet today.")
    }

    func testWhileABypassOrOvernightRunsTheMoodSaysWhatItDidToday() {
        let midnight = Int64(Calendar.current.startOfDay(for: Date(timeIntervalSince1970: 1_800_000_000)).timeIntervalSince1970)
        let now = midnight + 10 * 3_600
        let bypassed = (1...100).map { k in
            DemoData.entry(Int64(k), at: now - 600 + Int64(k), action: "read", outcome: "released", detail: "", decidedBy: "bypass")
        }
        let s = settings(.bypass, bypassUntil: now + 3_600, bypassSince: now - 700)
        let listed = TrustRunning.bypass(until: now + 3_600).entries(bypassed, settings: s)
        XCTAssertEqual(TrustText.running(.bypass(until: now + 3_600), listed: listed, now: now), "Bypass approved 100 today.", "counted from the list")
        XCTAssertEqual(TrustText.decided(bypassed, now: now), "Autopilot decided nothing yet today.", "what it said before")
        XCTAssertEqual(TrustText.running(.bypass(until: now + 3_600), listed: [], now: now), "Bypass approved nothing yet today.")
        let yesterday = DemoData.entry(200, at: midnight - 60, action: "read", outcome: "released", detail: "", decidedBy: "bypass")
        XCTAssertEqual(TrustText.running(.bypass(until: now + 3_600), listed: [yesterday] + listed.prefix(1), now: now), "Bypass approved 1 today.", "today only")
        XCTAssertEqual(TrustText.running(.overnight(until: now + 3_600), listed: Array(listed.prefix(12)), now: now), "\(AutopilotText.overnightName) decided 12 today.")
        XCTAssertNil(TrustText.running(.lockdown, listed: listed, now: now), "a lockdown keeps Autopilot's line")
    }

    func testARunningModePaintsTrustInItsField() {
        XCTAssertNil(TrustRunning.of(settings(.manual)))
        XCTAssertNil(TrustRunning.of(settings(.auto)))
        XCTAssertEqual(TrustRunning.of(settings(.bypass, bypassUntil: 100))?.field, .delete, "a bypass is red")
        XCTAssertEqual(TrustRunning.of(settings(.overnight, overnightUntil: 100))?.field, .grant, "Overnight is violet")
        XCTAssertEqual(TrustRunning.of(settings(.lockdown))?.field, .warn, "Lockdown is amber")
        XCTAssertEqual(TrustRunning.lockdown.hero.italic, "is denied")
        XCTAssertEqual(TrustRunning.bypass(until: 1).label, "Autopilot · Bypass")
    }

    func testTheListUnderTheFieldIsWhatItDecidedWhileItRan() {
        let entries = [
            DemoData.entry(3, at: 1_000, action: "read", outcome: "released", detail: "", decidedBy: "bypass"),
            DemoData.entry(2, at: 500, action: "read", outcome: "released", detail: "", decidedBy: "bypass"),
            DemoData.entry(1, at: 900, action: "read", outcome: "denied", detail: "", decidedBy: "lockdown"),
        ]
        let bypass = TrustRunning.bypass(until: 2_000)
        XCTAssertEqual(bypass.entries(entries, settings: settings(.bypass, bypassUntil: 2_000, bypassSince: 800)).map(\.id), [3], "only since it started")
        XCTAssertEqual(TrustRunning.lockdown.entries(entries, settings: settings(.lockdown)).map(\.id), [1])
        XCTAssertEqual(bypass.listTitle, "During this bypass")
    }

    func testALongListFoldsToItsLatestThree() {
        // 100 entries of a running bypass, in any order: newest first, folded to three, "Show all 97" opens the rest.
        let entries = (1...100).map { k in
            DemoData.entry(Int64(k), at: 1_000 + Int64((k * 37) % 100), action: "read", outcome: "released", detail: "", decidedBy: "bypass")
        }.shuffled()
        let mine = TrustRunning.bypass(until: 5_000).entries(entries, settings: settings(.bypass, bypassUntil: 5_000, bypassSince: 900))
        XCTAssertEqual(mine.count, 100, "nothing is dropped")
        XCTAssertEqual(mine.map(\.at), mine.map(\.at).sorted(by: >), "newest first")
        XCTAssertEqual(RunningListFold.shown(mine.count, expanded: false), 3)
        XCTAssertEqual(RunningListFold.shown(mine.count, expanded: true), 100)
        XCTAssertEqual(RunningListFold.showAll(mine.count), "Show all 97")
        XCTAssertEqual(RunningListFold.shown(2, expanded: false), 2)
        XCTAssertNil(RunningListFold.showAll(3), "nothing to open")
        XCTAssertEqual(RunningListFold.showAll(4), "Show all 1")
    }

    // MARK: The fold

    private let sizes = RunningFieldGeometry.Sizes(
        width: 390, labelHeight: 16, labelNaturalWidth: 180, clockOpen: CGSize(width: 220, height: 70), clockCompact: CGSize(width: 220, height: 70),
        heroHeight: 100, buttonCompactWidth: 120
    )

    func testTheFoldFollowsTheScrollAndCrossFadesUnderReduceMotion() {
        XCTAssertEqual(TrustFold.progress(scrolled: 0, range: 200, reduceMotion: false), 0)
        XCTAssertEqual(TrustFold.progress(scrolled: 50, range: 200, reduceMotion: false), 0.25, "continuous, not a snap")
        XCTAssertEqual(TrustFold.progress(scrolled: 900, range: 200, reduceMotion: false), 1)
        XCTAssertEqual(TrustFold.progress(scrolled: -40, range: 200, reduceMotion: false), 0, "pulling to refresh keeps it open")
        XCTAssertEqual(TrustFold.progress(scrolled: 99, range: 200, reduceMotion: true), 0)
        XCTAssertEqual(TrustFold.progress(scrolled: 100, range: 200, reduceMotion: true), 1, "Reduce Motion: one state or the other")
        XCTAssertEqual(TrustFold.progress(scrolled: 100, range: 0, reduceMotion: false), 0, "open until measured")
        XCTAssertEqual(TrustFold.held(500, range: 200), 200, "nothing past the fold redraws the field")
        XCTAssertEqual(TrustFold.held(-30, range: 200), 0)
        XCTAssertEqual(TrustFold.heroOpacity(0), 1)
        XCTAssertEqual(TrustFold.heroOpacity(0.45), 0)
        XCTAssertEqual(TrustFold.stateIdentifier(0), "runningFieldOpen")
        XCTAssertEqual(TrustFold.stateIdentifier(0.4), "runningFieldFolding")
        XCTAssertEqual(TrustFold.stateIdentifier(1), "runningFieldCompact")
    }

    func testOpenTheButtonRunsAcrossUnderTheSentence() {
        let g = RunningFieldGeometry(sizes, progress: 0)
        XCTAssertEqual(g.button.minX, 22)
        XCTAssertEqual(g.button.width, 346)
        XCTAssertEqual(g.button.height, 54)
        XCTAssertGreaterThan(g.button.minY, g.hero.maxY)
        XCTAssertGreaterThan(g.hero.minY, g.clock!.maxY)
        XCTAssertEqual(g.surface.height, g.openHeight)
        XCTAssertEqual(g.openHeight, g.button.maxY + 22)
    }

    func testCompactTheButtonSitsRightOfTheTimer() {
        let g = RunningFieldGeometry(sizes, progress: 1)
        XCTAssertEqual(g.button.maxX, 390 - 22, "at the right edge")
        XCTAssertEqual(g.button.width, 120, "its own width")
        XCTAssertEqual(g.button.height, 44, "still a full touch target")
        XCTAssertEqual(g.label.minY, 6, "the label line stays")
        let clock = g.clock!
        let drawnClock = clock.height * RunningFieldGeometry.clockScale
        XCTAssertEqual(g.button.midY, clock.minY + drawnClock / 2, accuracy: 0.5, "level with the timer")
        XCTAssertLessThan(clock.minX + clock.width * RunningFieldGeometry.clockScale, g.button.minX, "side by side")
        XCTAssertEqual(g.surface.height, g.compactHeight)
        XCTAssertLessThan(g.compactHeight, g.openHeight / 2)
        XCTAssertEqual(g.openHeight, RunningFieldGeometry(sizes, progress: 0).openHeight, "the space it keeps does not move")
    }

    func testHalfwayEverythingIsHalfwayThere() {
        let open = RunningFieldGeometry(sizes, progress: 0), compact = RunningFieldGeometry(sizes, progress: 1)
        let mid = RunningFieldGeometry(sizes, progress: 0.5)
        XCTAssertEqual(mid.surface.height, (open.surface.height + compact.surface.height) / 2, accuracy: 0.01)
        XCTAssertEqual(mid.button.minY, (open.button.minY + compact.button.minY) / 2, accuracy: 0.01)
        XCTAssertEqual(mid.button.width, (open.button.width + compact.button.width) / 2, accuracy: 0.01)
        XCTAssertEqual(RunningFieldGeometry.clockScale(0.5), (1 + RunningFieldGeometry.clockScale) / 2, accuracy: 0.001)
    }

    func testALockdownFoldsItsButtonOntoTheLabelLine() {
        var lockdown = sizes
        lockdown.clockOpen = nil
        lockdown.clockCompact = nil
        let g = RunningFieldGeometry(lockdown, progress: 1)
        XCTAssertNil(g.clock)
        XCTAssertEqual(g.button.midY, g.label.midY, accuracy: 0.5)
        XCTAssertLessThanOrEqual(g.label.maxX, g.button.minX, "the label gives the button room")
        XCTAssertEqual(g.compactHeight, 6 + 44 + 12)
    }

    func testALabelTooLongToShareItsLineKeepsItAndTheButtonGoesUnder() {
        var lockdown = sizes
        lockdown.clockOpen = nil
        lockdown.clockCompact = nil
        lockdown.labelNaturalWidth = 230
        lockdown.buttonCompactWidth = 170
        let g = RunningFieldGeometry(lockdown, progress: 1)
        XCTAssertEqual(g.label.width, 346, "never cut")
        XCTAssertEqual(g.button.maxX, 390 - 22, "at the right edge")
        XCTAssertGreaterThanOrEqual(g.button.minY, g.label.maxY, "under the label")
        XCTAssertEqual(g.button.height, 44)
    }
}
