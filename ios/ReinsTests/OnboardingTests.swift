import XCTest
@testable import Reins

/// Pairing codes (typed, scanned, linked), the create-account checks, the onboarding record, and the demo core's
/// `createAccount` and `pairingByCode`.
@MainActor
final class OnboardingTests: XCTestCase {
    // MARK: Pairing codes

    func testACodeIsNormalizedWhateverItsCaseSpacesAndDashes() {
        XCTAssertEqual(PairingCode.normalize("BCDF-GHJK"), "BCDF-GHJK")
        XCTAssertEqual(PairingCode.normalize("bcdfghjk"), "BCDF-GHJK")
        XCTAssertEqual(PairingCode.normalize(" bcdf ghjk "), "BCDF-GHJK")
        XCTAssertEqual(PairingCode.normalize("B-C-D-F-G-H-J-K"), "BCDF-GHJK")
    }

    func testOnlyEightLettersOfTheAlphabetMakeACode() {
        XCTAssertNil(PairingCode.normalize("BCDF-GHJ"), "too short")
        XCTAssertNil(PairingCode.normalize("BCDF-GHJKL"), "too long")
        XCTAssertNil(PairingCode.normalize("BCDF-GHJA"), "vowels are not in the alphabet")
        XCTAssertNil(PairingCode.normalize("BCDF-GHJ1"), "nor digits")
        XCTAssertNil(PairingCode.normalize("BCDF_GHJK"))
        XCTAssertNil(PairingCode.normalize(""))
    }

    func testTheCodeIsTakenFromLinksOnAnyServerAndFromBareText() {
        XCTAssertEqual(PairingCode.parse("https://app.reins2fa.com/pair?code=BCDF-GHJK"), "BCDF-GHJK")
        XCTAssertEqual(PairingCode.parse("https://vault.example.com/reins/pair?code=bcdfghjk"), "BCDF-GHJK", "a self-hosted server")
        XCTAssertEqual(PairingCode.parse("http://127.0.0.1:8000/pair?code=BCDF-GHJK"), "BCDF-GHJK", "a local test server")
        XCTAssertEqual(PairingCode.parse("reins://pair?code=BCDF-GHJK"), "BCDF-GHJK")
        XCTAssertEqual(PairingCode.parse("  BCDF-GHJK\n"), "BCDF-GHJK")
    }

    func testAnythingElseScannedIsNotACode() {
        XCTAssertNil(PairingCode.parse("https://app.reins2fa.com/login?code=BCDF-GHJK"), "not /pair")
        XCTAssertNil(PairingCode.parse("https://app.reins2fa.com/pair"), "no code")
        XCTAssertNil(PairingCode.parse("https://app.reins2fa.com/pair?code=BCDF-GHJA"), "not a code")
        XCTAssertNil(PairingCode.parse("reins://item?code=BCDF-GHJK"))
        XCTAssertNil(PairingCode.parse("ftp://example.com/pair?code=BCDF-GHJK"))
        XCTAssertNil(PairingCode.parse("WIFI:S:home;T:WPA;P:secret;;"))
        XCTAssertNil(PairingCode.parse(String(repeating: "B", count: 5000)))
    }

    func testThePairLinkRoundTrips() {
        let link = DeepLink.pair(code: "BCDF-GHJK")
        XCTAssertEqual(link.url.absoluteString, "reins://pair?code=BCDF-GHJK")
        XCTAssertEqual(DeepLink(url: link.url), link)
        XCTAssertEqual(DeepLink(url: URL(string: "reins://pair?code=bcdf%20ghjk")!), link, "normalized")
        XCTAssertNil(DeepLink(url: URL(string: "reins://pair?code=../x")!))
        XCTAssertNil(DeepLink(url: URL(string: "reins://pair")!))
    }

    func testAnOpenedUniversalLinkBecomesAPairLinkAndOtherWebLinksNothing() {
        XCTAssertEqual(DeepLink.opened(URL(string: "https://app.reins2fa.com/pair?code=BCDF-GHJK")!), .pair(code: "BCDF-GHJK"))
        XCTAssertEqual(DeepLink.opened(URL(string: "reins://home")!), .home)
        XCTAssertNil(DeepLink.opened(URL(string: "https://app.reins2fa.com/")!))
    }

    // MARK: Creating an account

    private func problem(email: String = "me@example.com", password: String = "correct horse battery",
                         again: String? = nil, terms: Bool = true) -> String? {
        NewAccountRules.problem(server: SignInState.defaultServer, email: email, password: password, again: again ?? password, terms: terms)
    }

    func testANewAccountNeedsTwelveCharactersMatchingPasswordsAndTheTerms() {
        XCTAssertNil(problem())
        XCTAssertNil(problem(password: "123456789012"), "exactly 12")
        XCTAssertEqual(problem(password: "12345678901"), "The master password needs at least 12 characters.")
        XCTAssertEqual(problem(again: "correct horse battery!"), "The two passwords do not match.")
        XCTAssertEqual(problem(terms: false), "Accept the Terms to continue.")
        XCTAssertEqual(problem(email: " "), "Enter your email address.")
        XCTAssertEqual(NewAccountRules.problem(server: "https://", email: "me@example.com", password: "123456789012", again: "123456789012", terms: true),
                       "Enter the server's address.")
    }

    func testTheOnboardingShowsOncePerAccountAndServer() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "onboarding-tests"))
        defaults.removePersistentDomain(forName: "onboarding-tests")
        XCTAssertTrue(OnboardingRecord.firstTime(server: "https://app.reins2fa.com", email: "Me@Example.com", defaults: defaults))
        XCTAssertFalse(OnboardingRecord.firstTime(server: "https://APP.reins2fa.com/", email: "me@example.com", defaults: defaults))
        XCTAssertTrue(OnboardingRecord.firstTime(server: "https://vault.example.com", email: "me@example.com", defaults: defaults))
        XCTAssertTrue(OnboardingRecord.firstTime(server: "https://app.reins2fa.com", email: "other@example.com", defaults: defaults))
        defaults.removePersistentDomain(forName: "onboarding-tests")
    }

    // MARK: The demo core

    func testTheDemoCoreCreatesAnAccountLikeTheServer() async throws {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3)
        do {
            _ = try await core.createAccount(serverUrl: DemoData.server, email: "new@example.com", password: "short")
            XCTFail("a short password is refused")
        } catch let CoreError.Invalid(reason) {
            XCTAssertTrue(reason.contains("12"))
        }
        do {
            _ = try await core.createAccount(serverUrl: DemoData.server, email: "taken@example.com", password: "correct horse battery")
            XCTFail("a registered email is refused")
        } catch CoreError.Invalid {}
        let session = await core.session()
        XCTAssertNil(session)
        let info = try await core.createAccount(serverUrl: DemoData.server, email: "new@example.com", password: "correct horse battery")
        XCTAssertEqual(info.email, "new@example.com")
        let after = await core.session()
        XCTAssertEqual(after, info)
    }

    func testADemoPairingCodeParksADesktopPairingToAnswer() async throws {
        let core = DemoReinsCore(syncCap: 0.3)
        let view = try await core.pairingByCode(userCode: "bcdf ghjk")
        XCTAssertNotNil(view.keyFingerprint)
        XCTAssertEqual(view.choices.count, 3)
        let parked = try await core.pairingView(pairingId: view.id)
        XCTAssertEqual(parked, view)
        let pending = try await core.pending()
        XCTAssertTrue(pending.contains { $0.id == view.id && $0.kind == .pairing })
        let again = try await core.pairingByCode(userCode: "BCDF-GHJK")
        XCTAssertEqual(again.id, view.id, "the same code parks one pairing")

        let before = try await core.connections().count
        try await core.answerPairing(pairingId: view.id, approve: true, chosenCode: view.choices.first, label: nil)
        let connections = try await core.connections()
        XCTAssertEqual(connections.count, before + 1)
        XCTAssertEqual(connections.last?.label, "Reins desktop app on studio")
    }

    func testAnExpiredCodeSaysToShowANewOne() async {
        let core = DemoReinsCore(syncCap: 0.3)
        do {
            _ = try await core.pairingByCode(userCode: "BBBB-CDFG")
            XCTFail("expired")
        } catch {
            XCTAssertEqual(AppModel.pairingCodeMessage(error), "This code has expired or was already used. Show a new one on your computer.")
        }
    }

    // MARK: The app model

    func testAPairLinkOpenedWhileSignedOutWaitsForTheSignInAndThenOpensThePairing() async throws {
        let core = DemoReinsCore(signedIn: false, syncCap: 0.3)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        await model.handle(.pair(code: "BCDF-GHJK"))
        XCTAssertEqual(model.waitingPairCode, "BCDF-GHJK")
        XCTAssertNil(model.sheet)

        let info = try await core.createAccount(serverUrl: DemoData.server, email: "new@example.com", password: "correct horse battery")
        await model.finishSignIn(info)
        XCTAssertTrue(model.onboarding)
        XCTAssertTrue(model.approvalDevice)
        XCTAssertNil(model.waitingPairCode)
        XCTAssertEqual(model.sheet, .pairing("code-BCDF-GHJK"))
        model.finishOnboarding()
        XCTAssertFalse(model.onboarding)
    }
}
