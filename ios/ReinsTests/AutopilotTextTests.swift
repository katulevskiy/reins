import XCTest
@testable import Reins

/// Autopilot's words (the Android app's `AutopilotTextTest`).
final class AutopilotTextTests: XCTestCase {
    private func model(_ state: ModelState = .notInstalled, downloaded: UInt64 = 0, size: UInt64 = 0, error: String? = nil) -> ModelStatus {
        ModelStatus(state: state, id: "laya-approvals-base-v1", label: "Laya approvals (base)", version: "1", sizeBytes: size,
                    downloadedBytes: downloaded, error: error, runtimeReady: true)
    }

    private func settings(bypass: Int64? = nil, connections: [ConnectionAutopilot] = []) -> AutopilotSettings {
        AutopilotSettings(mode: bypass != nil ? .bypass : .assisted, baseMode: .assisted, bypassUntil: bypass, defaultProfileId: "personal",
                          wifiOnly: true, model: model(.installed), connections: connections)
    }

    private func classView(decisions: UInt32 = 12, approved: UInt32 = 11, denied: UInt32 = 1, accuracy: Float? = nil, autoApprove: Bool = false,
                           autoDeny: Bool = false, manual: Bool? = nil, toUnlock: UInt32 = 8) -> ClassView {
        ClassView(classKey: "github/write/push", label: "GitHub · write · push", decisions: decisions, approved: approved, denied: denied,
                  shadowAccuracy: accuracy, autoApprove: autoApprove, autoDeny: autoDeny, manual: manual, decisionsToUnlock: toUnlock)
    }

    private func suggestion(verdict: Verdict = .approve, pDeny: Float = 0.02, floor: Bool = false, novel: Bool = false, judged: Bool = true,
                            reason: String = "Like 4 times you approved: Push to a branch · dkat/reins") -> SuggestionView {
        SuggestionView(requestId: "req1", verdict: verdict, mode: .assisted, pApprove: 0.97, pDeny: pDeny, confidence: 0.91, reason: reason,
                       neighbours: [], profileId: "personal", profileName: "Personal", classKey: "github/write/push", novel: novel,
                       floor: floor, judged: judged)
    }

    private func decision(_ verdict: Verdict = .approve, by: String = "autopilot") -> AutoDecisionView {
        AutoDecisionView(requestId: "req1", kind: .request, connectionId: "c1", connectionLabel: "Claude Code",
                         title: "Push to a branch · dkat/reins", verdict: verdict, decidedBy: by, pApprove: 0.97, confidence: 0.91, activityId: 42)
    }

    func testSwitchingModesSoundsLikeWhatItLetsHappen() {
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .manual, to: .auto), .autopilotOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .auto, to: .assisted), .autopilotOff)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .auto, to: .bypass), .bypassOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .bypass, to: .manual), .bypassOff)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .bypass, to: .lockdown), .lockdownOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .lockdown, to: .auto), .lockdownOff)
        XCTAssertNil(AutopilotFeedback.modeChange(from: .auto, to: .auto))
    }

    func testTimeLeftReadsInWholeMinutesAndAsAClock() {
        XCTAssertEqual(AutopilotText.minutesLeft(until: 1_000 + 41 * 60 + 5, now: 1_000), "42 min left")
        XCTAssertEqual(AutopilotText.minutesLeft(until: 1_010, now: 1_000), "1 min left")
        XCTAssertEqual(AutopilotText.clock(until: 1_000 + 14 * 60 + 5, now: 1_000), "14:05")
        XCTAssertEqual(AutopilotText.clock(until: 900, now: 1_000), "0:00")
    }

    func testTheBypassRingMeasuresAgainstTheLengthMostLikelyChosen() {
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 14 * 60), 15 * 60)
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 16 * 60), 30 * 60)
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 45 * 60), 60 * 60)
        XCTAssertEqual(AutopilotText.bypassChoice(60), "1 hour")
        XCTAssertEqual(AutopilotText.bypassChoice(15), "15 min")
    }

    func testTheBypassNoticeCoversTheGlobalBypassAndEachConnections() throws {
        XCTAssertNil(AutopilotText.bypassNotice(settings(), label: { $0 }, now: 1_000))
        XCTAssertNil(AutopilotText.bypassNotice(settings(bypass: 900), label: { $0 }, now: 1_000), "an ended bypass shows nothing")
        let global = try XCTUnwrap(AutopilotText.bypassNotice(settings(bypass: 1_600), label: { $0 }, now: 1_000))
        XCTAssertTrue(global.global)
        XCTAssertEqual(global.title, "Bypass on · 10 min left")
        XCTAssertTrue(global.text.contains("every AI"))
        let one = try XCTUnwrap(AutopilotText.bypassNotice(
            settings(connections: [ConnectionAutopilot(connectionId: "c1", baseMode: nil, bypassUntil: 1_120, mode: .bypass, profileId: "personal")]),
            label: { $0 == "c1" ? "Claude Code" : "?" },
            now: 1_000
        ))
        XCTAssertTrue(one.text.contains("Claude Code"))
        XCTAssertEqual(one.connectionIds, ["c1"])
        XCTAssertEqual(one.until, 1_120)
    }

    func testAutomaticDecisionsAreTitledByWhoDecided() {
        XCTAssertEqual(AutopilotText.decisionTitle(decision()), "Autopilot approved")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(.deny)), "Autopilot denied")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(by: "bypass")), "Approved by Bypass")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(.deny, by: "lockdown")), "Denied by Lockdown")
        XCTAssertEqual(AutopilotText.decisionDetail(decision()), "91% sure")
        XCTAssertNil(AutopilotText.decisionDetail(decision(by: "bypass")))
        XCTAssertEqual(AutopilotText.decisionText(decision()), "Push to a branch · dkat/reins — Claude Code")
    }

    func testSuggestionsSayWhatAutopilotWouldDoAndWhyItHoldsBack() {
        XCTAssertEqual(AutopilotText.suggestionHeadline(suggestion()), "Autopilot would approve · 97%")
        XCTAssertEqual(AutopilotText.suggestionHeadline(suggestion(verdict: .deny, pDeny: 0.9)), "Autopilot would deny · 90%")
        XCTAssertEqual(AutopilotText.suggestionHeadline(suggestion(floor: true, judged: false)), "Autopilot always asks you for this")
        XCTAssertEqual(AutopilotText.suggestionNotes(suggestion(novel: true)).count, 1)
        XCTAssertTrue(AutopilotText.suggestionNotes(suggestion(novel: true))[0].contains("never approved"))
        XCTAssertTrue(AutopilotText.suggestionNotes(suggestion(judged: false, reason: "No model on this phone")).contains("No model on this phone"))
    }

    func testAClassFillsItsRingWithAnswersAndIsFullOnceItRunsByItself() {
        XCTAssertEqual(AutopilotText.unlockProgress(classView(decisions: 12, toUnlock: 8)), 0.6, accuracy: 0.001)
        XCTAssertEqual(AutopilotText.unlockProgress(classView(autoApprove: true, toUnlock: 0)), 1)
        XCTAssertEqual(AutopilotText.classStatus(classView()), "8 more decisions to unlock")
        XCTAssertEqual(AutopilotText.classStatus(classView(toUnlock: 1)), "1 more decision to unlock")
        XCTAssertEqual(AutopilotText.classStatus(classView(manual: false)), "Locked by you · always asks")
        XCTAssertEqual(AutopilotText.classStatus(classView(autoApprove: true, toUnlock: 0)), "Approves on its own")
        XCTAssertEqual(AutopilotText.classStatus(classView(autoApprove: true, manual: true, toUnlock: 0)), "Unlocked by you · approves on its own")
        XCTAssertEqual(AutopilotText.classNumbers(classView(accuracy: 0.96)), "11 approved · 1 denied · 96% accurate")
    }

    func testAModelThatFailsItsCheckSaysSoPlainly() {
        XCTAssertTrue(AutopilotText.modelError("sha-256 of model.onnx does not match").contains("did not match"))
        XCTAssertTrue(AutopilotText.modelError("sha-256 of model.onnx does not match").contains("Reins"))
        XCTAssertEqual(AutopilotText.modelError("connection reset"), "Connection reset.")
        XCTAssertTrue(AutopilotText.modelError(nil).contains("Try again"))
    }

    func testTheModelCardStatesSizesInMegabytesAndKnowsTheBaseCheckpointsRange() throws {
        XCTAssertEqual(AutopilotText.modelSize(model()), "about 300–450 MB")
        let downloading = model(.downloading, downloaded: 103_000_000, size: 412_000_000)
        XCTAssertEqual(AutopilotText.modelState(downloading, waitingForNetwork: false, wifiOnly: true), "Downloading · 103 MB of 412 MB")
        XCTAssertEqual(try XCTUnwrap(AutopilotText.fraction(downloading)), 0.25, accuracy: 0.001)
        XCTAssertEqual(AutopilotText.modelState(model(), waitingForNetwork: true, wifiOnly: true), "Waiting for Wi-Fi")
        XCTAssertEqual(AutopilotText.modelState(model(), waitingForNetwork: true, wifiOnly: false), "Waiting for a connection")
        XCTAssertEqual(AutopilotText.modelState(model(.installed, size: 1), waitingForNetwork: false, wifiOnly: true), "Installed · ready")
        XCTAssertNil(AutopilotText.fraction(downloaded: 5, total: 0))
    }

    func testTheExamplesUseTheSituationFormat() {
        for e in AutopilotText.examples {
            XCTAssertTrue(e.situation.hasPrefix("connection: "), e.title)
            XCTAssertTrue(e.situation.split(separator: "\n").allSatisfy { $0.contains(": ") || $0 == "--- written by the AI ---" }, e.title)
        }
    }

    func testModesHaveNamesLinesAndSymbolsByKey() {
        XCTAssertEqual(AutopilotText.modes.map(AutopilotText.key), AutopilotText.modeKeys)
        XCTAssertEqual(AutopilotText.modes.map(AutopilotText.name), ["Manual", "Assisted", "Auto", "Bypass", "Lockdown"])
        XCTAssertEqual(AutopilotText.line(.lockdown), "Denies everything at once")
        XCTAssertEqual(AutopilotText.name(key: "nonsense"), "Manual")
        XCTAssertTrue(AutopilotText.needsModel(.assisted))
        XCTAssertFalse(AutopilotText.needsModel(.bypass))
    }

    func testCoreReasonsReadAsSentences() {
        XCTAssertEqual(AutopilotModel.sentence("a connection paired less than 10 minutes ago cannot be put in bypass"),
                       "A connection paired less than 10 minutes ago cannot be put in bypass.")
        XCTAssertEqual(AutopilotModel.sentence("Done."), "Done.")
    }
}
