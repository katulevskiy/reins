import XCTest
@testable import Reins

/// The grant clock, "New grant" and resuming (the Android app's DesignLogicTest, grant parts).
final class GrantLogicTests: XCTestCase {
    private let now: Int64 = 1_800_000_000

    private func scope(_ action: String, allMail: Bool = false) -> GrantScopeChoice {
        GrantScopeChoice(
            allMail: allMail, selectedMessagesOnly: false, senderAddresses: [],
            senderDomains: action == "send" || allMail ? [] : ["bank.com"], subjectPattern: nil, recipientAddresses: [],
            recipientDomains: action == "send" ? ["corp.example"] : [], resources: [], classes: []
        )
    }

    /// Active grants were made `age` seconds ago and end in `left`; ended ones expired two days ago.
    private func grant(
        _ id: String = "g1", active: Bool = true, uses: UInt32 = 3, maxUses: UInt32? = nil, action: String = "read",
        state: String? = nil, left: Int64 = 3_000, age: Int64 = 600, allMail: Bool = false, editable: GrantScopeChoice?? = .none
    ) -> GrantView {
        GrantView(
            id: id, connectionId: "c1", connectionLabel: "Claude", action: action,
            summary: action == "send" ? "Send emails to @corp.example" : "Read emails from @bank.com",
            expiresAt: active ? now + left : now - 86_400 * 2, maxUses: maxUses, uses: uses,
            createdAt: active ? now - age : now - 86_400 * 3, lastUsedAt: now - 500, origin: "approval", service: "gmail",
            account: "me@gmail.com", lines: ["From @bank.com"], active: active, state: state ?? (active ? "active" : "expired"),
            allMail: allMail, editableScope: editable ?? scope(action, allMail: allMail)
        )
    }

    // MARK: Clock

    func testTimeLeftIsWrittenInItsLargestUnit() {
        let cases: [(Int64, String)] = [
            (0, "0s"), (45, "45s"), (60, "1m"), (47 * 60 + 59, "47m"), (3_600, "1h"), (3 * 3_600 + 1_799, "3h"),
            (86_399, "23h"), (86_400, "1d"), (5 * 86_400 + 100, "5d"), (7 * 86_400 - 1, "6d"), (7 * 86_400, "1w"),
            (10 * 7 * 86_400 + 3, "10w"), (364 * 86_400, "52w"), (365 * 86_400, "1y"), (2 * 365 * 86_400 + 5, "2y"), (-5, "0s"),
        ]
        for (seconds, text) in cases { XCTAssertEqual(compactDuration(seconds), text, "\(seconds)") }
    }

    func testAGrantIsAboutToEndInTheLastTenthOfItsLifeBetweenFiveMinutesAndAnHour() {
        XCTAssertEqual(expiryLeadSeconds(createdAt: 0, expiresAt: 600), 300)
        XCTAssertEqual(expiryLeadSeconds(createdAt: 0, expiresAt: 3_600), 360)
        XCTAssertEqual(expiryLeadSeconds(createdAt: 0, expiresAt: 86_400), 3_600)
        XCTAssertEqual(expiryLeadSeconds(createdAt: 0, expiresAt: 30 * 86_400), 3_600)
        var g = grant(left: 3_000, age: 600)
        XCTAssertEqual(reminderAt(g), (g.expiresAt ?? 0) - 360)
        g.expiresAt = nil
        XCTAssertNil(reminderAt(g), "nothing to remind about")
    }

    func testTheClockKnowsTheFractionLeftWhetherItEndsSoonAndNeverEndsWithoutAnEnd() {
        let g = grant(left: 900, age: 2_700)
        let clock = grantClock(g, now: now)
        XCTAssertEqual(clock.remaining, 900)
        XCTAssertEqual(clock.fraction, 0.25, accuracy: 0.001)
        XCTAssertEqual(clock.label, "15m")
        XCTAssertFalse(clock.soon)
        XCTAssertTrue(grantClock(g, now: now + 600).soon)
        XCTAssertEqual(grantClock(g, now: (g.expiresAt ?? 0) + 10).remaining, 0)
        var open = g
        open.expiresAt = nil
        let c = grantClock(open, now: now)
        XCTAssertNil(c.remaining)
        XCTAssertEqual(c.fraction, 1)
        XCTAssertFalse(c.soon)
        XCTAssertEqual(c.label, "\u{221E}")
        var ended = g
        ended.active = false
        ended.state = "expired"
        XCTAssertFalse(grantClock(ended, now: now + 800).soon, "an ended grant is never ending soon")
    }

    func testAnEndedGrantSaysHowItEndedAndCanBeResumedForASensiblePeriod() {
        XCTAssertEqual(GrantText.endedLine(grant(active: false, state: "revoked"), now: now), "Deleted")
        XCTAssertEqual(GrantText.endedLine(grant(active: false, uses: 3, maxUses: 3, state: "used_up"), now: now), "All 3 uses spent")
        XCTAssertTrue(GrantText.endedLine(grant(active: false), now: now).hasPrefix("Expired "))
        XCTAssertEqual(["revoked", "used_up", "expired"].map { GrantText.inactiveWord(grant(active: false, state: $0)) }, ["Deleted", "Used up", "Expired"])
        XCTAssertEqual(resumePeriods(allMail: false).count, 4)
        XCTAssertEqual(resumePeriods(allMail: true), [.hour, .day, .week])
    }

    func testUsesAndTimesReadShort() {
        XCTAssertEqual(GrantText.usedTimes(0), "Not used yet")
        XCTAssertEqual(GrantText.usedTimes(1), "Used once")
        XCTAssertEqual(GrantText.usedTimes(4), "Used 4 times")
        XCTAssertEqual(GrantText.usesLeft(3, of: 5), "2 left")
        XCTAssertEqual(GrantText.usesLeft(5, of: 5), "All used")
        XCTAssertEqual(GrantText.usesLeft(9, of: 40), "9 of 40")
        XCTAssertEqual(GrantText.relative(now - 10, now: now), "just now")
        XCTAssertEqual(GrantText.relative(now - 300, now: now), "5 min ago")
        XCTAssertEqual(GrantText.relative(now - 3 * 3600, now: now), "3 h ago")
        XCTAssertEqual(GrantText.relative(now - 90_000, now: now), "yesterday")
        XCTAssertEqual(GrantText.expiry(now + 3_000, now: now), "expires in 50 min")
        XCTAssertEqual(GrantText.expiry(now - 1, now: now), "expired")
        XCTAssertEqual(GrantText.expiry(nil, now: now), "no time limit")
        XCTAssertEqual(GrantText.origin("ai_request", label: "Claude"), "Claude asked and you allowed it")
        XCTAssertEqual(GrantText.origin("user", label: "Claude"), "Created by you in advance")
    }

    // MARK: New grant

    func testNewGrantTextIsSplitIntoAddressesAndDomains() {
        let (addresses, domains) = parseParties("Alerts@Bank.com, @statements.bank.com  news.example\nalerts@bank.com")
        XCTAssertEqual(addresses, ["alerts@bank.com"])
        XCTAssertEqual(domains, ["statements.bank.com", "news.example"])
    }

    func testANewReadGrantNeedsSomeoneAndAOneTimeGrantHasNoExpiry() {
        guard case .invalid = buildNewGrant(NewGrantDraft(connectionId: "c1")) else { return XCTFail() }
        guard case .invalid = buildNewGrant(NewGrantDraft(partiesText: "a@b.com")) else { return XCTFail("no connection") }
        guard case let .ok(_, _, kind, standing) = buildNewGrant(
            NewGrantDraft(connectionId: "c1", account: "me@gmail.com", partiesText: "@bank.com", lifetime: .oneTime)
        ) else { return XCTFail() }
        XCTAssertEqual(kind, .read)
        XCTAssertEqual(standing.maxUses, 1)
        XCTAssertNil(standing.durationSecs)
        XCTAssertEqual(standing.scope.senderDomains, ["bank.com"])
    }

    func testAMissingAccountIsExplained() {
        XCTAssertEqual(
            buildNewGrant(NewGrantDraft(connectionId: "c1", partiesText: "@bank.com")),
            .invalid("Choose which Gmail account this is for.")
        )
    }

    func testAllMailForANewGrantIsTimeBoxedToAWeekAtMostAndNeverForSending() {
        guard case let .ok(_, _, _, week) = buildNewGrant(NewGrantDraft(connectionId: "c1", account: "me@gmail.com", anyMail: true, lifetime: .week)) else {
            return XCTFail()
        }
        XCTAssertTrue(week.scope.allMail)
        XCTAssertEqual(week.durationSecs, 604_800)
        for draft in [
            NewGrantDraft(connectionId: "c1", account: "me@gmail.com", anyMail: true, lifetime: .month),
            NewGrantDraft(connectionId: "c1", account: "me@gmail.com", anyMail: true, lifetime: .oneTime),
            NewGrantDraft(connectionId: "c1", account: "me@gmail.com", send: true, anyMail: true, lifetime: .day),
        ] {
            guard case .invalid = buildNewGrant(draft) else { return XCTFail("\(draft)") }
        }
        XCTAssertFalse(NewGrantLifetime.oneTime.allowedForAllMail)
        XCTAssertFalse(NewGrantLifetime.month.allowedForAllMail)
    }

    func testASendGrantListsItsRecipients() {
        guard case .invalid = buildNewGrant(NewGrantDraft(connectionId: "c1", account: "me@gmail.com", send: true)) else { return XCTFail() }
        guard case let .ok(_, _, kind, standing) = buildNewGrant(NewGrantDraft(
            connectionId: "c1", account: "me@gmail.com", send: true, partiesText: "ann@corp.com @corp.com", subject: " report ", lifetime: .hour
        )) else { return XCTFail() }
        XCTAssertEqual(kind, .send)
        XCTAssertEqual(standing.scope.recipientAddresses, ["ann@corp.com"])
        XCTAssertEqual(standing.scope.recipientDomains, ["corp.com"])
        XCTAssertEqual(standing.scope.subjectPattern, "report")
        XCTAssertTrue(standing.scope.senderAddresses.isEmpty)
    }

    // MARK: Resuming

    private func ended(_ action: String = "read", allMail: Bool = false, maxUses: UInt32? = nil, editable: GrantScopeChoice?? = .none) -> GrantView {
        grant("old", active: false, maxUses: maxUses, action: action, allMail: allMail, editable: editable)
    }

    private func ok(_ r: ResumeResult, file: StaticString = #filePath, line: UInt = #line) -> (seconds: Int64, standing: StandingGrant?) {
        guard case let .ok(seconds, standing) = r else {
            XCTFail("expected ok, got \(r)", file: file, line: line)
            return (0, nil)
        }
        return (seconds, standing)
    }

    private func isInvalid(_ r: ResumeResult) -> Bool {
        if case .invalid = r { return true }
        return false
    }

    func testResumingAsItWasIsAPlainResumeForTheChosenPeriod() {
        let g = ended()
        var draft = initialResumeDraft(g)
        XCTAssertEqual(draft.partiesText, "@bank.com")
        draft.period = .week
        let plain = ok(buildResume(g, draft))
        XCTAssertEqual(plain.seconds, 604_800)
        XCTAssertNil(plain.standing)
    }

    func testACustomTimeWinsOverTheQuickChoiceAndIsChecked() {
        let g = ended()
        let base = initialResumeDraft(g)
        func with(_ amount: String, _ unit: CustomUnit = .hours) -> ResumeDraft {
            var d = base
            d.customAmount = amount
            d.customUnit = unit
            return d
        }
        XCTAssertEqual(ok(buildResume(g, with("90", .minutes))).seconds, 5_400)
        XCTAssertEqual(ok(buildResume(g, with("3", .days))).seconds, 3 * 86_400)
        XCTAssertTrue(isInvalid(buildResume(g, with("0"))))
        XCTAssertTrue(isInvalid(buildResume(g, with("31", .days))))
        XCTAssertFalse(isInvalid(buildResume(g, with("59", .minutes))))
        XCTAssertTrue(isInvalid(buildResume(g, with("x"))))
    }

    func testChangingWhoItCoversOrHowOftenMakesAnEditedResume() {
        let g = ended()
        let base = initialResumeDraft(g)
        var moved = base
        moved.partiesText = "ann@corp.com, @statements.example"
        moved.subject = " invoice "
        let m = ok(buildResume(g, moved))
        XCTAssertEqual(m.standing?.scope.senderAddresses, ["ann@corp.com"])
        XCTAssertEqual(m.standing?.scope.senderDomains, ["statements.example"])
        XCTAssertEqual(m.standing?.scope.subjectPattern, "invoice")
        XCTAssertEqual(m.standing?.durationSecs, 86_400)
        var limited = base
        limited.limitUses = true
        limited.uses = "3"
        XCTAssertEqual(ok(buildResume(g, limited)).standing?.maxUses, 3)
        limited.uses = "0"
        XCTAssertTrue(isInvalid(buildResume(g, limited)))
        var empty = base
        empty.partiesText = ""
        empty.subject = ""
        XCTAssertTrue(isInvalid(buildResume(g, empty)))
        var same = base
        same.partiesText = "@BANK.com"
        XCTAssertNil(ok(buildResume(g, same)).standing, "the same senders in other spelling changed nothing")
    }

    func testAllMailCanBeResumedForAWeekAtMostAndNeverForSending() {
        let g = ended()
        var base = initialResumeDraft(g)
        base.allMail = true
        base.partiesText = ""
        var week = base
        week.period = .week
        XCTAssertEqual(ok(buildResume(g, week)).standing?.scope.allMail, true)
        var month = base
        month.period = .month
        XCTAssertTrue(isInvalid(buildResume(g, month)))
        var eight = base
        eight.customAmount = "8"
        eight.customUnit = .days
        XCTAssertTrue(isInvalid(buildResume(g, eight)))
        let send = ended("send")
        var noOne = initialResumeDraft(send)
        noOne.partiesText = ""
        XCTAssertTrue(isInvalid(buildResume(send, noOne)))
        var corp = initialResumeDraft(send)
        corp.partiesText = "@corp.example, bob@corp.example"
        XCTAssertEqual(ok(buildResume(send, corp)).standing?.scope.recipientDomains, ["corp.example"])
    }

    func testAGrantTheEditorCannotDescribeOnlyComesBackAsItWas() {
        let g = ended(editable: .some(nil))
        var draft = initialResumeDraft(g)
        draft.partiesText = "x@y.com"
        draft.limitUses = true
        draft.uses = "2"
        XCTAssertNil(ok(buildResume(g, draft)).standing)
    }
}
