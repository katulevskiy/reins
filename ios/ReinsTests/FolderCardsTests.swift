import XCTest
@testable import Reins

/// "Trust this folder?": the folders paired computers propose, read with what waits; Trust asks the owner, Not now puts
/// the card away, and a computer's Folders list the trusted and the suggested ones, removing at once.
@MainActor
final class FolderCardsTests: XCTestCase {
    private let mac = "c6"
    private let linux = "c7"

    private func app(_ owner: SetupOwnerAnswers = SetupOwnerAnswers()) async -> (DemoReinsCore, AppModel) {
        let core = DemoReinsCore(signedIn: true, syncCap: 0.3, folders: true)
        let model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: owner, demo: true)
        await model.refreshSession()
        await model.refreshPending()
        return (core, model)
    }

    func testTrustAsksTheOwnerAndConfirms() async throws {
        let owner = SetupOwnerAnswers()
        let (core, model) = await app(owner)
        // Newest computer first; the Mac's trusted folder is no card.
        XCTAssertEqual(model.folderQuestions.map(\.path), ["/Users/dan/Documents/dev/*", "/Users/dan/Documents/dev/tmp", "/home/dan/src/*"])
        XCTAssertEqual(model.folderQuestions.first?.computer, DemoData.desktop)

        let failed = await model.trustFolder(connectionId: mac, path: "/Users/dan/Documents/dev/*", kind: .root)
        XCTAssertNil(failed)
        XCTAssertEqual(owner.asked, ["Trust /Users/dan/Documents/dev/* as projects"])
        XCTAssertEqual(model.folderQuestions.map(\.path), ["/Users/dan/Documents/dev/tmp", "/home/dan/src/*"])
        let folders = try await core.computerFolders(connectionId: mac)
        XCTAssertTrue(folders.confirmed.contains(FolderView(path: "/Users/dan/Documents/dev/*", kind: .root)))
        XCTAssertFalse(folders.suggested.contains { $0.path == "/Users/dan/Documents/dev/*" })

        // Refused: nothing is trusted and the card stays.
        let refusing = SetupOwnerAnswers()
        refusing.answer = false
        let (otherCore, other) = await app(refusing)
        let refused = await other.trustFolder(connectionId: linux, path: "/home/dan/src/*", kind: .root)
        XCTAssertNotNil(refused)
        XCTAssertEqual(refusing.asked, ["Trust /home/dan/src/* as projects"])
        XCTAssertTrue(other.folderQuestions.contains { $0.connectionId == linux && $0.path == "/home/dan/src/*" })
        let still = try await otherCore.computerFolders(connectionId: linux)
        XCTAssertTrue(still.confirmed.isEmpty)

        // A folder the computer no longer proposes cannot be trusted.
        let gone = await model.trustFolder(connectionId: linux, path: "/etc", kind: .root)
        XCTAssertNotNil(gone)
    }

    func testNotNowDismisses() async throws {
        let owner = SetupOwnerAnswers()
        let (core, model) = await app(owner)
        let dismissed = await model.dismissFolder(connectionId: mac, path: "/Users/dan/Documents/dev/tmp", kind: .scratch)
        XCTAssertNil(dismissed)
        XCTAssertTrue(owner.asked.isEmpty, "putting one away needs nothing")
        XCTAssertFalse(model.folderQuestions.contains { $0.path == "/Users/dan/Documents/dev/tmp" })
        // It stays suggested on the computer's page, untrusted, and is not asked about again.
        let folders = try await core.computerFolders(connectionId: mac)
        XCTAssertTrue(folders.suggested.contains(FolderView(path: "/Users/dan/Documents/dev/tmp", kind: .scratch)))
        XCTAssertFalse(folders.confirmed.contains { $0.path == "/Users/dan/Documents/dev/tmp" })
        await model.refreshPending()
        XCTAssertFalse(model.folderQuestions.contains { $0.path == "/Users/dan/Documents/dev/tmp" })
    }

    func testFolderSectionListsConfirmedAndSuggested() async throws {
        let owner = SetupOwnerAnswers()
        let (_, model) = await app(owner)
        let page = ComputerFoldersModel(connectionId: mac)
        XCTAssertNil(page.folders)
        await page.load(model)
        XCTAssertEqual(page.trusted, [FolderView(path: "/Users/dan/Documents/dev/reins", kind: .root)])
        XCTAssertEqual(page.suggested, [
            FolderView(path: "/Users/dan/Documents/dev/*", kind: .root),
            FolderView(path: "/Users/dan/Documents/dev/tmp", kind: .scratch),
        ])

        // A dismissed one can still be trusted here, with the owner's check.
        _ = await model.dismissFolder(connectionId: mac, path: "/Users/dan/Documents/dev/tmp", kind: .scratch)
        await page.load(model)
        XCTAssertTrue(page.suggested.contains(FolderView(path: "/Users/dan/Documents/dev/tmp", kind: .scratch)))
        await page.trust(FolderView(path: "/Users/dan/Documents/dev/tmp", kind: .scratch), model)
        XCTAssertNil(page.error)
        XCTAssertEqual(owner.asked, ["Trust /Users/dan/Documents/dev/tmp as scratch"])
        XCTAssertTrue(page.trusted.contains(FolderView(path: "/Users/dan/Documents/dev/tmp", kind: .scratch)))
        XCTAssertFalse(page.suggested.contains { $0.path == "/Users/dan/Documents/dev/tmp" })

        // A computer that proposes nothing lists nothing.
        let empty = ComputerFoldersModel(connectionId: "c1")
        await empty.load(model)
        XCTAssertNotNil(empty.folders)
        XCTAssertTrue(empty.trusted.isEmpty && empty.suggested.isEmpty)
    }

    func testRemoveAppliesAtOnce() async throws {
        let owner = SetupOwnerAnswers()
        owner.answer = false
        let (core, model) = await app(owner)
        let page = ComputerFoldersModel(connectionId: mac)
        await page.load(model)
        let reins = FolderView(path: "/Users/dan/Documents/dev/reins", kind: .root)
        await page.remove(reins, model)
        // It only tightens: no owner check, even with one that would refuse.
        XCTAssertNil(page.error)
        XCTAssertTrue(owner.asked.isEmpty)
        XCTAssertFalse(page.trusted.contains(reins))
        let folders = try await core.computerFolders(connectionId: mac)
        XCTAssertFalse(folders.confirmed.contains(reins))
        // Still proposed, so suggested again, but not a card again.
        XCTAssertTrue(folders.suggested.contains(reins))
        XCTAssertFalse(model.folderQuestions.contains { $0.path == reins.path })

        // Removing what is not trusted says so.
        let again = await model.removeFolder(connectionId: mac, path: reins.path, kind: .root)
        XCTAssertNotNil(again)
    }

    func testFolderText() {
        func q(_ computer: String, _ path: String, _ kind: FolderKindChoice = .root) -> FolderQuestion {
            FolderQuestion(connectionId: "c6", computer: computer, path: path, kind: kind, at: 0)
        }
        XCTAssertEqual(FolderText.kindName(.root), "Projects")
        XCTAssertEqual(FolderText.kindName(.scratch), "Scratch")
        XCTAssertEqual(FolderText.from(q("M2A", "/a")), "M2A · Projects")
        XCTAssertEqual(FolderText.from(q("M2A", "/a", .scratch)), "M2A · Scratch")
        XCTAssertEqual(FolderText.from(q(" ", "/a")), "Projects")
        // From the computer: one short line for its name, nothing hidden or reordering.
        XCTAssertEqual(FolderText.from(q("M\u{202E}2A\nbox", "/a")), "M2A box · Projects")
        // The path whole: never cut, only cleaned.
        let long = "/Users/dan/" + String(repeating: "very-long-folder-name/", count: 10) + "*"
        XCTAssertEqual(FolderText.path(long), long)
        XCTAssertEqual(FolderText.path("/Users/\u{202E}dan/*"), "/Users/dan/*")
        XCTAssertNotEqual(FolderText.id(q("M2A", "/a")), FolderText.id(q("M2A", "/a", .scratch)), "a kind is its own card")
    }
}
