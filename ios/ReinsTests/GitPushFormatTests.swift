import XCTest
@testable import Reins

/// The Android app's GitPushFormatTest, plus the desktop app's lease and SSH wording (NewFeaturesLogicTest).
final class GitPushFormatTests: XCTestCase {
    static func commit(_ n: Int) -> GitCommitView {
        GitCommitView(shortSha: String(format: "%07x", 0xa1b2c00 + n), subject: "Commit number \(n)", author: "Ada Lovelace <ada@example.com>")
    }

    static func file(_ path: String, status: String = "modified", additions: UInt32? = 3, deletions: UInt32? = 1, binary: Bool = false) -> GitFileView {
        GitFileView(path: path, status: status, additions: additions, deletions: deletions, binary: binary)
    }

    /// By default `main` gains 9 commits (7 listed) touching 12 files (10 listed).
    static func ref(
        _ shortName: String = "main", kind: String = "branch", change: String = "update", force: Bool = false, forceUnknown: Bool = false,
        filesChanged: UInt32 = 12, files: [GitFileView] = (1...10).map { file("src/module\($0)/File\($0).kt") },
        additions: UInt64? = 120, deletions: UInt64? = 14
    ) -> GitRefView {
        GitRefView(
            name: kind == "tag" ? "refs/tags/\(shortName)" : "refs/heads/\(shortName)", kind: kind, shortName: shortName, change: change,
            force: force, forceUnknown: forceUnknown, commitCount: 9, commits: (1...7).map(commit), filesChanged: filesChanged,
            files: files, additions: additions, deletions: deletions
        )
    }

    func testEachKindOfRefChangeHasItsOwnChip() {
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref(change: "create")), "New branch")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref(change: "update")), "Update")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref(change: "delete")), "Delete")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref("v1.2", kind: "tag", change: "create")), "New tag")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref("v1.2", kind: "tag", change: "update")), "Tag")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref("v1.2", kind: "tag", change: "delete")), "Delete")
        XCTAssertEqual(GitPushFormat.chipLabel(Self.ref("refs/notes/x", kind: "other", change: "create")), "New")
    }

    func testAForcePushIsToldApartFromOneWhoseHistoryCouldNotBeChecked() {
        XCTAssertEqual(GitPushFormat.history(Self.ref()), .adds)
        XCTAssertEqual(GitPushFormat.history(Self.ref(force: true)), .rewrites)
        XCTAssertEqual(GitPushFormat.history(Self.ref(force: true, forceUnknown: true)), .unknown)
        // The core sets both together; an unknown history alone still warns.
        XCTAssertEqual(GitPushFormat.history(Self.ref(forceUnknown: true)), .unknown)
    }

    func testLineTotalsLeaveOutWhatCouldNotBeCounted() {
        XCTAssertEqual(GitPushFormat.totalsLabel(Self.ref()), "+120 −14 in 12 files")
        XCTAssertEqual(GitPushFormat.totalsLabel(Self.ref(filesChanged: 1, additions: 1, deletions: 0)), "+1 −0 in 1 file")
        XCTAssertEqual(GitPushFormat.totalsLabel(Self.ref(filesChanged: 2, additions: 5, deletions: nil)), "+5 in 2 files")
        XCTAssertEqual(GitPushFormat.totalsLabel(Self.ref(additions: nil, deletions: nil)), "12 files changed")
        XCTAssertNil(GitPushFormat.totalsLabel(Self.ref(filesChanged: 0, files: [])))
    }

    func testFilesShowAStatusLetterAndTheirOwnCounts() {
        XCTAssertEqual(GitPushFormat.fileLetter("added"), "A")
        XCTAssertEqual(GitPushFormat.fileLetter("modified"), "M")
        XCTAssertEqual(GitPushFormat.fileLetter("deleted"), "D")
        XCTAssertEqual(GitPushFormat.fileLetter("type_changed"), "T")
        XCTAssertEqual(GitPushFormat.fileLetter("something new"), "?")
        XCTAssertEqual(GitPushFormat.fileCounts(Self.file("a")), "+3 −1")
        XCTAssertEqual(GitPushFormat.fileCounts(Self.file("logo.png", additions: nil, deletions: nil, binary: true)), "binary")
        XCTAssertEqual(GitPushFormat.fileCounts(Self.file("a", additions: 7, deletions: nil)), "+7")
        XCTAssertEqual(GitPushFormat.fileCounts(Self.file("huge.json", additions: nil, deletions: nil)), "")
    }

    func testPackSizesAreHumanReadable() {
        XCTAssertEqual(GitPushFormat.packSize(0), "0 B")
        XCTAssertEqual(GitPushFormat.packSize(999), "999 B")
        XCTAssertEqual(GitPushFormat.packSize(12_700), "12.4 KB")
        XCTAssertEqual(GitPushFormat.packSize(3_145_728), "3.0 MB")
        XCTAssertEqual(GitPushFormat.packSize(1_610_612_736), "1.5 GB")
    }

    func testLeasesAndSshTargetsReadWell() {
        XCTAssertEqual(leaseLabel(60), "for 1 minute")
        XCTAssertEqual(leaseLabel(1_800), "for 30 minutes")
        XCTAssertEqual(leaseLabel(3_600), "for 1 hour")
        XCTAssertEqual(leaseLabel(5_400), "for 1 h 30 min")
        XCTAssertEqual(leaseLabel(86_400), "for 24 hours")
        XCTAssertEqual(sshTarget(host: "build.example.com", hostKey: "SHA256:x"), "build.example.com")
        XCTAssertEqual(sshTarget(host: nil, hostKey: "SHA256:x"), "SHA256:x")
        XCTAssertEqual(sshTarget(host: "  ", hostKey: "SHA256:x"), "SHA256:x")
        XCTAssertEqual(sshTarget(host: nil, hostKey: nil), "an unknown server")
    }

    func testAServerIsShownByItsHostNeverWithAQuery() {
        XCTAssertEqual(mcpHostLabel("https://mcp.linear.app/mcp"), "mcp.linear.app")
        XCTAssertEqual(mcpHostLabel("https://mcp.notion.com/mcp?key=secret"), "mcp.notion.com")
        XCTAssertEqual(mcpHostLabel("http://localhost:8080/mcp"), "localhost:8080")
        XCTAssertEqual(mcpHostLabel("not a url"), "not a url")
    }
}
