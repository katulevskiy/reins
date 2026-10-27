import UserNotifications
import XCTest
@testable import Reins

/// Circles in words and order (the Android app's `CircleLogicTest`): who someone is, the owner's progress line, "For
/// others" grouped by person with the soonest deadline first, the people split both ways, the Add screen's draft, the
/// Circle push and its one notification.
final class CircleLogicTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func review(
        _ id: String, person: String = "sam.lee@example.com", deadline: Int64 = 0, created: Int64 = 0,
        state: CircleReviewState = .waiting, title: String = "Purchase $42", agent: String = "ChatGPT"
    ) -> CircleReviewView {
        CircleReviewView(
            id: id, linkId: "l1", person: person, role: .guardian, title: title, subtitle: "Lego set", headline: "", service: "payments",
            agent: agent, line: "Ask · Outside", detail: nil, scopeLine: "Money", createdAt: created, deadline: deadline, state: state
        )
    }

    private func person(
        _ id: String, peer: String, role: CircleRole = .cosign, mine: Bool = true, state: CircleLinkState = .active
    ) -> CirclePerson {
        CirclePerson(
            linkId: id, peer: peer, role: role, mine: mine, scopeLine: "Money", deferred: true, windowSecs: 14_400, state: state, since: 0,
            ending: nil, needsConsent: false
        )
    }

    private func progress(
        needs: [String] = [], approved: [String] = [], waitingForYou: Bool = false, youApproved: Bool = false, line: String = ""
    ) -> CircleProgress {
        CircleProgress(needs: needs, approved: approved, waitingForYou: waitingForYou, youApproved: youApproved, deferred: true, deadline: 0, line: line)
    }

    // MARK: Names

    func testAPersonIsTheFirstWordOfTheirAddressCapitalised() {
        XCTAssertEqual(CircleLogic.name("sam.lee@example.com"), "Sam")
        XCTAssertEqual(CircleLogic.name("alex_rivera+reins@example.com"), "Alex")
        XCTAssertEqual(CircleLogic.name("Mom@family.example"), "Mom")
        XCTAssertEqual(CircleLogic.name("no-at-sign"), "No")
        XCTAssertEqual(CircleLogic.names(["a@x.com", "b@x.com", "c@x.com"]), "A, B and C")
        XCTAssertEqual(CircleLogic.names(["alex@x.com", "sam@x.com"]), "Alex and Sam")
        XCTAssertEqual(CircleLogic.names([]), "")
    }

    func testAPersonsNameIsCleanedAndCapped() {
        let name = CircleLogic.name("\u{202E}eve\u{200B}" + String(repeating: "x", count: 80) + "@example.com")
        XCTAssertFalse(name.contains("\u{202E}"), "no direction overrides from another account")
        XCTAssertLessThanOrEqual(name.count, 32)
    }

    // MARK: The owner's card

    func testTheProgressLineIsTheCoresWhenItHasOne() {
        XCTAssertEqual(CircleLogic.progressLine(progress(needs: ["alex@x.com"], line: "Needs Alex too")), "Needs Alex too")
    }

    func testTheProgressLineIsMadeFromWhoStillAnswersOtherwise() {
        XCTAssertEqual(CircleLogic.progressLine(progress(needs: ["alex@x.com"], waitingForYou: true)), "Needs Alex too")
        XCTAssertEqual(CircleLogic.progressLine(progress(approved: ["alex@x.com"], waitingForYou: true)), "Alex approved, waiting for you")
        XCTAssertEqual(CircleLogic.progressLine(progress(needs: ["alex@x.com"], youApproved: true)), "You approved · waiting for Alex")
        XCTAssertEqual(CircleLogic.progressLine(progress(youApproved: true)), "You approved")
    }

    // MARK: For others

    func testReviewsAreGroupedByPersonTheSoonestDeadlineFirst() {
        let reviews = [
            review("s-late", person: "sam@x.com", deadline: now + 9_000),
            review("j-soon", person: "jo@x.com", deadline: now + 600),
            review("s-soon", person: "sam@x.com", deadline: now + 3_000),
            review("j-tie-new", person: "jo@x.com", deadline: now + 7_000, created: 20),
            review("j-tie-old", person: "jo@x.com", deadline: now + 7_000, created: 10),
            review("done", person: "sam@x.com", deadline: now + 100, state: .approved),
        ]
        let groups = CircleLogic.groups(reviews)
        XCTAssertEqual(groups.map(\.person), ["jo@x.com", "sam@x.com"], "the group with the soonest deadline first")
        XCTAssertEqual(groups[0].reviews.map(\.id), ["j-soon", "j-tie-old", "j-tie-new"])
        XCTAssertEqual(groups[1].reviews.map(\.id), ["s-soon", "s-late"], "only what waits")
        XCTAssertEqual(groups[0].deadline, now + 600)
        XCTAssertEqual(groups[1].deadline, now + 3_000)
    }

    func testTwoPeopleOfOneNameNeverMix() {
        let groups = CircleLogic.groups([review("a", person: "sam@one.com"), review("b", person: "sam@two.com")])
        XCTAssertEqual(groups.count, 2)
    }

    func testSettledReviewsAreNewestFirstAndCapped() {
        let reviews = [
            review("w", state: .waiting),
            review("old", created: 10, state: .denied),
            review("new", created: 30, state: .approved),
            review("mid", created: 20, state: .withdrawn),
        ]
        XCTAssertEqual(CircleLogic.settled(reviews).map(\.id), ["new", "mid", "old"])
        XCTAssertEqual(CircleLogic.settled(reviews, limit: 2).map(\.id), ["new", "mid"])
        XCTAssertEqual(CircleLogic.outcome(.approved), "You approved")
        XCTAssertEqual(CircleLogic.outcome(.withdrawn), "Withdrawn")
    }

    func testTheCardSaysWhoseAgentAsks() {
        XCTAssertEqual(CircleLogic.whose(review("a")), "Sam's ChatGPT")
        XCTAssertEqual(CircleLogic.whose(review("a", agent: "")), "Sam")
    }

    func testDeadlinesReadAsTicketsDo() {
        XCTAssertEqual(CircleLogic.due(now + 3 * 3_600, now: now), "Due in 3 h")
        XCTAssertEqual(CircleLogic.due(now - 1, now: now), "Overdue")
    }

    func testANoteIsTrimmedAndCapped() {
        XCTAssertNil(CircleLogic.note("   "))
        XCTAssertNil(CircleLogic.note(nil))
        XCTAssertEqual(CircleLogic.note("  not this one \n"), "not this one")
        XCTAssertEqual(CircleLogic.note(String(repeating: "a", count: 900))?.count, 500)
    }

    func testNoticesAreNewestFirst() {
        let notices = [
            CircleNoticeView(id: "1", person: "dana@x.com", text: "Bypass on", outcome: nil, at: 10, lines: []),
            CircleNoticeView(id: "2", person: "dana@x.com", text: "Bypass off", outcome: nil, at: 20, lines: []),
        ]
        XCTAssertEqual(CircleLogic.notices(notices).map(\.id), ["2", "1"])
    }

    // MARK: Settings › Circles

    func testPeopleSplitIntoHelpingYouAndYouHelpRunningFirst() {
        let people = [
            person("ended", peer: "zed@x.com", state: .ended),
            person("paused", peer: "amy@x.com", state: .paused),
            person("bob", peer: "bob@x.com"),
            person("al", peer: "al@x.com"),
            person("helped", peer: "sam@x.com", role: .guardian, mine: false),
        ]
        let split = CircleLogic.split(people)
        XCTAssertEqual(split.helpingYou.map(\.linkId), ["al", "bob", "paused", "ended"])
        XCTAssertEqual(split.youHelp.map(\.linkId), ["helped"])
    }

    func testOnlyTheOwnerPausesAndAHelperLeaves() {
        let mine = person("m", peer: "a@x.com")
        let theirs = person("t", peer: "b@x.com", mine: false)
        XCTAssertTrue(CircleLogic.canPause(mine))
        XCTAssertFalse(CircleLogic.canPause(theirs))
        XCTAssertFalse(CircleLogic.canPause(person("e", peer: "c@x.com", state: .ended)))
        XCTAssertFalse(CircleLogic.canPause(person("s", peer: "c@x.com", state: .suspended)))
        XCTAssertEqual(CircleLogic.endTitle(mine), "End")
        XCTAssertEqual(CircleLogic.endTitle(theirs), "Leave")
        XCTAssertFalse(CircleLogic.canEnd(person("e", peer: "c@x.com", state: .ended)))
        XCTAssertNil(CircleLogic.badge(.active))
        XCTAssertEqual(CircleLogic.badge(.paused), "Paused")
    }

    func testClassAndWindowLines() {
        XCTAssertEqual(CircleLogic.classLine(deferred: true, windowSecs: 14_400), "Deferred · 4 h")
        XCTAssertEqual(CircleLogic.classLine(deferred: false, windowSecs: 120), "Instant · 2 min")
        XCTAssertEqual(CircleLogic.classLine(deferred: true, windowSecs: 0), "Deferred")
        XCTAssertEqual(CircleLogic.window(3 * 86_400), "3 d")
        XCTAssertEqual(CircleLogic.window(30), "1 min")
    }

    // MARK: Add someone

    func testTheDraftTakesTheClassDefaultsAndTheRolesViewOfTheCard() {
        let delegate = CircleLogic.draft(role: .delegate, preset: .money, deferred: true)
        XCTAssertEqual(delegate.preset, .money)
        XCTAssertTrue(delegate.deferred)
        XCTAssertNil(delegate.windowSecs, "the class's default window")
        XCTAssertFalse(delegate.fallBackToMe)
        XCTAssertTrue(delegate.fullCards, "a delegate sees the card as the owner does")
        XCTAssertTrue(CircleLogic.draft(role: .cosign, preset: .risky, deferred: false).fullCards)
        XCTAssertFalse(CircleLogic.draft(role: .guardian, preset: .everything, deferred: true).fullCards, "a guardian gets a summary")
        XCTAssertFalse(CircleLogic.draft(role: .watcher, preset: .everything, deferred: true).fullCards, "never for a watcher")
    }

    func testEveryPresetHasItsWords() {
        XCTAssertEqual(CircleLogic.presets.map(CircleLogic.presetTitle),
                       ["Everything", "Money", "Messages to Work or Outside", "What can't be undone", "Routine"])
        XCTAssertEqual(CircleLogic.presetTitle(.agent(connectionId: "c1", label: "Claude")), "Claude")
        XCTAssertEqual(CircleLogic.presetTitle(.agent(connectionId: "c1", label: "")), "One AI")
        XCTAssertEqual(CircleLogic.roles.map(CircleLogic.roleName), ["Delegate", "Co-sign", "Guardian", "Watcher"])
    }

    // MARK: The push and the notification

    func testACirclePushNamesWhyItCame() {
        XCTAssertEqual(PushPayload(kind: "circle", id: "review:pk_1")?.circle, CirclePush("review:pk_1"))
        XCTAssertEqual(CirclePush("review:pk_1")?.wake, .review)
        XCTAssertEqual(CirclePush("review:pk_1")?.id, "pk_1")
        XCTAssertTrue(CirclePush("alert:l1")?.alerts == true)
        XCTAssertTrue(CirclePush("news:x")?.alerts == false)
        XCTAssertNil(PushPayload(kind: "circle", id: "pk_1"), "no reason, no push")
        XCTAssertNil(PushPayload(kind: "circle", id: "other:pk_1"))
        XCTAssertNil(PushPayload(kind: "circle", id: "review:../etc"))
        XCTAssertEqual(PushPayload(kind: "circle", id: "review:pk_1")?.link, .home, "a tap opens Activity")
        XCTAssertNil(PushPayload(kind: "circle", id: "review:pk_1")?.itemKind, "never an item of the owner's own")
    }

    func testTheOneNotificationIsTitledByPersonWithNoAction() {
        let r = review("pk_1", deadline: now + 3 * 3_600)
        XCTAssertEqual(CircleLogic.notificationTitle(r), "Sam: purchase $42 waiting")
        XCTAssertEqual(CircleLogic.notificationTitle(review("x", title: "SSH key")), "Sam: SSH key waiting", "an acronym keeps its case")
        let c = NotificationContent.circleReview(r, settings: FeedbackSettings(), now: now)
        XCTAssertEqual(c.title, "Sam: purchase $42 waiting")
        XCTAssertEqual(c.body, "Lego set\nDue in 3 h")
        XCTAssertEqual(c.categoryIdentifier, NotificationCategory.circle.rawValue)
        XCTAssertEqual(c.userInfo[NotificationKey.id] as? String, "review:pk_1", "the same id the push names")
        let category = NotificationRouter.categories().first { $0.identifier == NotificationCategory.circle.rawValue }
        XCTAssertEqual(category?.actions.count, 0, "nothing is answered from the notification")
    }

    func testASettledReviewsNotificationGoes() {
        let info: [AnyHashable: Any] = [NotificationKey.id: "review:pk_1"]
        let date = Date(timeIntervalSince1970: TimeInterval(now))
        let soon = date.addingTimeInterval(5)
        let later = date.addingTimeInterval(120)
        XCTAssertTrue(CircleSweep.isSettled(info: info, date: date, waiting: [], settled: ["pk_1"], now: soon), "answered: at once")
        XCTAssertFalse(CircleSweep.isSettled(info: info, date: date, waiting: [], settled: [], now: soon), "not read yet: left a minute")
        XCTAssertTrue(CircleSweep.isSettled(info: info, date: date, waiting: [], settled: [], now: later))
        XCTAssertFalse(CircleSweep.isSettled(info: info, date: date, waiting: ["pk_1"], settled: [], now: later), "it waits")
        XCTAssertFalse(CircleSweep.isSettled(info: [NotificationKey.id: "alert:l1"], date: date, waiting: [], settled: [], now: later),
                       "a guardianship's notice stays")
    }

    // MARK: The demo core (screenshots, UI tests)

    func testTheDemoInvitationShowsTheCodeThenConfirms() async throws {
        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: false)
        let start = try await core.circleInviteCreate(draft: CircleLogic.draft(role: .cosign, preset: .money, deferred: true))
        XCTAssertTrue(start.link.hasPrefix("reins-circle:"))
        let first = try await core.circleInvitePoll(id: start.id)
        XCTAssertEqual(first, .waiting)
        let second = try await core.circleInvitePoll(id: start.id)
        guard case let .compare(code, _) = second else { return XCTFail("the code comes once they answered") }
        XCTAssertFalse(code.isEmpty)
        let third = try await core.circleInvitePoll(id: start.id)
        guard case .accepted = third else { return XCTFail("then they accept") }
        let person = try await core.circleInviteConfirm(id: start.id)
        XCTAssertEqual(person.role, .cosign)
        let people = try await core.circlePeople(refresh: false)
        XCTAssertEqual(CircleLogic.split(people).helpingYou.map(\.linkId), [person.linkId])
    }

    func testTheDemoAnswerSettlesTheReview() async throws {
        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: true)
        let waiting = CircleLogic.groups(try await core.circleReviews(refresh: false)).flatMap(\.reviews)
        let first = try XCTUnwrap(waiting.first)
        try await core.circleAnswer(reviewId: first.id, approve: false, note: "Not this one")
        let after = try await core.circleReviews(refresh: false)
        XCTAssertEqual(after.first { $0.id == first.id }?.state, .denied)
        XCTAssertFalse(CircleLogic.groups(after).flatMap(\.reviews).contains { $0.id == first.id })
    }
}
