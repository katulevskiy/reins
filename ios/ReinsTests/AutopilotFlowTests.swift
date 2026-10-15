import XCTest
@testable import Reins

/// Autopilot's screen model against the `-demo` core (the Android app's `AutopilotFlowTest`): modes and their
/// confirmations, the model download, profiles, "Try it", a connection's own mode and profile; what reaches the core,
/// what the user sees, which feedback plays.
@MainActor
final class AutopilotFlowTests: XCTestCase {
    private var feedback: EventFeedback!
    private var owner: AnswerAuthenticator!
    private var network: FakeNetwork!
    private var surroundings: FakeSurroundings!
    private var core: DemoReinsCore!
    private var model: AppModel!
    private var ap: AutopilotModel!

    private func start(modelInstalled: Bool = true) async {
        feedback = EventFeedback()
        owner = AnswerAuthenticator()
        network = FakeNetwork()
        surroundings = FakeSurroundings()
        core = DemoReinsCore(modelInstalled: modelInstalled, syncCap: 0.3)
        let downloads = ModelDownloads(core: core, feedback: feedback, network: network, surroundings: surroundings)
        model = AppModel(core: core, feedback: feedback, authenticator: owner, demo: true, modelDownloads: downloads)
        await model.refreshSession()
        ap = AutopilotModel(app: model)
        await ap.refresh()
    }

    private func until(_ timeout: TimeInterval = 8, _ condition: () -> Bool) async {
        let end = Date().addingTimeInterval(timeout)
        while !condition() && Date() < end { try? await Task.sleep(for: .milliseconds(50)) }
        XCTAssertTrue(condition(), "timed out")
    }

    // MARK: Modes

    func testPickingAutoSetsTheGlobalModeAndFeelsLikeMoreAutonomy() async {
        await start()
        XCTAssertEqual(model.autopilot?.mode, .manual)
        await ap.setMode(.auto)
        XCTAssertEqual(model.autopilot?.mode, .auto)
        XCTAssertTrue(feedback.lastWas(.autopilotOn))
        XCTAssertTrue(owner.asked.isEmpty, "Auto needs no confirmation")
        await ap.setMode(.manual)
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertTrue(feedback.lastWas(.autopilotOff))
    }

    /// Manual, Auto, Manual, Auto: each pick is the one in force, on screen and in the core, every time; tapped faster
    /// than the core answers, the last pick wins and nothing is left in between.
    func testManualAndAutoSwitchBackAndForthAndTheLastPickWins() async throws {
        await start()
        for mode in [AutopilotMode.auto, .manual, .auto] {
            await ap.setMode(mode)
            XCTAssertEqual(model.autopilot?.mode, mode)
            let read = try await core.autopilotSettings()
            XCTAssertEqual(read.mode, mode)
            XCTAssertNil(ap.error)
        }
        let picks: [AutopilotMode] = [.manual, .auto, .manual, .auto]
        var changes: [Task<Void, Never>] = []
        for mode in picks {
            changes.append(Task { await self.ap.setMode(mode) })
        }
        for change in changes { await change.value }
        XCTAssertEqual(model.autopilot?.mode, .auto)
        let read = try await core.autopilotSettings()
        XCTAssertEqual(read.mode, .auto, "the core got them in order")
        XCTAssertFalse(ap.busy)
    }

    func testBypassIsConfirmedByTheOwnerThenRunsAndStopsBackToTheModeItInterrupted() async throws {
        await start()
        await ap.setMode(.bypass, minutes: 30)
        XCTAssertEqual(owner.asked, ["Turn on Bypass for every AI"])
        XCTAssertEqual(model.autopilot?.mode, .bypass)
        let until = try XCTUnwrap(model.autopilot?.bypassUntil)
        XCTAssertEqual(Double(until) - Date().timeIntervalSince1970, 30 * 60, accuracy: 5)
        XCTAssertTrue(feedback.played(.bypassOn))
        await ap.stopBypass()
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertNil(model.autopilot?.bypassUntil)
        XCTAssertTrue(feedback.lastWas(.bypassOff))
    }

    func testARefusedConfirmationChangesNothing() async {
        await start()
        owner.answer = false
        await ap.setMode(.bypass, minutes: 15)
        await ap.startBypass(minutes: nil)
        await ap.startBypass(connectionId: "c1", minutes: 15, label: "Claude")
        XCTAssertEqual(owner.asked.count, 3)
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertEqual(ap.modeOf("c1"), .auto, "an AI's own bypass needs the owner too")
        XCTAssertNil(model.autopilot?.bypassUntil)
        XCTAssertFalse(feedback.played(.bypassOn))
    }

    func testABypassCanRunUntilTurnedOffAndShowsSoAtOnce() async throws {
        await start()
        let before = Int64(Date().timeIntervalSince1970)
        await ap.startBypass(minutes: nil)
        XCTAssertEqual(owner.asked, ["Turn on Bypass for every AI"])
        XCTAssertEqual(model.autopilot?.mode, .bypass)
        XCTAssertEqual(model.autopilot?.bypassUntil, AutopilotText.untilOff)
        let since = try XCTUnwrap(model.autopilot?.bypassSince)
        XCTAssertGreaterThanOrEqual(since, before)
        XCTAssertTrue(feedback.played(.bypassOn))
        let notice = try XCTUnwrap(AutopilotText.bypassNotice(model.autopilot, label: { $0 }, now: before + 86_400 * 3))
        XCTAssertEqual(notice.title, "Bypass on · until you stop it", "an endless bypass never runs out")
        await ap.stopBypass()
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertNil(model.autopilot?.bypassUntil)
        XCTAssertNil(model.autopilot?.bypassSince)
    }

    func testATimedBypassOfDaysStartsWithItsLength() async throws {
        await start()
        await ap.startBypass(minutes: 3 * 1_440)
        let until = try XCTUnwrap(model.autopilot?.bypassUntil)
        XCTAssertEqual(Double(until) - Date().timeIntervalSince1970, 3 * 86_400, accuracy: 5)
    }

    func testTheOptimisticBypassMatchesTheCoresRules() {
        var s = AutopilotSettings(
            mode: .lockdown, baseMode: .lockdown, bypassUntil: nil, bypassSince: nil, overnightWhenRisky: .wait, defaultProfileId: "p", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 1, downloadedBytes: 1, error: nil, runtimeReady: true),
            connections: []
        )
        s.applyBypass(minutes: nil, connectionId: nil, now: 1_000)
        XCTAssertEqual(s.mode, .bypass, "a bypass ends a lockdown")
        XCTAssertEqual(s.baseMode, .manual)
        XCTAssertEqual(s.bypassUntil, AutopilotText.untilOff)
        XCTAssertEqual(s.bypassSince, 1_000)
        s.applyBypass(minutes: 30, connectionId: "c1", now: 1_000)
        XCTAssertEqual(s.connections.first?.bypassUntil, 1_000 + 1_800)
        XCTAssertEqual(s.connections.first?.mode, .bypass)
        // A global bypass applies to an AI with a mode of its own, but not to one locked down.
        s.applyMode(.manual, minutes: 0, connectionId: "c2", now: 1_000)
        s.applyMode(.lockdown, minutes: 0, connectionId: "c3", now: 1_000)
        XCTAssertEqual(s.connections.first { $0.connectionId == "c2" }?.mode, .bypass)
        XCTAssertEqual(s.connections.first { $0.connectionId == "c3" }?.mode, .lockdown)
        s.applyMode(.auto, minutes: 0, connectionId: nil, now: 1_100)
        XCTAssertEqual(s.mode, .auto)
        XCTAssertNil(s.bypassSince)
    }

    // MARK: Overnight

    func testOvernightIsConfirmedByTheOwnerThenRunsAndStopsBackToTheModeItInterrupted() async throws {
        await start()
        OvernightReturn.forget(nil)
        await ap.setMode(.auto)
        await ap.startOvernight(minutes: 480)
        XCTAssertEqual(owner.asked, ["Turn on Overnight for every AI"])
        XCTAssertEqual(model.autopilot?.mode, .overnight)
        let until = try XCTUnwrap(model.autopilot?.overnightUntil)
        XCTAssertEqual(Double(until) - Date().timeIntervalSince1970, 8 * 3_600, accuracy: 5)
        XCTAssertNotNil(model.autopilot?.overnightSince)
        XCTAssertTrue(feedback.lastWas(.autopilotOn), "felt as more autonomy")
        XCTAssertEqual(ap.modeOf("c1"), .overnight, "a global Overnight covers an AI on Auto")
        await ap.stopOvernight()
        XCTAssertEqual(model.autopilot?.mode, .auto, "back to Auto, which it interrupted")
        XCTAssertNil(model.autopilot?.overnightUntil)
        XCTAssertTrue(feedback.lastWas(.autopilotOff))
        XCTAssertNil(ap.error)
    }

    func testARefusedOvernightChangesNothing() async {
        await start()
        owner.answer = false
        await ap.startOvernight(minutes: nil)
        await ap.setMode(.overnight)
        await ap.startOvernight(connectionId: "c1", minutes: 60, label: "Claude")
        XCTAssertEqual(owner.asked.count, 3, "picking Overnight as a mode asks the owner too")
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertEqual(ap.modeOf("c1"), .auto)
        XCTAssertNil(model.autopilot?.overnightUntil)
    }

    func testOvernightCanRunUntilTurnedOff() async throws {
        await start()
        let before = Int64(Date().timeIntervalSince1970)
        await ap.startOvernight(minutes: nil)
        XCTAssertEqual(model.autopilot?.mode, .overnight)
        XCTAssertEqual(model.autopilot?.overnightUntil, AutopilotText.untilOff)
        let since = try XCTUnwrap(model.autopilot?.overnightSince)
        XCTAssertGreaterThanOrEqual(since, before)
        await ap.stopOvernight()
        XCTAssertNotEqual(model.autopilot?.mode, .overnight)
    }

    func testAnAIsOwnOvernightStopsBackToItsOwnMode() async {
        await start()
        OvernightReturn.forget("c1")
        await ap.startOvernight(connectionId: "c1", minutes: 60, label: "Claude")
        XCTAssertEqual(owner.asked, ["Turn on Overnight for Claude"])
        XCTAssertEqual(ap.modeOf("c1"), .overnight)
        XCTAssertEqual(model.autopilot?.mode, .manual, "every other AI is as it was")
        await ap.stopOvernight("c1")
        XCTAssertEqual(ap.modeOf("c1"), .auto)
        XCTAssertNil(model.autopilot?.connections.first { $0.connectionId == "c1" }?.overnightUntil)
    }

    func testStoppingABypassOverAnOvernightLetsTheOvernightCarryOn() async throws {
        await start()
        OvernightReturn.forget(nil)
        await ap.setMode(.auto)
        await ap.startOvernight(minutes: 120)
        let end = try XCTUnwrap(model.autopilot?.overnightUntil)
        await ap.startBypass(minutes: 15)
        XCTAssertEqual(model.autopilot?.mode, .bypass, "a bypass goes over it")
        await ap.stopBypass()
        XCTAssertEqual(model.autopilot?.mode, .overnight)
        let resumed = try XCTUnwrap(model.autopilot?.overnightUntil)
        XCTAssertEqual(Double(resumed), Double(end), accuracy: 60, "until its own end")
        await ap.stopOvernight()
        XCTAssertEqual(model.autopilot?.mode, .auto, "and it still returns to what it interrupted")
    }

    func testWhatOvernightDoesWithWhatIsTooRiskyIsASetting() async throws {
        await start()
        XCTAssertEqual(model.autopilot?.overnightWhenRisky, .wait, "it waits for the user by default")
        await ap.setOvernightWhenRisky(.deny)
        XCTAssertEqual(model.autopilot?.overnightWhenRisky, .deny)
        let read = try await core.autopilotSettings()
        XCTAssertEqual(read.overnightWhenRisky, .deny)
        XCTAssertTrue(owner.asked.isEmpty, "a stricter or looser hold needs no owner check: Overnight itself had one")
    }

    func testTheOptimisticOvernightMatchesTheCoresRules() {
        var s = AutopilotSettings(
            mode: .auto, baseMode: .auto, bypassUntil: nil, bypassSince: nil, overnightWhenRisky: .wait, defaultProfileId: "p", wifiOnly: true,
            model: ModelStatus(state: .installed, id: "m", label: "m", version: "1", sizeBytes: 1, downloadedBytes: 1, error: nil, runtimeReady: true),
            connections: []
        )
        func mode(_ id: String) -> AutopilotMode? { s.connections.first { $0.connectionId == id }?.mode }
        s.applyMode(.manual, minutes: 0, connectionId: "kept", now: 1_000)
        s.applyMode(.auto, minutes: 0, connectionId: "own", now: 1_000)
        s.applyOvernight(minutes: 60, connectionId: nil, now: 1_000)
        XCTAssertEqual(s.mode, .overnight)
        XCTAssertEqual(s.overnightUntil, 1_000 + 3_600)
        XCTAssertEqual(s.overnightSince, 1_000)
        XCTAssertEqual(mode("kept"), .manual, "a global Overnight never loosens an AI kept on Manual")
        XCTAssertEqual(mode("own"), .overnight)
        s.applyBypass(minutes: 15, connectionId: nil, now: 1_000)
        XCTAssertEqual(s.mode, .bypass, "a bypass goes over it")
        s.applyBypassEnd(connectionId: nil, now: 1_100)
        XCTAssertEqual(s.mode, .overnight, "and it carries on after")
        s.applyMode(.auto, minutes: 0, connectionId: nil, now: 1_200)
        XCTAssertEqual(s.mode, .auto)
        XCTAssertNil(s.overnightUntil)
        XCTAssertNil(s.overnightSince)
        s.applyOvernight(minutes: nil, connectionId: "kept", now: 1_300)
        XCTAssertEqual(mode("kept"), .overnight, "an AI's own Overnight applies whatever it was kept on")
        XCTAssertEqual(s.connections.first { $0.connectionId == "kept" }?.overnightUntil, AutopilotText.untilOff)
        s.applyMode(.lockdown, minutes: 0, connectionId: nil, now: 1_400)
        XCTAssertEqual(mode("kept"), .lockdown, "a global lockdown beats it")
    }

    func testOvernightRemembersTheModeItInterruptedForStop() throws {
        let suite = "overnight-return-test"
        let d = try XCTUnwrap(UserDefaults(suiteName: suite))
        d.removePersistentDomain(forName: suite)
        defer { d.removePersistentDomain(forName: suite) }
        XCTAssertEqual(OvernightReturn.mode(nil, defaults: d), .manual, "every AI goes back to the default")
        XCTAssertNil(OvernightReturn.mode("c1", defaults: d), "an AI goes back to following every AI")
        OvernightReturn.remember(.auto, connectionId: nil, defaults: d)
        OvernightReturn.remember(.overnight, connectionId: nil, defaults: d)
        XCTAssertEqual(OvernightReturn.mode(nil, defaults: d), .auto, "a restart keeps what it first interrupted")
        OvernightReturn.remember(.manual, connectionId: "c1", defaults: d)
        XCTAssertEqual(OvernightReturn.mode("c1", defaults: d), .manual)
        OvernightReturn.remember(.lockdown, connectionId: "c1", defaults: d)
        XCTAssertNil(OvernightReturn.mode("c1", defaults: d), "never back to a lockdown it ended")
        OvernightReturn.forget(nil, defaults: d)
        XCTAssertEqual(OvernightReturn.mode(nil, defaults: d), .manual)
    }

    func testLockdownDeniesWhatWaitsAndEndsBackToThePickedMode() async {
        await start()
        let waiting = model.pending.filter { $0.kind == .request }.count
        XCTAssertGreaterThan(waiting, 0)
        await ap.setMode(.lockdown)
        XCTAssertTrue(owner.asked.isEmpty, "Lock down is a plain tap: it only holds requests back")
        XCTAssertEqual(model.autopilot?.mode, .lockdown)
        XCTAssertTrue(feedback.lastWas(.lockdownOn))
        await model.refreshPending()
        XCTAssertTrue(model.pending.allSatisfy { $0.kind != .request }, "every waiting request was denied")
        XCTAssertTrue(model.pending.contains { $0.kind == .pairing }, "new connections still reach the user")
        await ap.endLockdown()
        XCTAssertEqual(model.autopilot?.mode, .manual, "the mode before it was replaced by Lockdown, so Manual")
        XCTAssertTrue(feedback.lastWas(.lockdownOff))
    }

    func testWithoutTheModelTheModeIsManual() async {
        await start(modelInstalled: false)
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertFalse(ap.modelReady)
    }

    // MARK: The model

    func testTheModelDownloadsOnWifiWithProgressAndAKeepAlive() async {
        await start(modelInstalled: false)
        XCTAssertFalse(ap.needsMobileDataConsent())
        ap.download()
        XCTAssertEqual(ap.downloadJob, .running)
        XCTAssertEqual(surroundings.log.first, "background")
        await until { model.modelDownloads.downloaded > 0 }
        XCTAssertEqual(ap.model?.state, .downloading)
        XCTAssertTrue(surroundings.log.contains("activity"))
        await until { ap.downloadJob == .idle }
        await until { ap.modelReady }
        XCTAssertEqual(ap.model?.state, .installed)
        XCTAssertEqual(model.autopilot?.mode, .manual, "still Manual, now with hints")
        XCTAssertTrue(feedback.lastWas(.grantCreated))
        XCTAssertEqual(Array(surroundings.log.suffix(2)), ["ended:ok", "foreground"])
    }

    func testOnMobileDataTheDownloadAsksFirstAndWaitsForWifi() async {
        await start(modelInstalled: false)
        network.set(.cellular)
        XCTAssertTrue(ap.needsMobileDataConsent())
        ap.download()
        XCTAssertEqual(ap.downloadJob, .waiting)
        XCTAssertTrue(ap.waitingForWifi)
        XCTAssertEqual(AutopilotText.modelState(ap.model!, waitingForNetwork: true, wifiOnly: ap.waitingForWifi), "Waiting for Wi-Fi")
        network.set(.unmetered)
        XCTAssertEqual(ap.downloadJob, .running, "Wi-Fi came back")
        await until { ap.downloadJob == .idle }
        await until { ap.modelReady }
    }

    func testMobileDataCanBeUsedOnceOrAllowedForGood() async {
        await start(modelInstalled: false)
        network.set(.cellular)
        ap.downloadNow()
        XCTAssertEqual(ap.downloadJob, .running)
        await until { ap.downloadJob == .idle }

        await ap.deleteModel()
        await ap.setWifiOnly(false)
        XCTAssertEqual(model.autopilot?.wifiOnly, false)
        XCTAssertFalse(ap.needsMobileDataConsent())
        ap.download()
        XCTAssertEqual(ap.downloadJob, .running)
        await until { ap.downloadJob == .idle }
    }

    func testOfflineTheDownloadWaitsForAConnectionAndCanBeCancelled() async {
        await start(modelInstalled: false)
        network.set(.offline)
        ap.download()
        XCTAssertEqual(ap.downloadJob, .waiting)
        ap.cancelDownload()
        XCTAssertEqual(ap.downloadJob, .idle)
        network.set(.unmetered)
        XCTAssertEqual(ap.downloadJob, .idle, "a cancelled download does not start by itself")
        XCTAssertFalse(ap.modelReady)
    }

    func testTheModelCanBeDeleted() async {
        await start()
        await ap.deleteModel()
        XCTAssertEqual(model.autopilot?.model.state, .notInstalled)
        XCTAssertTrue(feedback.played(.revoked))
    }

    // MARK: Profiles

    func testAProfileTakesAPresetAndAClassLock() async throws {
        await start()
        let personal = try XCTUnwrap(ap.profile("personal"))
        XCTAssertEqual(AutopilotText.classStatus(try XCTUnwrap(personal.classes.first { $0.classKey == "github/write/push" })), "Approves on its own")
        await ap.setPreset("personal", .cautious)
        XCTAssertEqual(ap.profile("personal")?.preset, .cautious)
        XCTAssertEqual(ap.profile("personal")?.approveAt, 0.98)
        await ap.setClassLock("personal", "gmail/read", locked: false)
        XCTAssertTrue(feedback.lastWas(.autopilotOn), "unlocking is felt as more autonomy")
        XCTAssertEqual(ap.profile("personal")?.classes.first { $0.classKey == "gmail/read" }?.manual, true)
        await ap.setClassLock("personal", "gmail/read", locked: true)
        XCTAssertEqual(ap.profile("personal")?.classes.first { $0.classKey == "gmail/read" }?.manual, false)
    }

    func testCustomThresholdsAreClampedSavedAndReplayed() async throws {
        await start()
        await ap.setThresholds("personal", approveAt: 0.93, denyAt: 0.5)
        let personal = try XCTUnwrap(ap.profile("personal"))
        XCTAssertEqual(personal.preset, .custom)
        XCTAssertEqual(personal.approveAt, 0.93, accuracy: 0.0001)
        XCTAssertEqual(personal.denyAt, 0.8, accuracy: 0.0001, "80% at least")
        await ap.loadReplay("personal", approveAt: 0.93, denyAt: 0.88)
        let strict = try XCTUnwrap(ap.replay)
        XCTAssertEqual(strict.days, 30)
        await ap.loadReplay("personal", approveAt: 0.82, denyAt: 0.82)
        let loose = try XCTUnwrap(ap.replay)
        XCTAssertGreaterThan(loose.decided, strict.decided, "lower thresholds decide more")
        XCTAssertEqual(loose.decided + loose.asked, strict.decided + strict.asked)
    }

    func testThisWeeksNumbersAreRead() async throws {
        await start()
        await ap.loadStats()
        let stats = try XCTUnwrap(ap.stats)
        XCTAssertEqual(stats.days, 7)
        XCTAssertGreaterThan(stats.scored, 0)
        XCTAssertNotNil(stats.matched)
        XCTAssertEqual(stats.kindsOnAuto, 1, "Personal's push")
    }

    func testAProfileCanBeCreatedMadeDefaultResetAndDeleted() async throws {
        await start()
        let made = await ap.createProfile(name: "  Side project ", icon: "🚀")
        let id = try XCTUnwrap(made)
        XCTAssertEqual(ap.profile(id)?.name, "Side project")
        XCTAssertEqual(ap.profile(id)?.icon, "🚀")
        await ap.makeDefault(id)
        XCTAssertEqual(ap.profile(id)?.isDefault, true)
        XCTAssertEqual(model.autopilot?.defaultProfileId, id)
        XCTAssertEqual(ap.notice, "Side project is now the default profile.")
        await ap.renameProfile(id, name: "Side", icon: nil)
        XCTAssertEqual(ap.profile(id)?.name, "Side")
        await ap.resetProfile("personal")
        XCTAssertEqual(ap.profile("personal")?.memoryCount, 0)
        XCTAssertTrue(ap.notice?.contains("Forgotten") ?? false)
        let deleted = await ap.deleteProfile(id)
        XCTAssertTrue(deleted)
        XCTAssertNil(ap.profile(id))
        let blank = await ap.createProfile(name: "   ", icon: nil)
        XCTAssertNil(blank, "a blank name makes nothing")
    }

    func testTheLastProfileCannotBeDeletedAndSaysWhy() async {
        await start()
        _ = await ap.deleteProfile("work")
        let deleted = await ap.deleteProfile("personal")
        XCTAssertFalse(deleted)
        XCTAssertEqual(ap.error, "The last profile cannot be deleted.")
        XCTAssertTrue(feedback.lastWas(.error))
    }

    // MARK: Try it

    func testTryItShowsTheVerdictAndAnotherProfileCanBeTried() async {
        await start()
        await ap.evaluate(profileId: nil, situation: AutopilotText.examples[0].situation)
        XCTAssertEqual(ap.evaluation?.verdict, .approve)
        XCTAssertEqual(ap.evaluation?.profileId, "personal")
        XCTAssertFalse(ap.evaluation?.neighbours.isEmpty ?? true)
        XCTAssertTrue(feedback.lastWas(.refresh))
        await ap.evaluate(profileId: "work", situation: "connection: x\nservice: vault\noperation: Get a password")
        XCTAssertEqual(ap.evaluation?.verdict, .deny)
        XCTAssertEqual(ap.evaluation?.profileName, "Work")
        ap.clearEvaluation()
        XCTAssertNil(ap.evaluation)
    }

    // MARK: A connection's own mode and profile

    func testAConnectionGetsItsOwnModeAndProfile() async {
        await start()
        await ap.setMode(.auto, connectionId: "c2")
        XCTAssertEqual(model.autopilot?.connections.first { $0.connectionId == "c2" }?.baseMode, .auto)
        await ap.assignProfile("c2", "work")
        XCTAssertEqual(model.autopilot?.connections.first { $0.connectionId == "c2" }?.profileId, "work")
        XCTAssertTrue(ap.profile("work")?.connections.contains("c2") ?? false)
        await ap.setMode(nil, connectionId: "c2")
        XCTAssertNil(model.autopilot?.connections.first { $0.connectionId == "c2" }?.baseMode)
    }

    func testAConnectionBypassIsConfirmedNamingTheAIAndStopsBackToItsOwnMode() async {
        await start()
        // Claude (c1) has its own Auto mode in the demo.
        await ap.startBypass(connectionId: "c1", minutes: 15)
        XCTAssertEqual(owner.asked, ["Turn on Bypass for Claude"])
        XCTAssertEqual(ap.modeOf("c1"), .bypass)
        XCTAssertEqual(ap.modeOf(nil), .manual, "other AIs are not affected")
        await ap.stopBypass("c1")
        XCTAssertEqual(ap.modeOf("c1"), .auto)
    }

    func testAJustPairedConnectionCannotBeBypassedAndSaysWhy() async throws {
        await start()
        try await core.answerPairing(pairingId: "pair1", approve: true, chosenCode: 42, label: nil)
        await model.refreshConnections()
        let fresh = try XCTUnwrap(model.connections.first { $0.label == "Gemini" })
        let refusal = try XCTUnwrap(ap.bypassRefusal(fresh.id))
        XCTAssertTrue(refusal.contains("less than 10 minutes"))
        XCTAssertTrue(refusal.contains("10 min"))
        XCTAssertNil(ap.bypassRefusal("c1"), "an old connection can be bypassed")
        XCTAssertNil(ap.bypassRefusal(nil), "so can every AI")
    }
}

// MARK: Fakes

/// Records what played; an event is its cue followed by its haptic (`Feedback.play`).
private final class EventFeedback: Feedback {
    private var log: [String] = []
    func haptic(_ haptic: Haptic) { log.append("h:\(haptic)") }
    func cue(_ cue: Cue, step: Int) { log.append("c:\(cue)") }
    func cueUnlessRecent(_ cue: Cue) { log.append("c:\(cue)") }

    private func marks(_ e: FeedbackEvent) -> [String] { [e.cue.map { "c:\($0)" }, e.haptic.map { "h:\($0)" }].compactMap { $0 } }

    func lastWas(_ e: FeedbackEvent) -> Bool { log.suffix(marks(e).count) == ArraySlice(marks(e)) }

    func played(_ e: FeedbackEvent) -> Bool {
        let m = marks(e)
        guard log.count >= m.count else { return false }
        return (0...(log.count - m.count)).contains { Array(log[$0..<($0 + m.count)]) == m }
    }
}

private final class AnswerAuthenticator: Authenticating {
    var answer = true
    var asked: [String] = []
    func confirm(_ reason: String) async -> Bool {
        asked.append(reason)
        return answer
    }
}

@MainActor
private final class FakeNetwork: NetworkWatching {
    private(set) var current = NetworkState.unmetered
    var onChange: ((NetworkState) -> Void)?

    func set(_ state: NetworkState) {
        current = state
        onChange?(state)
    }
}

@MainActor
private final class FakeSurroundings: DownloadSurroundings {
    var log: [String] = []
    var inFront = true
    func beginBackground() { log.append("background") }
    func endBackground() { log.append("foreground") }
    func activityStarted(total: UInt64) { log.append("activity") }
    func activityProgress(downloaded: UInt64, total: UInt64) {}
    func activityEnded(failed: Bool) { log.append(failed ? "ended:failed" : "ended:ok") }
}
