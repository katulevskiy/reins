import AuthenticationServices
import CryptoKit
import XCTest
@testable import Reins

/// A passkey sheet that answers with what the test scripts, and remembers what it was asked (the Android app's
/// `FakePasskeys`).
@MainActor
final class ScriptedPasskeys: PasskeyPrompting {
    var created: [PasskeyResult]
    var used: [PasskeyResult]
    private(set) var asked: [[Data]] = []
    private(set) var creations = 0

    init(created: [PasskeyResult] = [], used: [PasskeyResult] = []) {
        self.created = created
        self.used = used
    }

    func create(_ options: VaultPasskeyOptions) async -> PasskeyResult {
        creations += 1
        return created.isEmpty ? .cancelled : created.removeFirst()
    }

    func get(_ options: VaultPasskeyOptions, allowed: [Data]) async -> PasskeyResult {
        asked.append(allowed)
        return used.isEmpty ? .cancelled : used.removeFirst()
    }
}

@MainActor
final class VaultPasskeyTests: XCTestCase {
    private let prf = DemoReinsCore.demoPrfOutput
    private let newId = Data("new-passkey".utf8)

    // MARK: What the platform said

    func testARegistrationGivesThePrfOutputOrSaysWhetherTheProviderHasPrf() {
        XCTAssertEqual(PasskeyParse.registration(credentialId: newId, prfSupported: true, first: prf), .passkey(credentialId: newId, prf: prf))
        // Some providers give the output only as the passkey is used: it is asked for then.
        XCTAssertEqual(PasskeyParse.registration(credentialId: newId, prfSupported: true, first: nil), .passkey(credentialId: newId, prf: nil))
        XCTAssertEqual(PasskeyParse.registration(credentialId: newId, prfSupported: false, first: nil), .unsupported)
        XCTAssertEqual(PasskeyParse.registration(credentialId: newId, prfSupported: nil, first: nil), .unsupported)
        XCTAssertEqual(PasskeyParse.registration(credentialId: newId, prfSupported: true, first: Data()), .passkey(credentialId: newId, prf: nil))
        guard case .failed = PasskeyParse.registration(credentialId: Data(), prfSupported: true, first: prf) else { return XCTFail("no id") }
    }

    func testAnAssertionWithoutPrfCannotOpenTheVault() {
        XCTAssertEqual(PasskeyParse.assertion(credentialId: newId, first: prf), .passkey(credentialId: newId, prf: prf))
        XCTAssertEqual(PasskeyParse.assertion(credentialId: newId, first: nil), .unsupported)
    }

    func testClosingTheSheetIsNoErrorAndOtherErrorsSayWhatHappened() {
        XCTAssertEqual(PasskeyParse.error(ASAuthorizationError(.canceled), creating: true), .cancelled)
        XCTAssertEqual(PasskeyParse.error(ASAuthorizationError(.canceled), creating: false), .cancelled)
        XCTAssertEqual(PasskeyParse.error(ASAuthorizationError(.notHandled), creating: false), .unsupported)
        XCTAssertEqual(PasskeyParse.error(ASAuthorizationError(.matchedExcludedCredential), creating: true),
                       .failed("This password manager has a passkey for your vault already."))
        XCTAssertEqual(PasskeyParse.error(ASAuthorizationError(.failed), creating: true), .failed("The passkey could not be made. Try again."))
        XCTAssertEqual(PasskeyParse.error(CocoaError(.featureUnsupported), creating: false), .failed("The passkey could not be used. Try again."))
    }

    func testThePrfOutputIsTheKeysBytes() {
        let key = SymmetricKey(data: prf)
        XCTAssertEqual(PasskeyParse.bytes(key), prf)
    }

    func testAPasskeyIsNamedAfterThePhone() {
        XCTAssertEqual(VaultPasskeys.deviceName(name: "Dana's iPhone", model: "iPhone"), "Dana's iPhone")
        XCTAssertEqual(VaultPasskeys.deviceName(name: "  ", model: "iPhone"), "iPhone")
    }

    // MARK: Adding one

    private func core(passkeys: Bool = false, locked: Bool = false) -> DemoReinsCore {
        DemoReinsCore(syncCap: 0.01, keysLocked: locked, passkeys: passkeys)
    }

    func testAddingKeepsTheVaultsCopyForTheNewPasskey() async throws {
        let core = core()
        let prompt = ScriptedPasskeys(created: [.passkey(credentialId: newId, prf: prf)])
        let list = try await VaultPasskeys.add(core: core, prompt: prompt, name: "iPhone")
        XCTAssertEqual(list?.map(\.credentialId), [newId])
        XCTAssertEqual(list?.first?.name, "iPhone")
        XCTAssertEqual(prompt.asked, [], "the output came with the passkey: no second prompt")
    }

    func testAProviderThatGivesThePrfOnlyOnUseIsAskedOnceMoreForTheNewPasskey() async throws {
        let core = core()
        let prompt = ScriptedPasskeys(created: [.passkey(credentialId: newId, prf: nil)], used: [.passkey(credentialId: newId, prf: prf)])
        let list = try await VaultPasskeys.add(core: core, prompt: prompt, name: "iPhone")
        XCTAssertEqual(prompt.asked, [[newId]])
        XCTAssertEqual(list?.map(\.credentialId), [newId])
    }

    func testAPasswordManagerWithoutPrfAddsNothingAndSaysSo() async throws {
        let core = core()
        for prompt in [
            ScriptedPasskeys(created: [.unsupported]),
            ScriptedPasskeys(created: [.passkey(credentialId: newId, prf: nil)], used: [.unsupported]),
        ] {
            do {
                _ = try await VaultPasskeys.add(core: core, prompt: prompt, name: "iPhone")
                XCTFail("added")
            } catch let error as PasskeyError {
                XCTAssertEqual(error.message, VaultPasskeys.unsupported)
                XCTAssertEqual(error.localizedDescription, VaultPasskeys.unsupported)
            }
        }
        let left = try await core.vaultPasskeys()
        XCTAssertEqual(left, [])
    }

    func testClosingTheSheetAddsNothing() async throws {
        let core = core()
        let none = try await VaultPasskeys.add(core: core, prompt: ScriptedPasskeys(created: [.cancelled]), name: "iPhone")
        XCTAssertNil(none)
        let late = try await VaultPasskeys.add(
            core: core, prompt: ScriptedPasskeys(created: [.passkey(credentialId: newId, prf: nil)], used: [.cancelled]), name: "iPhone"
        )
        XCTAssertNil(late)
        let left = try await core.vaultPasskeys()
        XCTAssertEqual(left, [])
    }

    // MARK: Unlocking with one

    func testAPasskeyOpensALockedVault() async throws {
        let core = core(passkeys: true, locked: true)
        let prompt = ScriptedPasskeys(used: [.passkey(credentialId: DemoReinsCore.demoPasskey.credentialId, prf: prf)])
        let opened = try await VaultPasskeys.unlock(core: core, prompt: prompt)
        XCTAssertTrue(opened)
        XCTAssertEqual(prompt.asked, [[DemoReinsCore.demoPasskey.credentialId]], "only the account's passkeys are offered")
        let keys = try await core.accountKeys()
        XCTAssertEqual(keys, .unlocked)
    }

    func testNoPasskeyOnThisPhoneSaysWhatElseOpensTheVault() async throws {
        let core = core(passkeys: true, locked: true)
        for result in [PasskeyResult.unsupported, .passkey(credentialId: DemoReinsCore.demoPasskey.credentialId, prf: nil)] {
            do {
                _ = try await VaultPasskeys.unlock(core: core, prompt: ScriptedPasskeys(used: [result]))
                XCTFail("opened")
            } catch let error as PasskeyError {
                XCTAssertEqual(error.message, VaultPasskeys.noneHere)
            }
        }
        let closed = try await VaultPasskeys.unlock(core: core, prompt: ScriptedPasskeys(used: [.cancelled]))
        XCTAssertFalse(closed)
        let keys = try await core.accountKeys()
        XCTAssertEqual(keys, .locked)
    }

    // MARK: The offer before the recovery code

    private func withUnrecordedCode(_ body: () async throws -> Void) async rethrows {
        let key = RecoveryRecord.key(server: DemoData.server, code: DemoData.recoveryCode)
        let previous = AppGroup.defaults.object(forKey: key)
        AppGroup.defaults.removeObject(forKey: key)
        defer {
            if let previous { AppGroup.defaults.set(previous, forKey: key) } else { AppGroup.defaults.removeObject(forKey: key) }
            DeviceStatus.clear()
        }
        try await body()
    }

    private func model(_ core: DemoReinsCore) -> AppModel {
        AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator())
    }

    func testANewVaultWithoutAPasskeyIsOfferedOneBeforeTheRecoveryCode() async {
        await withUnrecordedCode {
            let app = model(core())
            await app.refreshSession()
            XCTAssertTrue(app.passkeyOffer)
            XCTAssertEqual(app.recoveryToRecord, DemoData.recoveryCode, "the code follows")
            app.vaultPasskeyAdded()
            XCTAssertFalse(app.passkeyOffer)
            XCTAssertEqual(app.recoveryToRecord, DemoData.recoveryCode, "and is still required")
        }
    }

    func testDecliningGoesToTheCodeAndTheOfferStaysAwayForThisSignIn() async {
        await withUnrecordedCode {
            let app = model(core())
            await app.refreshSession()
            app.declinePasskeyOffer()
            XCTAssertFalse(app.passkeyOffer)
            XCTAssertEqual(app.recoveryToRecord, DemoData.recoveryCode)
            await app.refreshSession()
            XCTAssertFalse(app.passkeyOffer, "declined for this sign-in")
            // A new sign-in offers it again.
            app.setSession(.signedOut)
            await app.refreshSession()
            XCTAssertTrue(app.passkeyOffer)
        }
    }

    func testNoOfferWhenTheAccountHasAPasskeyTheListIsUnknownOrTheCodeIsRecorded() async {
        await withUnrecordedCode {
            let withPasskey = model(core(passkeys: true))
            await withPasskey.refreshSession()
            XCTAssertFalse(withPasskey.passkeyOffer)
            XCTAssertNotNil(withPasskey.recoveryToRecord)

            let offline = model(DemoReinsCore(syncCap: 0.01, passkeys: false, passkeysOffline: true))
            await offline.refreshSession()
            XCTAssertFalse(offline.passkeyOffer, "offline never blocks")
            XCTAssertNotNil(offline.recoveryToRecord)

            RecoveryRecord.confirm(server: DemoData.server, code: DemoData.recoveryCode)
            let recorded = model(core())
            await recorded.refreshSession()
            XCTAssertFalse(recorded.passkeyOffer)
            XCTAssertNil(recorded.recoveryToRecord)
        }
    }

    // MARK: Settings

    func testTheRowSaysHowManyPasskeysOpenTheVault() {
        XCTAssertEqual(SettingsText.passkeysSummary(nil), "Unlock your vault on a new phone")
        XCTAssertEqual(SettingsText.passkeysSummary(0), "None: add one to unlock on a new phone")
        XCTAssertEqual(SettingsText.passkeysSummary(1), "1 passkey")
        XCTAssertEqual(SettingsText.passkeysSummary(3), "3 passkeys")
        XCTAssertTrue(SettingsText.passkeyAdded(1_760_000_000).hasPrefix("Added "))
    }

    func testAddingAndRemovingInSettingsKeepsTheList() async {
        let core = core(passkeys: true)
        let app = model(core)
        await app.refreshSession()
        app.passkeys = ScriptedPasskeys(created: [.passkey(credentialId: newId, prf: prf)])
        let vm = VaultPasskeysModel()
        await vm.load(app)
        XCTAssertEqual(vm.passkeys?.count, 1)
        let added = await vm.add(app)
        XCTAssertTrue(added)
        XCTAssertEqual(vm.passkeys?.map(\.credentialId), [DemoReinsCore.demoPasskey.credentialId, newId])
        await vm.remove(DemoReinsCore.demoPasskey.credentialId, app)
        XCTAssertEqual(vm.passkeys?.map(\.credentialId), [newId])
        XCTAssertNil(vm.error)
        app.passkeys = ScriptedPasskeys(created: [.unsupported])
        let refused = await vm.add(app)
        XCTAssertFalse(refused)
        XCTAssertEqual(vm.error, VaultPasskeys.unsupported)
        XCTAssertEqual(VaultPasskeysModel.tag(Data([0xfb, 0xff])), "-_8")
    }
}
