import UserNotifications
import XCTest
@testable import Reins

/// The platform layer's logic (the Android app's `PushTest`, `GrantRemindersTest`, `NotifierTest`,
/// `AutopilotNotificationsTest`): what pushes are accepted, when grant reminders fire and what they say, the
/// notifications' wording, categories and sounds, the sign-in helpers.
final class PlatformTests: XCTestCase {
    private let now: Int64 = 1_700_000_100

    // MARK: Push

    func testOnlyTheExactPushShapeIsAccepted() {
        XCTAssertEqual(PushPayload(kind: "req", id: "abc_DEF-123")?.id, "abc_DEF-123")
        XCTAssertEqual(PushPayload(kind: "pair", id: "x")?.itemKind, .pairing)
        XCTAssertEqual(PushPayload(kind: "blob", id: "x")?.itemKind, .blob)
        XCTAssertNotNil(PushPayload(kind: "replaced", id: "x"))
        XCTAssertNotNil(PushPayload(kind: "replaced", id: ""), "the server sends replaced with an empty id")
        XCTAssertNil(PushPayload(kind: "replaced", id: "../x"))
        XCTAssertNil(PushPayload(kind: "other", id: "x"))
        XCTAssertNil(PushPayload(kind: "req", id: nil))
        XCTAssertNil(PushPayload(kind: nil, id: "x"))
        XCTAssertNil(PushPayload(kind: "req", id: "../etc"))
        XCTAssertNil(PushPayload(kind: "req", id: String(repeating: "a", count: 129)))
        XCTAssertNil(PushPayload(kind: "req", id: ""))
        let push: [AnyHashable: Any] = ["aps": ["alert": ["title": "Reins"]], "t": "req", "id": "r1"]
        XCTAssertEqual(PushPayload(userInfo: push)?.link, .item(kind: .request, id: "r1"))
        XCTAssertNil(PushPayload(userInfo: ["t": "req", "id": 5]))
        XCTAssertNil(PushPayload(kind: "replaced", id: "")?.link)
    }

    func testThePresenceMarkGoesStale() {
        let d = UserDefaults(suiteName: "PlatformTests.presence")!
        d.removePersistentDomain(forName: "PlatformTests.presence")
        let t = Date(timeIntervalSince1970: 1_000_000)
        XCTAssertFalse(AppPresence.inFront(now: t, defaults: d))
        d.set(t.timeIntervalSince1970, forKey: AppPresence.key)
        XCTAssertTrue(AppPresence.inFront(now: t.addingTimeInterval(20), defaults: d))
        XCTAssertFalse(AppPresence.inFront(now: t.addingTimeInterval(AppPresence.freshness + 1), defaults: d))
        XCTAssertGreaterThan(AppPresence.freshness, AppPresence.interval * 2, "a beat can be late once")
    }

    // MARK: Grant reminders

    private func grant(_ id: String, left: Int64, age: Int64 = 600, active: Bool = true, expires: Bool = true) -> GrantView {
        GrantView(
            id: id, connectionId: "c1", connectionLabel: "Claude", action: "read", summary: "Read emails from @bank.com",
            expiresAt: expires ? now + left : nil, maxUses: nil, uses: 0, createdAt: now - age, lastUsedAt: nil, origin: "approval",
            service: "gmail", account: "me@gmail.com", lines: [], active: active, state: active ? "active" : "expired", allMail: false,
            editableScope: nil
        )
    }

    func testTheLeadIsATenthOfTheGrantsLifeBetweenFiveMinutesAndAnHour() {
        XCTAssertEqual(GrantReminders.leadSeconds(createdAt: 0, expiresAt: 3_600), 360)
        XCTAssertEqual(GrantReminders.leadSeconds(createdAt: 0, expiresAt: 600), 300)
        XCTAssertEqual(GrantReminders.leadSeconds(createdAt: 0, expiresAt: 86_400 * 7), 3_600)
        XCTAssertNil(GrantReminders.reminderAt(createdAt: 0, expiresAt: nil))
    }

    func testOnlyRunningGrantsThatEndByThemselvesRemind() {
        let g = grant("g1", left: 3_000, age: 600)
        let plan = GrantReminders.plan([g, grant("open", left: 10, expires: false), grant("old", left: 100, active: false), grant("over", left: -5)], now: now)
        XCTAssertEqual(plan.map(\.id), ["g1"])
        XCTAssertEqual(plan.first?.fireAt, now + 3_000 - 360)
        XCTAssertEqual(plan.first.map { GrantReminders.dueAt($0, now: now) }, now + 3_000 - 360)
    }

    func testAGrantInItsLastStretchRemindsAlmostAtOnce() {
        let r = GrantReminders.plan([grant("g1", left: 120, age: 3_480)], now: now)[0]
        XCTAssertEqual(GrantReminders.dueAt(r, now: now), now + 1)
        let c = GrantReminders.content(r, now: now, settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Access ends in 1m")
    }

    func testResumingOrDeletingAGrantReplacesOrCancelsItsReminder() {
        let g = grant("g1", left: 3_000)
        let first = GrantReminders.plan([g], now: now)
        var (remove, add) = GrantReminders.diff(wanted: first, scheduled: [])
        XCTAssertEqual(remove, [])
        XCTAssertEqual(add, first)
        var scheduled = Set(first.map(\.identifier))
        (remove, add) = GrantReminders.diff(wanted: first, scheduled: scheduled)
        XCTAssertTrue(remove.isEmpty && add.isEmpty, "never scheduled twice, even after it fired")
        var resumed = g
        resumed.expiresAt = (g.expiresAt ?? 0) + 7_200
        let second = GrantReminders.plan([resumed], now: now)
        (remove, add) = GrantReminders.diff(wanted: second, scheduled: scheduled)
        XCTAssertEqual(remove, first.map(\.identifier))
        XCTAssertEqual(add, second, "a new end time is a new reminder")
        scheduled = Set(second.map(\.identifier))
        (remove, add) = GrantReminders.diff(wanted: [], scheduled: scheduled)
        XCTAssertEqual(remove, second.map(\.identifier))
        XCTAssertTrue(add.isEmpty)
    }

    func testTheReminderSaysWhatEndsAndOpensTheGrant() {
        let r = GrantReminders.plan([grant("g1", left: 3_300, age: 0)], now: now)[0]
        let c = GrantReminders.content(r, now: now, settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Access ends in 5m")
        XCTAssertEqual(c.body, "Claude: Read emails from @bank.com")
        XCTAssertEqual(c.categoryIdentifier, "grant")
        XCTAssertEqual(c.userInfo[NotificationKey.link] as? String, "reins://grant?id=g1")
        XCTAssertNotNil(c.sound)
        var quiet = FeedbackSettings()
        quiet.alertSounds = false
        XCTAssertNil(GrantReminders.content(r, now: now, settings: quiet).sound)
    }

    func testCompactDurations() {
        XCTAssertEqual([59, 60, 3_599, 3_600, 86_400, 7 * 86_400, 365 * 86_400, -4].map(GrantReminders.compactDuration),
                       ["59s", "1m", "59m", "1h", "1d", "1w", "1y", "0s"])
    }

    // MARK: Notification wording

    private func item(_ kind: PendingKind = .request, action: String = "send", suggestion: String? = nil) -> PendingItem {
        PendingItem(
            kind: kind, id: "r1", title: kind == .pairing ? "Connect Gemini to Rewarden?" : "Send email", subtitle: "me@gmail.com",
            createdAt: now, connectionId: "c1", connectionLabel: "Claude", action: action, count: 2, service: "gmail", account: "me@gmail.com",
            waitUntil: nil, op: "", opTitle: "", suggestion: suggestion
        )
    }

    func testARequestSaysWhoWantsWhat() {
        let c = NotificationContent.pending(item(), settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Approval needed")
        XCTAssertEqual(c.body, "Claude: Send email to 2\nme@gmail.com")
        XCTAssertEqual(c.subtitle, "")
        XCTAssertEqual(c.categoryIdentifier, "request")
        XCTAssertEqual(c.threadIdentifier, "requests")
        XCTAssertEqual(c.interruptionLevel, .timeSensitive)
        XCTAssertEqual(c.userInfo[NotificationKey.kind] as? String, "req")
        XCTAssertEqual(c.userInfo[NotificationKey.id] as? String, "r1")
        XCTAssertEqual(c.userInfo[NotificationKey.link] as? String, "reins://item?kind=request&id=r1")
        XCTAssertEqual(NotificationText.title(item(action: "grant")), "Permission requested")
        XCTAssertEqual(NotificationText.title(item(.pairing)), "Connect an AI")
        XCTAssertEqual(NotificationText.body(item(.pairing)), "Connect Gemini to Rewarden?\nme@gmail.com")
        XCTAssertEqual(NotificationText.title(item(.blob)), "File to check")
        XCTAssertEqual(NotificationContent.pending(item(.blob), settings: FeedbackSettings()).categoryIdentifier, "blob")
    }

    func testAPendingItemCarriesAutopilotsSuggestion() {
        let c = NotificationContent.pending(item(suggestion: "Autopilot would approve · 97%"), settings: FeedbackSettings())
        XCTAssertEqual(c.body, "Claude: Send email to 2\nme@gmail.com\nAutopilot would approve · 97%")
    }

    func testRequestSoundsFollowTheInAppSwitches() {
        XCTAssertNotNil(NotificationContent.pending(item(), settings: FeedbackSettings()).sound)
        var s = FeedbackSettings()
        s.requestSounds = false
        XCTAssertNil(NotificationContent.pending(item(), settings: s).sound)
        s = FeedbackSettings()
        s.master = false
        XCTAssertNil(NotificationContent.pending(item(), settings: s).sound, "the master switch silences notifications too")
        s = FeedbackSettings()
        s.sounds = false
        XCTAssertNil(NotificationContent.pending(item(), settings: s).sound)
    }

    private func decision(_ verdict: Verdict, by: String = "autopilot", activity: Int64? = 14) -> AutoDecisionView {
        AutoDecisionView(
            requestId: "r9", kind: .request, connectionId: "c1", connectionLabel: "Claude Code", title: "Push to a branch · dkat/rewarden",
            verdict: verdict, decidedBy: by, pApprove: 0.97, confidence: 0.97, activityId: activity
        )
    }

    func testAnAutomaticApprovalIsQuietAndCanBeReported() {
        let c = NotificationContent.decision(decision(.approve), settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Autopilot approved")
        XCTAssertEqual(c.body, "Push to a branch · dkat/rewarden — Claude Code\n97% sure")
        XCTAssertEqual(c.subtitle, "")
        XCTAssertNil(c.sound)
        XCTAssertEqual(c.interruptionLevel, .passive)
        XCTAssertEqual(c.categoryIdentifier, "autopilot")
        XCTAssertEqual(c.userInfo[NotificationKey.link] as? String, "reins://activity?id=14")
    }

    func testAnAutomaticDenialClicksUnderTheAutopilotSwitch() {
        let c = NotificationContent.decision(decision(.deny), settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Autopilot denied")
        XCTAssertNotNil(c.sound)
        XCTAssertEqual(c.interruptionLevel, .active)
        var s = FeedbackSettings()
        s.autopilotSounds = false
        XCTAssertNil(NotificationContent.decision(decision(.deny), settings: s).sound)
    }

    func testBypassAndLockdownSayWhoDecided() {
        XCTAssertEqual(NotificationText.decisionTitle(decision(.approve, by: "bypass")), "Approved by Bypass")
        XCTAssertEqual(NotificationText.decisionTitle(decision(.deny, by: "bypass")), "Denied in Bypass")
        XCTAssertEqual(NotificationText.decisionTitle(decision(.deny, by: "lockdown")), "Denied by Lockdown")
        XCTAssertNil(NotificationText.decisionDetail(decision(.approve, by: "bypass")), "bypass does not judge")
        let none = NotificationContent.decision(decision(.approve, activity: nil), settings: FeedbackSettings())
        XCTAssertEqual(none.userInfo[NotificationKey.link] as? String, "reins://home", "without an entry it opens the app")
        XCTAssertNil(none.userInfo[NotificationKey.activity])
    }

    func testAPauseSaysWhy() {
        XCTAssertEqual(NotificationText.pausedTitle("Claude"), "Autopilot paused for Claude")
        XCTAssertEqual(NotificationText.pausedBody("unusual volume."),
                       "Its requests wait for you again: unusual volume. Autopilot carries on by itself once things calm down.")
        XCTAssertTrue(NotificationText.pausedBody("  ").contains("again: unusual volume."))
        let quiet = NotificationContent.status(title: "t", body: "b", link: .home, settings: FeedbackSettings(), silent: true)
        XCTAssertNil(quiet.sound, "in front the app chimes itself")
    }

    func testTheGenericPushKeepsItsTextAndGetsItsKindsCategoryAndSound() {
        let push = UNMutableNotificationContent()
        push.title = "Reins"
        push.body = "Something is waiting for you"
        push.userInfo = ["t": "pair", "id": "p1"]
        let c = NotificationContent.generic(push, payload: PushPayload(userInfo: push.userInfo), settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Reins")
        XCTAssertEqual(c.body, "Something is waiting for you")
        XCTAssertEqual(c.categoryIdentifier, "pairing")
        XCTAssertEqual(c.userInfo[NotificationKey.link] as? String, "reins://item?kind=pairing&id=p1")
        XCTAssertNotNil(c.sound)
        var off = FeedbackSettings()
        off.master = false
        XCTAssertNil(NotificationContent.generic(push, payload: nil, settings: off).sound)
        let handled = NotificationContent.handled(push)
        XCTAssertEqual(handled.title, "Already handled")
        XCTAssertNil(handled.sound)
    }

    func testEveryCategoryHidesItsTextOnTheLockScreen() {
        let categories = NotificationRouter.categories()
        XCTAssertEqual(Set(categories.map(\.identifier)), Set(NotificationCategory.allCases.map(\.rawValue)))
        for c in categories { XCTAssertFalse(c.hiddenPreviewsBodyPlaceholder.isEmpty, c.identifier) }
        let request = categories.first { $0.identifier == "request" }
        XCTAssertEqual(request?.actions.map(\.identifier), ["deny"])
        XCTAssertEqual(request?.actions.first?.options.contains(.destructive), true)
        XCTAssertEqual(request?.actions.first?.options.contains(.authenticationRequired), true)
        XCTAssertEqual(request?.hiddenPreviewsBodyPlaceholder, "Something is waiting for you")
        XCTAssertEqual(categories.first { $0.identifier == "autopilot" }?.actions.map(\.identifier), ["report"])
    }

    func testTheExtensionLeavesModelWorkToTheApp() {
        func settings(_ mode: AutopilotMode, connection: AutopilotMode = .manual, model: ModelState = .installed) -> AutopilotSettings {
            AutopilotSettings(
                mode: mode, baseMode: mode, bypassUntil: nil, defaultProfileId: "p", wifiOnly: true,
                model: ModelStatus(state: model, id: "m", label: "m", version: "1", sizeBytes: 1, downloadedBytes: 1, error: nil, runtimeReady: true),
                connections: [ConnectionAutopilot(connectionId: "c1", baseMode: connection, bypassUntil: nil, mode: connection, profileId: "p")]
            )
        }
        XCTAssertFalse(ExtensionPolicy.needsModel(settings(.manual)))
        XCTAssertFalse(ExtensionPolicy.needsModel(settings(.lockdown)))
        XCTAssertFalse(ExtensionPolicy.needsModel(settings(.bypass)))
        XCTAssertTrue(ExtensionPolicy.needsModel(settings(.assisted)))
        XCTAssertTrue(ExtensionPolicy.needsModel(settings(.manual, connection: .auto)))
        XCTAssertFalse(ExtensionPolicy.needsModel(settings(.auto, model: .notInstalled)), "without a model the app could not judge either")
    }

    // MARK: Sign-in helpers

    func testOnlyTheCoresRedirectGoesToTheCore() {
        XCTAssertTrue(McpSignIn.isRedirect("dev.rewarden.android://mcp-oauth?code=a&state=b"))
        XCTAssertFalse(McpSignIn.isRedirect("dev.rewarden.android://other?code=a"))
        XCTAssertFalse(McpSignIn.isRedirect("https://mcp-oauth/?code=a"))
        XCTAssertFalse(McpSignIn.isRedirect("dev.rewarden.android://mcp-oauth?" + String(repeating: "a", count: 9_000)))
        XCTAssertTrue(McpSignIn.isWebPage(URL(string: "https://linear.app/oauth")!))
        XCTAssertFalse(McpSignIn.isWebPage(URL(string: "file:///etc/passwd")!))
        XCTAssertFalse(McpSignIn.isWebPage(URL(string: "reins://home")!))
    }

    func testGoogleScopesPerService() {
        XCTAssertEqual(GoogleScopes.of("gmail"), [GoogleScopes.gmailReadonly, GoogleScopes.gmailSend])
        XCTAssertEqual(GoogleScopes.of("gcalendar"), [GoogleScopes.calendarEvents, GoogleScopes.calendarReadonly])
        XCTAssertEqual(GoogleScopes.of("gcontacts"), [GoogleScopes.contactsReadonly])
        XCTAssertFalse(GoogleConfig(clientId: "", redirectScheme: "x").isConfigured)
        XCTAssertFalse(GoogleConfig(clientId: "$(REINS_GOOGLE_CLIENT_ID)", redirectScheme: "x").isConfigured)
        XCTAssertTrue(GoogleConfig(clientId: "1-a.apps.googleusercontent.com", redirectScheme: "com.googleusercontent.apps.1-a").isConfigured)
    }

    func testTheGoogleRequestUsesPkceAndChecksItsAnswer() throws {
        let config = GoogleConfig(clientId: "1-a.apps.googleusercontent.com", redirectScheme: "com.googleusercontent.apps.1-a")
        // RFC 7636 appendix B.
        let request = GoogleAuthRequest(config: config, service: "gcalendar", loginHint: "Me@Gmail.com",
                                        verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk", state: "s1")
        XCTAssertEqual(request.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        let query = Dictionary(uniqueKeysWithValues: (URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") })
        XCTAssertEqual(query["redirect_uri"], "com.googleusercontent.apps.1-a:/oauth2redirect")
        XCTAssertEqual(query["code_challenge_method"], "S256")
        XCTAssertEqual(query["login_hint"], "Me@Gmail.com")
        XCTAssertEqual(query["scope"], "openid email \(GoogleScopes.calendarEvents) \(GoogleScopes.calendarReadonly)")
        XCTAssertEqual(try request.code(from: URL(string: "com.googleusercontent.apps.1-a:/oauth2redirect?state=s1&code=C")!), "C")
        XCTAssertThrowsError(try request.code(from: URL(string: "com.googleusercontent.apps.1-a:/oauth2redirect?state=other&code=C")!))
        XCTAssertThrowsError(try request.code(from: URL(string: "evil:/oauth2redirect?state=s1&code=C")!))
        XCTAssertThrowsError(try request.code(from: URL(string: "com.googleusercontent.apps.1-a:/oauth2redirect?state=s1&error=access_denied")!)) {
            XCTAssertEqual($0 as? GoogleAuthError, .denied)
        }
    }

    func testTheSignedInAddressComesFromTheIdToken() {
        let claims = Data(#"{"email":"me@gmail.com","sub":"1"}"#.utf8).base64EncodedString()
            .replacingOccurrences(of: "=", with: "").replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
        XCTAssertEqual(GoogleAuthRequest.email(fromIdToken: "h.\(claims).s"), "me@gmail.com")
        XCTAssertNil(GoogleAuthRequest.email(fromIdToken: "garbage"))
        XCTAssertEqual(String(data: GoogleHTTP.formBody(["b": "a b&c", "a": "x/y"]), encoding: .utf8), "a=x%2Fy&b=a%20b%26c")
    }

    // MARK: Phone calendar

    func testAllDayEventsTravelAsUtcMidnights() {
        var berlin = Calendar(identifier: .gregorian)
        berlin.timeZone = TimeZone(identifier: "Europe/Berlin")!
        let start = berlin.date(from: DateComponents(year: 2026, month: 10, day: 1))!
        let end = berlin.date(from: DateComponents(year: 2026, month: 10, day: 1, hour: 23, minute: 59, second: 59))!
        let (s, e) = PhoneBridge.allDayBounds(start: start, end: end, calendar: berlin)
        XCTAssertEqual(s, 1_790_812_800, "1 Oct 2026 00:00 UTC")
        XCTAssertEqual(e, 1_790_812_800 + 86_400)
        let (ls, le) = PhoneBridge.localAllDay(start: s, end: e, calendar: berlin)
        XCTAssertEqual(ls, start)
        XCTAssertEqual(le, start, "a one-day event starts and ends on its day")
        let (_, twoDays) = PhoneBridge.localAllDay(start: s, end: e + 86_400, calendar: berlin)
        XCTAssertEqual(twoDays, berlin.date(byAdding: .day, value: 1, to: start))
    }
}
