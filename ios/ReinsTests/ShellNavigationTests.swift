import XCTest
@testable import Reins

/// The four places (Activity, Trust, Connections, Settings): where links and the old routes land now that Autopilot and
/// the grants live on Trust and the integrations on Connections, and what those screens say.
@MainActor
final class ShellNavigationTests: XCTestCase {
    private var model: AppModel!

    private func start() async {
        model = AppModel(core: DemoReinsCore(signedIn: true), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        await model.refreshPending()
    }

    func testTheFourPlacesAndTheirTabIdentifiers() {
        XCTAssertEqual(AppSection.allCases, [.activity, .trust, .connections, .settings])
        XCTAssertEqual(AppSection.allCases.map(\.identifier), ["tab:activity", "tab:trust", "tab:connections", "tab:settings"])
        XCTAssertEqual(AppSection.allCases.map(\.glyph), [.activity, .shield, .link, .settings])
    }

    func testOldLinksLandOnTheirNewPlaces() async {
        await start()
        model.show(.sounds, in: .settings)
        await model.handle(.autopilot)
        XCTAssertEqual(model.section, .trust, "Autopilot lives on Trust")
        XCTAssertEqual(model.path(.trust), [])

        await model.handle(.integrations)
        XCTAssertEqual(model.section, .connections, "the integrations live on Connections")
        XCTAssertEqual(model.path(.connections), [])

        let grant = try? XCTUnwrap(model.grants.first)
        if let grant {
            await model.handle(.grant(id: grant.id))
            XCTAssertEqual(model.section, .trust, "a grant opens on Trust")
            XCTAssertEqual(model.path(.trust), [.grantDetail(grant.id)])
        }
        XCTAssertEqual(model.path(.settings), [.sounds], "other places keep what is open on them")
    }

    func testOpeningAPlaceShowsItsRoot() async {
        await start()
        model.show(.expiredGrants, in: .trust)
        model.open(.activity)
        XCTAssertEqual(model.section, .activity)
        model.open(.trust)
        XCTAssertEqual(model.section, .trust)
        XCTAssertEqual(model.path(.trust), [], "Autopilot settings from Activity's menu is Trust's root")
    }

    func testAutopilotOpensItsPagesFromTrustsRoot() async {
        await start()
        model.open(.trust)
        model.openFromAutopilot(.tryIt(nil))
        XCTAssertEqual(model.path(.trust), [.tryIt(nil)])
        model.openFromAutopilot(.autopilotProfile("p1"))
        XCTAssertEqual(model.path(.trust), [.tryIt(nil), .autopilotProfile("p1")], "deeper, one more on top")
    }

    func testConnectionsSaysWhoCanAsk() {
        XCTAssertEqual(ConnectionsText.mood(ais: 3, computers: 1), "Three AIs and a computer can ask.")
        XCTAssertEqual(ConnectionsText.mood(ais: 1, computers: 0), "One AI can ask.")
        XCTAssertEqual(ConnectionsText.mood(ais: 0, computers: 2), "Two computers can ask.")
        XCTAssertEqual(ConnectionsText.mood(ais: 12, computers: 1), "12 AIs and a computer can ask.")
        XCTAssertEqual(ConnectionsText.mood(ais: 0, computers: 0), "Nothing can ask yet.")
    }

    func testTrustSaysWhatAutopilotDid() {
        XCTAssertNil(TrustText.mood(nil))
        XCTAssertEqual(TrustText.mood(AutopilotStats(days: 7, scored: 87, matched: 0.96, compared: 80, kindsOnAuto: 1, kinds: 4)), "Autopilot scored 87 requests this week.")
        XCTAssertEqual(TrustText.mood(AutopilotStats(days: 7, scored: 1, matched: nil, compared: 0, kindsOnAuto: 0, kinds: 0)), "Autopilot scored one request this week.")
        XCTAssertEqual(TrustText.mood(AutopilotStats(days: 7, scored: 0, matched: nil, compared: 0, kindsOnAuto: 0, kinds: 0)), "Autopilot has not scored anything this week.")
    }
}
