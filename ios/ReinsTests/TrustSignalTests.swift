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
}
