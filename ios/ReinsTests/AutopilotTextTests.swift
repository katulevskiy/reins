import XCTest
@testable import Reins

/// Autopilot's words (the Android app's `AutopilotTextTest`).
final class AutopilotTextTests: XCTestCase {
    private func model(_ state: ModelState = .notInstalled, downloaded: UInt64 = 0, size: UInt64 = 0, error: String? = nil,
                       update: Bool = false) -> ModelStatus {
        ModelStatus(state: state, id: "laya-approvals-base-v1", label: "Laya approvals (base)", version: "1", sizeBytes: size,
                    downloadedBytes: downloaded, error: error, runtimeReady: true, update: update)
    }

    private func settings(bypass: Int64? = nil, since: Int64? = nil, connections: [ConnectionAutopilot] = []) -> AutopilotSettings {
        AutopilotSettings(mode: bypass != nil ? .bypass : .manual, baseMode: .manual, bypassUntil: bypass, bypassSince: since,
                          overnightWhenRisky: .wait, defaultProfileId: "personal", wifiOnly: true, model: model(.installed), connections: connections)
    }

    private func connection(_ id: String, until: Int64?, since: Int64? = nil, profile: String = "personal") -> ConnectionAutopilot {
        ConnectionAutopilot(connectionId: id, baseMode: nil, bypassUntil: until, bypassSince: since, mode: until != nil ? .bypass : .manual,
                            profileId: profile)
    }

    private func profile(_ id: String = "personal", isDefault: Bool = true, memory: UInt32 = 140, connections: [String] = [],
                         classes: [ClassView] = []) -> ProfileView {
        ProfileView(id: id, name: id, icon: nil, preset: .custom, approveAt: 0.93, denyAt: 0.88, agreement: 0.94, isDefault: isDefault,
                    memoryCount: memory, connections: connections, classes: classes, trainedAt: nil)
    }

    private func classView(decisions: UInt32 = 12, approved: UInt32 = 11, denied: UInt32 = 1, accuracy: Float? = nil, autoApprove: Bool = false,
                           autoDeny: Bool = false, manual: Bool? = nil, toUnlock: UInt32 = 8) -> ClassView {
        ClassView(classKey: "github/write/push", label: "GitHub · write · push", decisions: decisions, approved: approved, denied: denied,
                  shadowAccuracy: accuracy, autoApprove: autoApprove, autoDeny: autoDeny, manual: manual, decisionsToUnlock: toUnlock)
    }

    private func suggestion(verdict: Verdict = .approve, pDeny: Float = 0.02, floor: Bool = false, novel: Bool = false, judged: Bool = true,
                            reason: String = "Like 4 times you approved: Push to a branch · dkat/reins") -> SuggestionView {
        SuggestionView(requestId: "req1", verdict: verdict, mode: .manual, pApprove: 0.97, pDeny: pDeny, confidence: 0.91, reason: reason,
                       neighbours: [], profileId: "personal", profileName: "Personal", classKey: "github/write/push", novel: novel,
                       floor: floor, judged: judged)
    }

    private func decision(_ verdict: Verdict = .approve, by: String = "autopilot") -> AutoDecisionView {
        AutoDecisionView(requestId: "req1", kind: .request, connectionId: "c1", connectionLabel: "Claude Code",
                         title: "Push to a branch · dkat/reins", verdict: verdict, decidedBy: by, pApprove: 0.97, confidence: 0.91, activityId: 42)
    }

    func testSwitchingModesSoundsLikeWhatItLetsHappen() {
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .manual, to: .auto), .autopilotOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .auto, to: .manual), .autopilotOff)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .auto, to: .bypass), .bypassOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .bypass, to: .manual), .bypassOff)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .bypass, to: .lockdown), .lockdownOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .lockdown, to: .auto), .lockdownOff)
        XCTAssertNil(AutopilotFeedback.modeChange(from: .auto, to: .auto))
    }

    func testTimeLeftReadsInMinutesHoursAndDaysAndUntilOff() {
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 41 * 60 + 5, now: 1_000), "42 min left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_010, now: 1_000), "1 min left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 900, now: 1_000), "1 min left", "never negative")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 3_600, now: 1_000), "1 h left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 4 * 3_600 + 1, now: 1_000), "5 h left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 47 * 3_600, now: 1_000), "47 h left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 48 * 3_600, now: 1_000), "2 days left")
        XCTAssertEqual(AutopilotText.timeLeft(until: 1_000 + 2 * 86_400 + 1, now: 1_000), "3 days left")
        XCTAssertEqual(AutopilotText.timeLeft(until: AutopilotText.untilOff, now: 1_000), "until you stop it")
    }

    func testTheClockReadsMinutesHoursAndDaysAndInfinityWithoutAnEnd() {
        XCTAssertEqual(AutopilotText.clock(until: 1_000 + 14 * 60 + 5, now: 1_000), "14:05")
        XCTAssertEqual(AutopilotText.clock(until: 900, now: 1_000), "0:00")
        XCTAssertEqual(AutopilotText.clock(until: 1_000 + 5 * 3_600 + 4 * 60 + 9, now: 1_000), "5:04:09")
        XCTAssertEqual(AutopilotText.clock(until: 1_000 + 2 * 86_400 + 4 * 3_600 + 59, now: 1_000), "2d 4h")
        XCTAssertEqual(AutopilotText.clock(until: AutopilotText.untilOff, now: 1_000), "∞")
        XCTAssertEqual(AutopilotText.clock(until: AutopilotText.untilOff, now: 0), "∞")
    }

    func testLengthsReadAsTheLargestWholeUnit() {
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: nil), "Until off")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 15), "15 min")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 90), "90 min")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 60), "1 hour")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 240), "4 hours")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 1_440), "1 day")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 3 * 1_440), "3 days")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 10_080), "1 week")
        XCTAssertEqual(AutopilotText.lengthLabel(minutes: 3 * 10_080), "3 weeks")
        XCTAssertEqual(AutopilotText.bypassMinutes.map { AutopilotText.lengthLabel(minutes: $0) }, ["15 min", "1 hour", "1 day"])
        XCTAssertEqual(AutopilotText.bypassMoreMinutes.map { AutopilotText.lengthLabel(minutes: $0) }, ["4 hours", "1 week"])
    }

    func testACustomLengthStaysWithinWhatTheCoreTakes() {
        XCTAssertEqual(AutopilotText.customMinutes(count: 3, unit: .days), 3 * 1_440)
        XCTAssertEqual(AutopilotText.customMinutes(count: 0, unit: .hours), 60, "at least one")
        XCTAssertEqual(AutopilotText.customMinutes(count: 200, unit: .minutes), 99, "the stepper's largest")
        XCTAssertEqual(AutopilotText.customMinutes(count: 60, unit: .weeks), 52 * 10_080, "52 weeks at most")
        XCTAssertLessThanOrEqual(52 * 10_080, AutopilotText.maxBypassMinutes, "within a year")
        XCTAssertEqual(AutopilotText.customMinutes(count: 99, unit: .days), 99 * 1_440)
        XCTAssertTrue(AutopilotText.countAndUnit(minutes: 3 * 1_440) == (3, .days))
        XCTAssertTrue(AutopilotText.countAndUnit(minutes: 10_080) == (1, .weeks))
        XCTAssertTrue(AutopilotText.countAndUnit(minutes: 90) == (90, .minutes))
        XCTAssertTrue(AutopilotText.countAndUnit(minutes: 6_000) == (4, .days), "100 hours is past the stepper: the nearest days")
    }

    func testHowLongAnEndlessBypassHasRun() {
        XCTAssertEqual(AutopilotText.ranFor(since: 1_000, now: 1_030), "just started")
        XCTAssertEqual(AutopilotText.ranFor(since: 1_000, now: 1_000 + 12 * 60 + 30), "12 min so far")
        XCTAssertEqual(AutopilotText.ranFor(since: 1_000, now: 1_000 + 5 * 3_600 + 59 * 60), "5 h so far")
        XCTAssertEqual(AutopilotText.ranFor(since: 1_000, now: 1_000 + 2 * 86_400 + 23 * 3_600), "2 days so far")
        XCTAssertEqual(AutopilotText.ranFor(since: 2_000, now: 1_000), "just started", "never negative")
        XCTAssertEqual(AutopilotText.bypassLine(until: 2_000, since: 1_000, now: 1_500), "Approving without asking")
        XCTAssertEqual(AutopilotText.bypassLine(until: AutopilotText.untilOff, since: 1_000, now: 1_000 + 2 * 86_400), "On until you stop it · 2 days so far")
        XCTAssertEqual(AutopilotText.bypassLine(until: AutopilotText.untilOff, since: nil, now: 1_000), "On until you stop it")
    }

    func testTheBypassRingMeasuresFromItsStartOrTheLengthMostLikelyChosen() {
        XCTAssertEqual(AutopilotText.bypassFraction(until: AutopilotText.untilOff, since: 1_000, now: 99_999), 1)
        XCTAssertEqual(AutopilotText.bypassFraction(until: 2_000, since: 1_000, now: 1_250), 0.75, accuracy: 0.001)
        XCTAssertEqual(AutopilotText.bypassFraction(until: 2_000, since: 1_000, now: 3_000), 0)
        XCTAssertEqual(AutopilotText.bypassFraction(until: 2_000, since: 1_000, now: 500), 1, "clamped")
        // Without its start: the shortest offered length that fits.
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 14 * 60), 15 * 60)
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 16 * 60), 60 * 60)
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 5 * 3_600), 86_400)
        XCTAssertEqual(AutopilotText.bypassLength(leftSeconds: 3 * 86_400), 3 * 86_400, "longer than any: full")
        XCTAssertEqual(AutopilotText.bypassFraction(until: 1_000 + 30 * 60, since: nil, now: 1_000), 0.5, accuracy: 0.001)
    }

    func testTheBypassNoticeCoversTheGlobalBypassAndEachConnections() throws {
        XCTAssertNil(AutopilotText.bypassNotice(settings(), label: { $0 }, now: 1_000))
        XCTAssertNil(AutopilotText.bypassNotice(settings(bypass: 900), label: { $0 }, now: 1_000), "an ended bypass shows nothing")
        let global = try XCTUnwrap(AutopilotText.bypassNotice(settings(bypass: 1_600, since: 400), label: { $0 }, now: 1_000))
        XCTAssertTrue(global.global)
        XCTAssertEqual(global.title, "Bypass on · 10 min left")
        XCTAssertEqual(global.text, "For every AI. New connections and new phones still ask.")
        XCTAssertEqual(global.since, 400)
        XCTAssertFalse(global.untilOff)
        let one = try XCTUnwrap(AutopilotText.bypassNotice(
            settings(connections: [connection("c1", until: 1_120)]),
            label: { $0 == "c1" ? "Claude Code" : "?" },
            now: 1_000
        ))
        XCTAssertTrue(one.text.contains("Claude Code"))
        XCTAssertEqual(one.connectionIds, ["c1"])
        XCTAssertEqual(one.until, 1_120)
    }

    func testABypassUntilOffIsTitledWithoutACountdown() throws {
        let off = try XCTUnwrap(AutopilotText.bypassNotice(settings(bypass: AutopilotText.untilOff, since: 900), label: { $0 }, now: 1_000))
        XCTAssertTrue(off.untilOff)
        XCTAssertEqual(off.title, "Bypass on · until you stop it")
        XCTAssertEqual(off.since, 900)
        XCTAssertEqual(AutopilotText.bypassTitle(until: AutopilotText.untilOff, now: 1_000), "Bypass on · until you stop it")
        // A connection's endless bypass wins over a timed global one.
        let mixed = try XCTUnwrap(AutopilotText.bypassNotice(
            settings(bypass: 1_600, since: 400, connections: [connection("c1", until: AutopilotText.untilOff, since: 800)]),
            label: { $0 }, now: 1_000
        ))
        XCTAssertTrue(mixed.untilOff)
        XCTAssertEqual(mixed.since, 400, "the earliest start")
    }

    func testProfilesKindsAndThresholdsInWords() {
        XCTAssertEqual(AutopilotText.presets.map(AutopilotText.presetName), ["Cautious", "Balanced", "Relaxed", "Custom"])
        XCTAssertNil(AutopilotText.presetThresholds(.custom))
        XCTAssertEqual(AutopilotText.presetThresholds(.cautious)?.approveAt, 0.98)
        XCTAssertEqual(AutopilotText.presetLine(profile()), "Approves when 93% sure, denies when 88% sure")
        XCTAssertEqual(AutopilotText.thresholdPercent(0.5), 80)
        XCTAssertEqual(AutopilotText.thresholdPercent(1), 99)
        XCTAssertEqual(AutopilotText.thresholdPercent(0.934), 93)
        XCTAssertEqual(AutopilotModel.clampThreshold(0.7), 0.8, accuracy: 0.0001)
        XCTAssertEqual(AutopilotText.replayLine(ThresholdReplay(days: 30, decided: 41, asked: 12, undone: 0)),
                       "Last 30 days at these: 41 decided for you, 12 asked, 0 you'd have undone.")
        XCTAssertEqual(AutopilotText.replayParts(ThresholdReplay(days: 30, decided: 41, asked: 12, undone: 0)).filter(\.strong).map(\.text),
                       ["41 decided", "12 asked"])
        XCTAssertEqual(AutopilotText.share(nil), "—")
        XCTAssertEqual(AutopilotText.share(0.94), "94%")
        XCTAssertEqual(AutopilotText.kindLine(classView(decisions: 31, autoApprove: true, toUnlock: 0)), "On Auto · 31 decisions")
        XCTAssertEqual(AutopilotText.kindLine(classView(decisions: 1, autoApprove: true, toUnlock: 0)), "On Auto · 1 decision")
        XCTAssertEqual(AutopilotText.kindLine(classView(toUnlock: 8)), "8 more to unlock")
        XCTAssertEqual(AutopilotText.kindLine(classView(autoApprove: true, manual: false)), "Always asks")
        XCTAssertEqual(AutopilotText.kindLine(classView(toUnlock: 0)), "Learning")
    }

    func testAProfileNamesTheAIsThatLearnIntoIt() {
        let ais: [(id: String, label: String)] = [("c1", "Claude Code"), ("c2", "ChatGPT"), ("c3", "Cursor")]
        let s = settings(connections: [connection("c3", until: nil, profile: "work")])
        let personal = profile()
        let work = profile("work", isDefault: false, memory: 1, connections: ["c3"])
        XCTAssertEqual(AutopilotText.profileAis(personal, settings: s, connections: ais), ["Claude Code", "ChatGPT"])
        XCTAssertEqual(AutopilotText.profileAis(work, settings: s, connections: ais), ["Cursor"])
        XCTAssertEqual(AutopilotText.profileLine(personal, ais: ["Claude Code", "ChatGPT"]), "Claude Code, ChatGPT · 140 decisions")
        XCTAssertEqual(AutopilotText.profileLine(work, ais: []), "1 decision")
    }

    func testAutomaticDecisionsAreTitledByWhoDecided() {
        XCTAssertEqual(AutopilotText.decisionTitle(decision()), "Autopilot approved")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(.deny)), "Autopilot denied")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(by: "bypass")), "Approved by Bypass")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(.deny, by: "lockdown")), "Denied by Lockdown")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(by: "overnight")), "Approved by Overnight")
        XCTAssertEqual(AutopilotText.decisionTitle(decision(.deny, by: "overnight")), "Denied by Overnight")
        XCTAssertEqual(AutopilotText.decisionDetail(decision(by: "overnight")), "91% sure", "Overnight judges as Auto does")
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
        XCTAssertTrue(AutopilotText.modelError("sha-256 of model.onnx does not match").contains("app update"))
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

    func testAnOlderModelStillScoresWhileItsUpdateComes() {
        XCTAssertTrue(AutopilotText.modelUsable(model(.installed, update: true)))
        XCTAssertTrue(AutopilotText.modelUsable(model(.downloading, update: true)), "the older model scores until the newer one is in")
        XCTAssertFalse(AutopilotText.modelUsable(model(.downloading)), "a first download has nothing to score with yet")
        XCTAssertFalse(AutopilotText.modelUsable(nil))
    }

    /// A Gregorian calendar in UTC, so the days do not depend on where the tests run.
    private var utc: Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: "UTC")!
        return c
    }

    private func at(_ year: Int, _ month: Int, _ day: Int, _ hour: Int = 12) -> Date {
        utc.date(from: DateComponents(year: year, month: month, day: day, hour: hour))!
    }

    private func secs(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970) }

    func testWhenTheModelWasUpdatedCountsCalendarDays() {
        let now = at(2026, 10, 15, 14)
        func ago(_ date: Date) -> String? { AutopilotText.updatedAgo(secs(date), now: now, calendar: utc) }
        XCTAssertNil(AutopilotText.updatedAgo(nil, now: now, calendar: utc))
        XCTAssertEqual(ago(at(2026, 10, 15, 0)), "Updated today")
        XCTAssertEqual(ago(at(2026, 10, 15, 23)), "Updated today", "later today (a clock set back) is still today")
        XCTAssertEqual(ago(at(2026, 10, 14, 23)), "Updated yesterday", "15 hours ago, but yesterday")
        XCTAssertEqual(ago(at(2026, 10, 14, 0)), "Updated yesterday")
        XCTAssertEqual(ago(at(2026, 10, 13, 23)), "Updated 2 days ago")
        XCTAssertEqual(ago(at(2026, 10, 9)), "Updated 6 days ago")
        XCTAssertEqual(ago(at(2026, 10, 8)), "Updated Oct 8")
        XCTAssertEqual(ago(at(2026, 1, 3)), "Updated Jan 3")
        XCTAssertEqual(ago(at(2025, 12, 30)), "Updated Dec 30, 2025", "another year says which")
        XCTAssertEqual(ago(at(2026, 10, 20)), "Updated today", "a date ahead of the clock")
    }

    func testTheModelsRowShowsItsVersionAndWhenItWasUpdated() {
        let now = at(2026, 10, 15, 14)
        var m = model(.installed)
        m.version = "2"
        // The value on the right stays short; when it was updated is the row's second line.
        XCTAssertEqual(AutopilotText.modelVersion(m), "v2")
        XCTAssertEqual(AutopilotText.modelVersionSpoken(m), "Ready, version 2")
        XCTAssertNil(AutopilotText.updatedAgo(m.updatedAt, now: now, calendar: utc), "when it was put in use is not known: no line")
        m.updatedAt = secs(at(2026, 10, 15, 9))
        XCTAssertEqual(AutopilotText.updatedAgo(m.updatedAt, now: now, calendar: utc), "Updated today")
        m.updatedAt = secs(at(2026, 10, 12))
        XCTAssertEqual(AutopilotText.updatedAgo(m.updatedAt, now: now, calendar: utc), "Updated 3 days ago")
    }

    func testTheExamplesUseTheSituationFormat() {
        for e in AutopilotText.examples {
            XCTAssertTrue(e.situation.hasPrefix("connection: "), e.title)
            XCTAssertTrue(e.situation.split(separator: "\n").allSatisfy { $0.contains(": ") || $0 == "--- written by the AI ---" }, e.title)
        }
    }

    func testOvernightIsNamedOnceAndSaysWhatItDoes() {
        XCTAssertEqual(AutopilotText.overnightName, "Overnight")
        XCTAssertEqual(AutopilotText.name(.overnight), AutopilotText.overnightName)
        XCTAssertEqual(AutopilotText.name(key: "overnight"), AutopilotText.overnightName)
        XCTAssertEqual(AutopilotText.key(.overnight), "overnight")
        XCTAssertEqual(AutopilotText.symbol(.overnight), "moon.stars.fill")
        XCTAssertEqual(AutopilotText.line(.overnight), "Holds what is too risky")
        XCTAssertTrue(AutopilotText.needsModel(.overnight), "it decides like Auto")
        XCTAssertEqual(AutopilotText.overnightMinutes.map { AutopilotText.lengthLabel(minutes: $0) }, ["4 hours", "8 hours", "12 hours"])
        XCTAssertEqual(AutopilotText.overnightTitle(until: 1_000 + 5 * 3_600, now: 1_000), "Overnight on · 5 h left")
        XCTAssertEqual(AutopilotText.overnightLine(until: 9_000, since: 1_000, now: 1_000), "Holds what is too risky")
        XCTAssertEqual(AutopilotText.overnightLine(until: AutopilotText.untilOff, since: 1_000, now: 1_720), "On until you stop it · 12 min so far")
        XCTAssertEqual(AutopilotText.overnightText(scope: "every AI"), "For every AI. Holds what is too risky.")
    }

    func testOvernightSitsBetweenAutoAndBypassAndFeelsSo() {
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .auto, to: .overnight), .autopilotOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .overnight, to: .auto), .autopilotOff)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .overnight, to: .bypass), .bypassOn)
        XCTAssertEqual(AutopilotFeedback.modeChange(from: .overnight, to: .lockdown), .lockdownOn)
        XCTAssertGreaterThan(AutopilotText.autonomy(.overnight), AutopilotText.autonomy(.auto))
        XCTAssertLessThan(AutopilotText.autonomy(.overnight), AutopilotText.autonomy(.bypass))
    }

    func testOvernightsHelpSaysItDecidesLikeAutoWithoutARiskModel() {
        XCTAssertEqual(AutopilotText.overnightHelpText(riskRated: false), "This model does not rate risk, so Overnight decides like Auto.")
        XCTAssertTrue(AutopilotText.overnightHelpText(riskRated: true).contains("catastrophic"))
        XCTAssertTrue(AutopilotText.modeHelp.contains(AutopilotText.overnightName))
        XCTAssertEqual([AutopilotText.whenRiskyWait, AutopilotText.whenRiskyDeny], ["Wait for me", "Deny it"])
    }

    func testTheRiskReadsAsItsLevelAndWhy() {
        XCTAssertEqual(AutopilotText.riskLine(RiskView(level: .severe, pCatastrophic: 0.2, why: "force push", held: false)), "Risk: severe · force push")
        XCTAssertEqual(AutopilotText.riskLine(RiskView(level: .catastrophic, pCatastrophic: 0.9, why: "  ", held: true)), "Risk: catastrophic")
        XCTAssertEqual([RiskLevel.benign, .risky, .severe, .catastrophic].map(AutopilotText.riskWord), ["low", "moderate", "severe", "catastrophic"])
        XCTAssertEqual(AutopilotText.heldLine, "Held by Overnight")
    }

    func testModesHaveNamesLinesAndSymbolsByKey() {
        XCTAssertEqual(AutopilotText.modes.map(AutopilotText.key), AutopilotText.modeKeys)
        XCTAssertEqual(AutopilotText.modes.map(AutopilotText.name), ["Manual", "Auto", "Overnight", "Bypass", "Lockdown"])
        XCTAssertEqual(AutopilotText.line(.lockdown), "Deny everything")
        XCTAssertEqual(AutopilotText.line(.manual), "Ask, with a hint")
        XCTAssertEqual(AutopilotText.byRisk, [.lockdown, .manual, .auto, .overnight, .bypass])
        XCTAssertEqual(AutopilotText.name(key: "nonsense"), "Manual")
        // "assisted", from an older version, is Manual now.
        XCTAssertEqual(AutopilotText.name(key: "assisted"), "Manual")
        XCTAssertEqual(AutopilotText.symbol(key: "assisted"), "person.fill.checkmark")
        XCTAssertEqual(AutopilotText.symbol(.auto), "bolt.fill")
        XCTAssertEqual(AutopilotText.symbol(.bypass), "bolt.fill")
        XCTAssertEqual(AutopilotText.cardLine(.manual), "You decide. Autopilot shows its score.")
        XCTAssertEqual(AutopilotText.cardLine(.auto), "Decides the easy ones. Asks the rest.")
        XCTAssertTrue(AutopilotText.needsModel(.auto))
        XCTAssertFalse(AutopilotText.needsModel(.manual), "Manual only goes without its hints")
        XCTAssertFalse(AutopilotText.needsModel(.bypass))
    }

    func testCoreReasonsReadAsSentences() {
        XCTAssertEqual(AutopilotModel.sentence("a connection paired less than 10 minutes ago cannot be put in bypass"),
                       "A connection paired less than 10 minutes ago cannot be put in bypass.")
        XCTAssertEqual(AutopilotModel.sentence("Done."), "Done.")
    }

    func testAProfilesIconIsAGlyphNeverAnEmoji() {
        // The stored values stay Android's; each is drawn as its glyph, an unknown one as a person.
        XCTAssertEqual(AutopilotText.icons.map(AutopilotText.profileGlyph), [.person, .briefcase, .home, .flask, .rocket, .lock, .moon, .star])
        XCTAssertEqual(AutopilotText.profileGlyph("🦄"), .person)
        XCTAssertEqual(AutopilotText.profileGlyph(nil), .person)
        XCTAssertEqual(AutopilotText.profileIconName("💼"), "Work")
        XCTAssertEqual(Set(AutopilotText.icons.map(AutopilotText.profileGlyph)).count, AutopilotText.icons.count, "each icon its own glyph")
    }
}
