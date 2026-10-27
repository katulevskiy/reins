import UserNotifications
import XCTest
@testable import Reins

/// Personal guardianship (decisions v2 P7, 04 §4; the Android app's `GuardianFlowTest`): the ward picks the notice up
/// front, a minor's leaving waits for the guardian, what the phone could not check says "Reported by the server", the
/// weekly summary is the ward's switch alone, and a missing age signal never blocks anything.
@MainActor
final class GuardianshipTests: XCTestCase {
    private let now: Int64 = 1_700_000_000

    private func person(
        _ id: String, peer: String, role: CircleRole = .guardian, mine: Bool, state: CircleLinkState = .active,
        ending: Int64? = nil, needsConsent: Bool = false
    ) -> CirclePerson {
        CirclePerson(
            linkId: id, peer: peer, role: role, mine: mine, scopeLine: "Money", deferred: true, windowSecs: 14_400, state: state,
            since: 0, ending: ending, needsConsent: needsConsent
        )
    }

    private func ward(
        minor: Bool = false, minorProposed: Bool = false, noticeDays: UInt8 = 3, ending: Int64? = nil, needsConsent: Bool = false,
        digest: Bool = false
    ) -> WardView {
        WardView(
            linkId: "l6", guardian: "mom@family.example", scopeLine: "Money", noticeDays: noticeDays, minor: minor,
            minorProposed: minorProposed, decisions: [], ending: ending, needsConsent: needsConsent, digest: digest
        )
    }

    private func notice(_ id: String, verified: Bool, needsConsent: Bool = false, line: String = "Ended early", at: Int64 = 0) -> GuardianNoticeView {
        GuardianNoticeView(id: id, linkId: "l3", ward: "sam.lee@example.com", line: line, at: at, verified: verified, needsConsent: needsConsent)
    }

    // MARK: The notice, chosen up front

    func testWardChoosesNoticeUpFront() async throws {
        XCTAssertEqual(GuardianLogic.noticeChoices, [0, 3, 7], "0, 3 or 7 days: never more, never permanent")
        XCTAssertEqual(GuardianLogic.noticeLine(3), "Ends after 3 days' notice")
        XCTAssertEqual(GuardianLogic.noticeLine(7), "Ends after 7 days' notice")
        XCTAssertEqual(GuardianLogic.noticeLine(0), "Ends right away")
        XCTAssertEqual(GuardianLogic.noticeChoices.map(GuardianLogic.noticeLabel), ["None", "3 days", "7 days"])

        // The link waits for the ward's pick; the other roles never carry one.
        XCTAssertFalse(GuardianLogic.canMakeLink(role: .guardian, noticeDays: nil))
        XCTAssertFalse(GuardianLogic.canMakeLink(role: .guardian, noticeDays: 30), "only the offered choices")
        XCTAssertTrue(GuardianLogic.canMakeLink(role: .guardian, noticeDays: 0))
        XCTAssertTrue(GuardianLogic.canMakeLink(role: .cosign, noticeDays: nil))
        XCTAssertEqual(GuardianLogic.draft(role: .guardian, preset: .money, deferred: true, noticeDays: 7).noticeDays, 7)
        XCTAssertNil(GuardianLogic.draft(role: .guardian, preset: .money, deferred: true, noticeDays: 30).noticeDays)
        XCTAssertNil(GuardianLogic.draft(role: .delegate, preset: .money, deferred: true, noticeDays: 3).noticeDays)
        XCTAssertNil(CircleLogic.draft(role: .watcher, preset: .everything, deferred: true).noticeDays)

        // The Add sheet: Guardian, no link until the notice is picked, then it goes in the draft.
        let add = AddCircleModel()
        add.role = .guardian
        XCTAssertFalse(add.canMakeLink)
        add.noticeDays = 3
        XCTAssertTrue(add.canMakeLink)
        XCTAssertEqual(add.draft.noticeDays, 3)

        // The demo core holds to it, and the ward's view carries the pick once confirmed.
        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: false)
        let noNotice = GuardianLogic.draft(role: .guardian, preset: .money, deferred: true, noticeDays: nil)
        do {
            _ = try await core.circleInviteCreate(draft: noNotice)
            XCTFail("a guardianship needs the ward's notice")
        } catch {}
        let start = try await core.circleInviteCreate(draft: add.draft)
        for _ in 0..<3 { _ = try await core.circleInvitePoll(id: start.id) }
        let person = try await core.circleInviteConfirm(id: start.id)
        XCTAssertTrue(GuardianLogic.isWard(person))
        let made = try await core.guardianWardView(linkId: person.linkId)
        XCTAssertEqual(made.noticeDays, 3)

        // The guardian's join sheet shows the ward's notice before Accept.
        let opened = try await core.circleJoinOpen(link: "reins-circle:demo.reins.dev/x-k3Hq9VxT2")
        let terms = try await core.circleJoinPoll(id: opened.id)
        XCTAssertEqual(terms.role, .guardian)
        XCTAssertEqual(terms.stage, .compare)
        XCTAssertEqual(terms.noticeDays, 3)
    }

    // MARK: Leaving

    func testMinorLeaveWaitsForConsent() async throws {
        // An adult's: Leave, then the countdown while the obligations stay.
        XCTAssertEqual(GuardianLogic.leave(ward()), .available(noticeDays: 3))
        XCTAssertEqual(GuardianLogic.leave(ward(ending: now + 3 * 86_400)), .ending(at: now + 3 * 86_400))
        XCTAssertEqual(GuardianLogic.endingLine(now + 2 * 86_400 + 4 * 3_600, now: now), "Ends in 2 d 4 h")
        XCTAssertEqual(GuardianLogic.endingLine(now + 5 * 3_600, now: now), "Ends in 5 h")
        XCTAssertEqual(GuardianLogic.endingLine(now + 90, now: now), "Ends in 2 min")
        XCTAssertEqual(GuardianLogic.endingLine(now - 1, now: now), "Ending")
        XCTAssertEqual(GuardianLogic.leaveMessage(ward()), "Ends after 3 days' notice. Mom is told.")

        // A minor's: it waits for the guardian, on both sides.
        XCTAssertEqual(GuardianLogic.leave(ward(minor: true, needsConsent: true)), .waitingForConsent)
        XCTAssertEqual(GuardianLogic.leaveMessage(ward(minor: true)), "Mom is asked to agree.")
        let mine = person("l6", peer: "mom@family.example", mine: true, needsConsent: true)
        XCTAssertEqual(GuardianLogic.badge(mine, now: now), "Waiting for Mom")
        XCTAssertFalse(CircleLogic.canEnd(mine), "asked once")
        let theirs = person("l3", peer: "sam.lee@example.com", mine: false, needsConsent: true)
        XCTAssertEqual(GuardianLogic.badge(theirs, now: now), "Asked to leave")
        XCTAssertEqual(GuardianLogic.consents([theirs, mine]).map(\.linkId), ["l3"], "only the guardian is asked")
        XCTAssertEqual(GuardianLogic.consentLine(theirs.peer), "Sam asked to leave. Allow?")

        // The ward never pauses its guardianship; it leaves.
        let running = person("l6", peer: "mom@family.example", mine: true)
        XCTAssertFalse(CircleLogic.canPause(running))
        XCTAssertEqual(CircleLogic.endTitle(running), "Leave")
        XCTAssertTrue(CircleLogic.canPause(person("l1", peer: "a@x.com", role: .cosign, mine: true)), "other owners still pause")

        // The demo: accept the mark, ask to leave, and it waits; the guardian's Allow ends it.
        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: true)
        try await core.guardianAcceptMinor(linkId: "l6", signal: AgeSignal.unavailable.input)
        try await core.circleEnd(linkId: "l6")
        let view = try await core.guardianWardView(linkId: "l6")
        XCTAssertEqual(GuardianLogic.leave(view), .waitingForConsent)
        let still = try await core.circlePeople(refresh: false).first { $0.linkId == "l6" }
        XCTAssertEqual(still?.state, .active, "the obligations stay until the guardian agrees")
        do {
            try await core.circlePause(linkId: "l6", paused: true)
            XCTFail("a ward can't pause its guardian")
        } catch {}
    }

    // MARK: The guardian's notices

    func testUnverifiedNoticeIsLabelled() {
        XCTAssertEqual(GuardianLogic.source(notice("a", verified: false)), "Reported by the server")
        XCTAssertNil(GuardianLogic.source(notice("b", verified: true)), "what the phone checked shows as it is")
        XCTAssertEqual(GuardianLogic.notices([notice("old", verified: true, at: 1), notice("new", verified: false, at: 2)]).map(\.id), ["new", "old"])
        XCTAssertEqual(GuardianLogic.noticeText(notice("c", verified: true, line: "")), "Guardianship changed")

        // The one notification: titled by the ward's short name, opening Circles.
        let c = NotificationContent.guardianNotice(notice("a", verified: false), settings: FeedbackSettings())
        XCTAssertEqual(c.title, "Sam: guardianship changed")
        XCTAssertEqual(c.body, "Ended early\nReported by the server")
        XCTAssertEqual(c.categoryIdentifier, NotificationCategory.circle.rawValue)
        XCTAssertEqual(c.userInfo[NotificationKey.id] as? String, "alert:l3", "the same id the push names")
        XCTAssertEqual(c.userInfo[NotificationKey.link] as? String, DeepLink.circles.url.absoluteString)
        XCTAssertEqual(DeepLink(url: DeepLink.circles.url), .circles)
        let ask = NotificationContent.guardianNotice(notice("d", verified: true, needsConsent: true, line: "Asked to leave"), settings: FeedbackSettings())
        XCTAssertEqual(ask.body, "Sam asked to leave. Allow?")

        // A guardianship's alert the extension found no notice for says it changed, never that it ended.
        let active = person("l3", peer: "sam.lee@example.com", mine: false)
        XCTAssertEqual(NotificationText.circleAlertBody(active, now: now), "Guardianship changed")
        XCTAssertEqual(NotificationText.circleAlertBody(person("l3", peer: "sam.lee@example.com", mine: false, state: .ended), now: now), "Guardianship ended")
        XCTAssertEqual(NotificationText.circleAlertBody(person("l3", peer: "sam.lee@example.com", mine: false, ending: now + 3 * 86_400), now: now),
                       "Asked to leave · Ends in 3 d")
    }

    // MARK: The weekly summary

    func testDigestSwitchIsWardOnly() async throws {
        XCTAssertTrue(GuardianLogic.showsDigestSwitch(person("l6", peer: "mom@family.example", mine: true)))
        XCTAssertFalse(GuardianLogic.showsDigestSwitch(person("l3", peer: "sam.lee@example.com", mine: false)), "never the guardian's")
        XCTAssertFalse(GuardianLogic.showsDigestSwitch(person("l1", peer: "a@x.com", role: .watcher, mine: true)))
        XCTAssertFalse(GuardianLogic.showsDigestSwitch(person("l6", peer: "mom@family.example", mine: true, state: .ended)))
        XCTAssertEqual(GuardianLogic.digestLabel("mom@family.example"), "Weekly summary for Mom")
        XCTAssertFalse(ward().digest, "off until the ward turns it on")

        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: true)
        try await core.guardianSetDigest(linkId: "l6", on: true)
        let on = try await core.guardianWardView(linkId: "l6")
        XCTAssertTrue(on.digest)
        do {
            try await core.guardianSetDigest(linkId: "l3", on: true)
            XCTFail("the guardian has no switch")
        } catch {}
    }

    // MARK: The age signal

    func testUnavailableAgeSignalNeverBlocks() async throws {
        XCTAssertEqual(AgeSignal.of(lower: nil, upper: nil), .unavailable)
        XCTAssertEqual(AgeSignal.of(lower: 13, upper: 15), .minor(upper: 15))
        XCTAssertEqual(AgeSignal.of(lower: 16, upper: 17), .minor(upper: 17))
        XCTAssertEqual(AgeSignal.of(lower: 18, upper: nil), .adult)
        XCTAssertEqual(AgeSignal.unavailable.input, .unavailable)
        XCTAssertEqual(AgeSignal.minor(upper: 15).input, .minor(upper: 15, platform: .apple))
        XCTAssertEqual(AgeSignal.adult.input, .adult)
        let none = await FixedAgeSignals().check()
        XCTAssertEqual(none, .unavailable)

        // Setup never asks for it.
        XCTAssertTrue(GuardianLogic.canMakeLink(role: .guardian, noticeDays: 3))

        // Accepting a minor's mark goes through with no signal (the ward's own word); an adult's is refused.
        let core = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: true)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        let vm = GuardianWardModel(linkId: "l6", signals: FixedAgeSignals(signal: .unavailable))
        await vm.load(model)
        XCTAssertTrue(GuardianLogic.showsMinorCard(try XCTUnwrap(vm.view)))
        await vm.acceptMinor(model)
        XCTAssertNil(vm.error)
        XCTAssertEqual(vm.view?.minor, true)
        XCTAssertFalse(GuardianLogic.showsMinorCard(try XCTUnwrap(vm.view)))

        let adult = DemoReinsCore(signedIn: true, syncCap: 0.1, quiet: true, circles: true)
        let adultModel = AppModel(core: adult, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        let refused = GuardianWardModel(linkId: "l6", signals: FixedAgeSignals(signal: .adult))
        await refused.load(adultModel)
        await refused.acceptMinor(adultModel)
        XCTAssertNotNil(refused.error, "an adult's account isn't a minor's")
        XCTAssertEqual(refused.view?.minor, false)
    }
}
