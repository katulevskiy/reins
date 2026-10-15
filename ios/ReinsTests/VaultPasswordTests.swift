import XCTest
@testable import Reins

/// "Generate, don't type": the New login form's password.
final class VaultPasswordTests: XCTestCase {
    func testAGeneratedPasswordIs24CharactersFromItsAlphabet() {
        let password = VaultText.generatedPassword()
        XCTAssertEqual(password.count, 24)
        XCTAssertTrue(password.allSatisfy { VaultText.passwordAlphabet.contains($0) }, password)
    }

    func testTwoGeneratedPasswordsDiffer() {
        XCTAssertNotEqual(VaultText.generatedPassword(), VaultText.generatedPassword())
    }

    func testTheAlphabetHasNoLookAlikes() {
        for c in "O0Il1" { XCTAssertFalse(VaultText.passwordAlphabet.contains(c), "\(c)") }
    }

    func testOnlyAnSshKeyIsDeletedAsAKey() {
        XCTAssertEqual(VaultText.deleteTitle(.sshKey), "Delete this key")
        XCTAssertEqual(VaultText.deleteTitle(.login), "Delete this item")
    }
}
