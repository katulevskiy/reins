import XCTest
@testable import Reins

@MainActor
final class RecoveryRecordTests: XCTestCase {
    func testAcknowledgementIsSpecificToTheServerAndSecretAndDoesNotStoreTheSecret() {
        let name = "recovery-record-tests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let server = "https://app.example.com"
        let code = DemoData.recoveryCode
        XCTAssertFalse(RecoveryRecord.confirmed(server: server, code: code, defaults: defaults))
        RecoveryRecord.confirm(server: server + "/", code: code, defaults: defaults)
        XCTAssertTrue(RecoveryRecord.confirmed(server: server.uppercased(), code: code, defaults: defaults))
        XCTAssertFalse(RecoveryRecord.confirmed(server: "https://other.example.com", code: code, defaults: defaults))
        XCTAssertFalse(RecoveryRecord.confirmed(server: server, code: code + "X", defaults: defaults))
        XCTAssertFalse(RecoveryRecord.key(server: server, code: code).contains(code))
    }

    func testRecordingIsRequiredAfterRestartAndDoesNotFollowTheEmail() async {
        let core = DemoReinsCore(syncCap: 0.01)
        let key = RecoveryRecord.key(server: DemoData.server, code: DemoData.recoveryCode)
        let previous = AppGroup.defaults.object(forKey: key)
        AppGroup.defaults.removeObject(forKey: key)
        defer {
            if let previous { AppGroup.defaults.set(previous, forKey: key) }
            else { AppGroup.defaults.removeObject(forKey: key) }
            DeviceStatus.clear()
        }
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator())
        await model.refreshSession()
        XCTAssertEqual(model.recoveryToRecord, DemoData.recoveryCode)
        model.finishOnboarding()
        XCTAssertNotNil(model.recoveryToRecord)
        let relaunched = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator())
        await relaunched.refreshSession()
        XCTAssertNotNil(relaunched.recoveryToRecord)
        relaunched.confirmRecoveryRecord()
        XCTAssertNil(relaunched.recoveryToRecord)
        await model.refreshSession()
        XCTAssertNil(model.recoveryToRecord)
    }

    func testFinalGroupMustMatch() {
        XCTAssertFalse(RecoveryRecord.matchesLastGroup(code: DemoData.recoveryCode, entered: ""))
        XCTAssertFalse(RecoveryRecord.matchesLastGroup(code: DemoData.recoveryCode, entered: "AAAA"))
        XCTAssertTrue(RecoveryRecord.matchesLastGroup(code: DemoData.recoveryCode, entered: " ze4b "))
    }
}
