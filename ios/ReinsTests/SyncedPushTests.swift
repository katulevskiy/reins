import XCTest
@testable import Reins

/// A push decided on another device: its rows from the push kept with the entry, and its changes line by line loaded
/// from GitHub when this iPhone does not have them.
@MainActor
final class SyncedPushTests: XCTestCase {
    private func file(_ path: String, diff: Bool, binary: Bool = false) -> GitFileView {
        GitFileView(
            path: path, status: "modified", additions: 3, deletions: 1, binary: binary,
            diff: diff ? [DiffLineView(kind: .add, oldLine: nil, newLine: 1, text: "let x = 1")] : []
        )
    }

    private func commit(_ files: [GitFileView]) -> GitCommitView {
        GitCommitView(shortSha: "9f3c2a1", subject: "Log in with a passkey", author: "Ada <ada@example.com>", filesChanged: UInt32(files.count), files: files)
    }

    private func label(_ lead: String) -> RequestLabel {
        RequestLabel(provider: "github", action: "push", lead: lead, subject: "octo/app", code: false, tail: "")
    }

    // MARK: Diff not kept

    func testLoadFromGitHubIsOfferedWhenTheChangesWereNotKept() {
        XCTAssertTrue(PushDiffs.missing(commit([file("a.swift", diff: true)]), notKept: true))
    }

    func testLoadFromGitHubIsOfferedWhenATextFileHasNoDiff() {
        XCTAssertTrue(PushDiffs.missing(commit([file("a.swift", diff: true), file("b.swift", diff: false)]), notKept: false))
    }

    func testNothingIsOfferedWhenEveryTextFileHasItsDiff() {
        XCTAssertFalse(PushDiffs.missing(commit([file("a.swift", diff: true), file("logo.png", diff: false, binary: true)]), notKept: false))
    }

    func testALoadThatFailsSaysWhyAndCanBeRetried() async {
        let diffs = PushDiffs(entryId: 99, notKept: true)
        let kept = commit([file("a.swift", diff: false)])
        await diffs.load(kept.shortSha, core: DemoReinsCore(syncCap: 0.3), feedback: NoFeedback.shared)
        XCTAssertNotNil(diffs.failed[kept.shortSha], "the error is shown, with Retry")
        XCTAssertTrue(diffs.missing(kept), "still offered")
        XCTAssertFalse(diffs.loading.contains(kept.shortSha), "the spinner stops")
    }

    // MARK: Rows

    func testARowCountsTheCommitsFromThePush() {
        let git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: [
            DemoData.gitRef("feature/passkeys", change: "create", commitCount: 8, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
        ], defaultBranch: nil)
        let line = AskLabels.line(label("Push to"), field: .push, tense: .done, git: git)
        XCTAssertEqual(line.text, "Pushed 8 commits to octo/app")
        XCTAssertEqual(line.mono, "feature/passkeys · new branch")
    }

    func testARowsBranchLineSaysAForcePushAndATag() {
        let git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: [
            DemoData.gitRef("main", force: true, commitCount: 1, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
            DemoData.gitRef("v1.2.0", kind: "tag", change: "create", commitCount: 0, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
        ], defaultBranch: nil)
        XCTAssertEqual(AskLabels.refsLine(git), "main · force push, v1.2.0 · new tag")
        XCTAssertEqual(AskLabels.line(label("Push 1 commit to"), field: .push, git: git).text, "Push 1 commit to octo/app")
    }

    func testABranchNameIsCleanedAndKeptOnOneLine() {
        let git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: [
            DemoData.gitRef("fix\u{202E}txt.exe\nmore", commitCount: 1, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil),
        ], defaultBranch: nil)
        let mono = AskLabels.refsLine(git) ?? ""
        XCTAssertFalse(mono.contains("\u{202E}"), "no bidi override")
        XCTAssertFalse(mono.contains("\n"), "one line")
    }

    func testWithoutThePushARowKeepsTheCoresWords() {
        XCTAssertEqual(AskLabels.line(label("Push 3 commits to"), field: .push).text, "Push 3 commits to octo/app")
    }
}
