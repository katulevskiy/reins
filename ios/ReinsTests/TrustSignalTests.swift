import XCTest
@testable import Reins

/// Trust in Next: which mode runs as Trust's field, what its stack lists and folds to, the field's two-step collapse,
/// where the ears go, and how grant rows and the ended list read.
final class TrustSignalTests: XCTestCase {
    private func settings(_ mode: AutopilotMode, bypassUntil: Int64? = nil, bypassSince: Int64? = nil, overnightUntil: Int64? = nil) -> AutopilotSettings {
        AutopilotSettings(
            mode: mode, baseMode: .manual, bypassUntil: bypassUntil, bypassSince: bypassSince, overnightUntil: overnightUntil, overnightSince: nil,
            defaultProfileId: "personal", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 0, downloadedBytes: 0, error: nil, runtimeReady: true, risk: true, update: false),
            connections: []
        )
    }

    func testARunningModeBecomesTrustsField() {
        XCTAssertNil(TrustRunning.of(settings(.manual)))
        XCTAssertNil(TrustRunning.of(settings(.auto)))
        XCTAssertEqual(TrustRunning.of(settings(.bypass, bypassUntil: 100))?.kind, .bypass, "a bypass is red")
        XCTAssertEqual(TrustRunning.of(settings(.overnight, overnightUntil: 100))?.kind, .overnight, "Overnight is navy")
        XCTAssertEqual(TrustRunning.of(settings(.lockdown))?.kind, .lockdown, "Lockdown is amber")
        XCTAssertEqual(TrustRunning.bypass(until: 1).kind.fill, Palette.fieldDelete)
        XCTAssertEqual(TrustRunning.overnight(until: 1).kind.fill, Palette.modeOvernight)
        XCTAssertNil(TrustRunning.lockdown.until, "the core keeps no end for a lockdown")
        XCTAssertEqual(TrustRunning.bypass(until: 1).countGlyph, .check)
        XCTAssertEqual(TrustRunning.overnight(until: 1).countGlyph, .pause)
        XCTAssertEqual(TrustRunning.lockdown.countGlyph, .close)
        XCTAssertEqual(TrustRunning.bypass(until: 1).stopIdentifier, "stopBypass")
        XCTAssertEqual(TrustRunning.lockdown.stopIdentifier, "endLockdown")
    }

    func testTheClockSaysWhatIsLeft() {
        XCTAssertEqual(RunningClock.text(.bypass(until: 1_000 + 47 * 60 + 12), now: 1_000), "47:12")
        XCTAssertEqual(RunningClock.text(.overnight(until: 1_000 + 5 * 3_600 + 59 * 60 + 12), now: 1_000), "5:59:12")
        XCTAssertEqual(RunningClock.text(.bypass(until: AutopilotText.untilOff), now: 1_000), "∞")
        XCTAssertNil(RunningClock.text(.lockdown, now: 1_000))
        XCTAssertEqual(RunningClock.spoken(.lockdown, now: 1_000), "Lockdown")
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
        XCTAssertEqual(bypass.listTitle, "During bypass")
        XCTAssertEqual(TrustRunning.lockdown.listTitle, "Denied")
        XCTAssertEqual(bypass.outcome, .bypass)
        XCTAssertEqual(TrustRunning.lockdown.outcome, .locked)
    }

    func testALongListStacksUnderItsNewest() {
        // 100 entries of a running bypass, in any order: newest first, folded to one with two peeking, "+99" opens it.
        let entries = (1...100).map { k in
            DemoData.entry(Int64(k), at: 1_000 + Int64((k * 37) % 100), action: "read", outcome: "released", detail: "", decidedBy: "bypass")
        }.shuffled()
        let mine = TrustRunning.bypass(until: 5_000).entries(entries, settings: settings(.bypass, bypassUntil: 5_000, bypassSince: 900))
        XCTAssertEqual(mine.count, 100, "nothing is dropped")
        XCTAssertEqual(mine.map(\.at), mine.map(\.at).sorted(by: >), "newest first")
        XCTAssertEqual(RunningListFold.shown(mine.count, expanded: false), 1)
        XCTAssertEqual(RunningListFold.shown(mine.count, expanded: true), 100)
        XCTAssertEqual(RunningListFold.showAll(mine.count), "Show all 99")
        XCTAssertEqual(RunningListFold.hidden(4), 3, "+3")
        XCTAssertNil(RunningListFold.showAll(1), "nothing to open")
        XCTAssertEqual(RunningListFold.peeks(1), 0)
        XCTAssertEqual(RunningListFold.peeks(2), 1)
        XCTAssertEqual(RunningListFold.peeks(40), 2)
    }

    // MARK: The collapse

    func testScrollingCollapsesInTwoSteps() {
        // A full field 268 pt tall, the short bar 118, the mini 60.
        func stage(_ y: CGFloat) -> RunningStage { RunningStage.at(offset: y, fieldBottom: 268, shortHeight: 118, miniHeight: 60) }
        XCTAssertEqual(stage(-40), .full, "pulling to refresh keeps it full")
        XCTAssertEqual(stage(0), .full)
        XCTAssertEqual(stage(149), .full)
        XCTAssertEqual(stage(150), .short, "once only the short bar's height is left")
        XCTAssertEqual(stage(247), .short, "the short bar is seen for a while")
        XCTAssertEqual(stage(248), .mini)
        XCTAssertEqual(stage(2_000), .mini)
        XCTAssertEqual(RunningStage.at(offset: 500, fieldBottom: 0, shortHeight: 118, miniHeight: 60), .full, "full until measured")
        XCTAssertEqual(RunningStage.full.value, "open")
        XCTAssertEqual(RunningStage.short.value, "short")
        XCTAssertEqual(RunningStage.mini.value, "compact")
    }

    func testTheEarsGoBesideTheIslandOrUnderTheStatusBar() {
        let island = EarsPlacement(safeTop: 59, accessibility: false)
        XCTAssertTrue(island.beside)
        XCTAssertTrue(island.hidesStatusBar, "the ears take the status bar's place")
        XCTAssertEqual(island.top, 12)
        XCTAssertEqual(island.side, 30, "clear of the corner curve")
        XCTAssertEqual(island.maxWidth, 95, "never reaching the island")
        XCTAssertEqual(island.contentTop, 64)
        XCTAssertEqual(island.shortHeight, 118)
        XCTAssertEqual(island.miniHeight, 60)

        let se = EarsPlacement(safeTop: 20, accessibility: false)
        XCTAssertFalse(se.beside, "no island: the status bar stays")
        XCTAssertFalse(se.hidesStatusBar)
        XCTAssertEqual(se.top, 20, "right under the status bar")
        XCTAssertEqual(se.side, 16)
        XCTAssertEqual(se.maxWidth, 160)
        XCTAssertEqual(se.contentTop, 66)

        let largest = EarsPlacement(safeTop: 59, accessibility: true)
        XCTAssertFalse(largest.beside, "accessibility sizes no longer fit beside the island")
        XCTAssertFalse(largest.hidesStatusBar)
        XCTAssertGreaterThanOrEqual(largest.top, 59)
        XCTAssertGreaterThanOrEqual(largest.height, 44)
    }

    // MARK: Grants

    func testAGrantsTargetIsSetApart() {
        XCTAssertEqual(GrantWords.runs("Read emails from @bank.com"), [AskText.Run(text: "Read emails from "), AskText.Run(text: "@bank.com", target: true)])
        XCTAssertEqual(GrantWords.runs("Change issues and code of octo/app"), [AskText.Run(text: "Change issues and code of "), AskText.Run(text: "octo/app", target: true)])
        XCTAssertEqual(GrantWords.runs("Read all emails"), [AskText.Run(text: "Read all emails")])
        XCTAssertEqual(GrantWords.runs("Read\nemails from @bank.com").map(\.text).joined().contains("\n"), false, "one clean line")
    }

    func testAGrantsFactsReadShort() {
        let g = DemoData.grants(1_800_000_000).first { $0.maxUses == nil && $0.uses > 1 }!
        XCTAssertEqual(GrantFactText.uses(g), "\(g.uses) uses")
        var capped = g
        capped.maxUses = 5
        capped.uses = 3
        XCTAssertEqual(GrantFactText.uses(capped), "3 of 5 uses")
        capped.uses = 0
        capped.maxUses = nil
        XCTAssertEqual(GrantFactText.uses(capped), "Not used yet")
        capped.expiresAt = nil
        capped.active = true
        XCTAssertEqual(GrantFactText.ends(capped), "Never ends on its own")
        XCTAssertEqual(GrantFact.glyph(never: "Send or change anything"), .send)
        XCTAssertEqual(GrantFact.glyph(never: "Codes and passwords"), .vault)
    }

    func testEndedGrantsGroupByHowTheyEnded() {
        var expired = DemoData.grants(1_800_000_000)[0]
        expired.active = false
        expired.state = "expired"
        var usedUp = expired
        usedUp.id = "u"
        usedUp.state = "used_up"
        usedUp.maxUses = 3
        usedUp.uses = 3
        var revoked = expired
        revoked.id = "r"
        revoked.state = "revoked"
        let groups = EndedGroup.split([revoked, usedUp, expired])
        XCTAssertEqual(groups.map(\.kind), [.expired, .usedUp, .byYou])
        XCTAssertEqual(groups.map(\.kind.title), ["Expired", "Used up", "Ended by you"])
        XCTAssertTrue(EndedGroup.line(usedUp).hasSuffix("3 of 3 uses"))
        XCTAssertFalse(EndedGroup.line(revoked).contains("·"), "ended by you: who only")
        XCTAssertEqual(EndedGroup.split([expired]).count, 1, "empty groups are left out")
    }
}
