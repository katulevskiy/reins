import XCTest
@testable import Reins

/// The vault on the phone and Settings > Devices (the Android app's VaultFlowTest and DevicesFlowTest, for the parts
/// that are logic): what a form sends, the reference hint, the demo core's vault and devices, and the 403 message.
@MainActor
final class VaultDevicesTests: XCTestCase {
    func testANewItemSendsWhatIsFilledInAndAnEditOnlyWhatChanged() {
        let fields = VaultFormField.fields(.login, apiKey: false)
        XCTAssertEqual(fields.map(\.key), ["username", "password", "uris", "totp"])
        XCTAssertEqual(VaultFormField.fields(.login, apiKey: true).map(\.key), ["password", "uris"])
        let new = VaultFormField.changed(fields, values: ["username": "octo", "password": "pw", "uris": " "], original: nil)
        XCTAssertEqual(new.map(\.key), ["username", "password"])
        // An edit: the password left empty stays as it was; a plain field sent only when it changed.
        let edit = VaultFormField.changed(
            fields, values: ["username": "octo-cat", "password": "", "uris": "https://github.com"],
            original: ["username": "octo", "uris": "https://github.com"]
        )
        XCTAssertEqual(edit.map(\.key), ["username"])
        XCTAssertEqual(edit.first?.value, "octo-cat")
    }

    func testTheHintSaysWhatToWriteOnTheComputer() {
        XCTAssertEqual(VaultText.referenceHint(.login, name: " OpenAI "), "Use it as vault:OpenAI/password")
        XCTAssertEqual(VaultText.referenceHint(.note, name: ""), "Use it as vault:NAME/notes")
        XCTAssertNil(VaultText.referenceHint(.card, name: "Visa"))
        XCTAssertEqual(NewVaultItem.apiKey.newTitle, "New API key")
        XCTAssertEqual(NewVaultItem.login.newTitle, "New login")
    }

    func testTheDemoVaultAndDevicesBehaveLikeTheCore() async throws {
        let core = DemoReinsCore(signedIn: true, modelInstalled: true, syncCap: 0.3)
        let items = try await core.vaultItems(query: "open")
        XCTAssertEqual(items.map(\.name), ["OpenAI"])
        let secret = try await core.vaultReveal(id: "openai", key: "password")
        XCTAssertEqual(secret, "sk-demo-0000")
        let made = try await core.vaultGenerateSshKey(name: "Laptop")
        XCTAssertTrue(made.publicKey.hasPrefix("ssh-ed25519 "))
        let item = try await core.vaultItem(id: made.id)
        XCTAssertEqual(item.kind, .sshKey)
        try await core.signOutDevice(deviceId: "d-old")
        let devices = try await core.devices()
        XCTAssertEqual(devices.map(\.id), ["d-this"])
    }

    func testAPhoneThatIsNotTheApprovalDeviceIsToldWhichOneIs() {
        XCTAssertTrue(DevicesModel.message(CoreError.Server(status: 403, reason: "x")).hasPrefix("Only your approval phone"))
    }
}
