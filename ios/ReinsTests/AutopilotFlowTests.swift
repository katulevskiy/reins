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
        XCTAssertEqual(model.autopilot?.mode, .assisted)
        await ap.setMode(.auto)
        XCTAssertEqual(model.autopilot?.mode, .auto)
        XCTAssertTrue(feedback.lastWas(.autopilotOn))
        XCTAssertTrue(owner.asked.isEmpty, "Auto needs no confirmation")
        await ap.setMode(.manual)
        XCTAssertEqual(model.autopilot?.mode, .manual)
        XCTAssertTrue(feedback.lastWas(.autopilotOff))
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
        XCTAssertEqual(model.autopilot?.mode, .assisted)
        XCTAssertNil(model.autopilot?.bypassUntil)
        XCTAssertTrue(feedback.lastWas(.bypassOff))
    }

    func testARefusedConfirmationChangesNothing() async {
        await start()
        owner.answer = false
        await ap.setMode(.bypass, minutes: 15)
        await ap.setMode(.lockdown)
        XCTAssertEqual(owner.asked.count, 2)
        XCTAssertEqual(model.autopilot?.mode, .assisted)
        XCTAssertFalse(feedback.played(.bypassOn))
        XCTAssertFalse(feedback.played(.lockdownOn))
    }

    func testLockdownIsConfirmedDeniesWhatWaitsAndEndsBackToThePickedMode() async {
        await start()
        let waiting = model.pending.filter { $0.kind == .request }.count
        XCTAssertGreaterThan(waiting, 0)
        await ap.setMode(.lockdown)
        XCTAssertEqual(owner.asked, ["Lock down every AI"])
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
        XCTAssertEqual(model.autopilot?.mode, .assisted, "Assisted once a model is installed")
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
        await ap.setClassLock("personal", "gmail/read", locked: false)
        XCTAssertTrue(feedback.lastWas(.autopilotOn), "unlocking is felt as more autonomy")
        XCTAssertEqual(ap.profile("personal")?.classes.first { $0.classKey == "gmail/read" }?.manual, true)
        await ap.setClassLock("personal", "gmail/read", locked: true)
        XCTAssertEqual(ap.profile("personal")?.classes.first { $0.classKey == "gmail/read" }?.manual, false)
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
        await ap.setMode(.bypass, minutes: 15, connectionId: "c1")
        XCTAssertEqual(owner.asked, ["Turn on Bypass for Claude"])
        XCTAssertEqual(ap.modeOf("c1"), .bypass)
        XCTAssertEqual(ap.modeOf(nil), .assisted, "other AIs are not affected")
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
