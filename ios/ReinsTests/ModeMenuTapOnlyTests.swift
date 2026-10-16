import XCTest
@testable import Reins

/// Activity's mode menu: what loosens control, or stops what runs, is never picked by a slide from the chip, only by a
/// tap on its row (`SignalMenuItem.tapOnly`).
final class ModeMenuTapOnlyTests: XCTestCase {
    private func settings(_ mode: AutopilotMode, bypassUntil: Int64? = nil, overnightUntil: Int64? = nil) -> AutopilotSettings {
        AutopilotSettings(
            mode: mode, baseMode: .manual, bypassUntil: bypassUntil, bypassSince: nil, overnightUntil: overnightUntil, overnightSince: nil,
            overnightWhenRisky: .wait, defaultProfileId: "personal", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 0, downloadedBytes: 0, error: nil, runtimeReady: true, risk: true, update: false),
            connections: []
        )
    }

    private func tapOnly(_ s: AutopilotSettings) -> Set<String> {
        Set(ActivityModeMenu.items(s, modelReady: true).filter(\.tapOnly).map(\.id))
    }

    func testFromManualOnlyAutoNeedsATap() {
        XCTAssertEqual(tapOnly(settings(.manual)), ["quick:AUTO"])
    }

    func testFromLockdownLeavingItNeedsATap() {
        XCTAssertEqual(tapOnly(settings(.lockdown)), ["quick:MANUAL", "quick:AUTO", "quick:LOCKDOWN"])
    }

    func testFromAutoNothingTightensByTapOnly() {
        // Manual and Lockdown tighten; Overnight and Bypass… open their own held sheets.
        XCTAssertEqual(tapOnly(settings(.auto)), [])
    }

    func testStoppingABypassOrAnOvernightNeedsATap() {
        let now = Int64(Date().timeIntervalSince1970)
        XCTAssertTrue(tapOnly(settings(.bypass, bypassUntil: now + 600)).contains("quick:BYPASS"))
        XCTAssertTrue(tapOnly(settings(.overnight, overnightUntil: now + 600)).contains("quick:OVERNIGHT"))
    }
}
