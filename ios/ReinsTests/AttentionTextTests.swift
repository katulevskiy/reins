import XCTest
@testable import Reins

/// The Android app's AttentionTest, case for case: the summary on a folded row, the order flagged rows take, the one
/// chip on an email, and what the sheet starts ticked.
final class AttentionTextTests: XCTestCase {
    private static func attention(_ level: AttentionLevel, _ reasons: (ReasonCode, String)...) -> Attention {
        Attention(level: level, reasons: reasons.map { AttentionReason(code: $0.0, label: $0.1) })
    }

    private static let ci = attention(.close, (.changesCi, "Changes CI"))
    private static let deletes = attention(.look, (.deletesFile, "Deletes a file"))

    private static func commit(_ n: Int, _ attention: Attention? = nil) -> GitCommitView {
        GitCommitView(shortSha: "c\(n)", subject: "Commit \(n)", author: "Ada <ada@example.com>", attention: attention)
    }

    private static func ref(count: UInt32, commits: [GitCommitView], force: Bool = false, forceUnknown: Bool = false) -> GitRefView {
        GitRefView(
            name: "refs/heads/main", kind: "branch", shortName: "main", change: "update", force: force, forceUnknown: forceUnknown,
            commitCount: count, commits: commits, filesChanged: 0, files: [], additions: nil, deletions: nil
        )
    }

    private static func mail(_ id: String, sensitive: Bool = false, _ attention: Attention? = nil) -> MessageView {
        MessageView(id: id, from: "a@b.com", subject: "Subject \(id)", date: 0, snippet: "", coveredByGrant: false, sensitive: sensitive, attention: attention)
    }

    // MARK: The summary

    func testTheSummaryCountsFlaggedItemsAndSaysWhenNotAllAreListed() {
        XCTAssertEqual(AttentionText.summary(flagged: 2, listed: 8, total: 8), "2 of 8 to look at")
        XCTAssertEqual(AttentionText.summary(flagged: 2, listed: 50, total: 200), "2 of 50 shown to look at")
        XCTAssertNil(AttentionText.summary(flagged: 0, listed: 8, total: 8), "nothing flagged, no line")

        let commits = (0..<8).map { Self.commit($0, $0 == 2 ? Self.deletes : $0 == 5 ? Self.ci : nil) }
        XCTAssertEqual(AttentionText.summary(Self.ref(count: 8, commits: commits)), "2 of 8 to look at")
        XCTAssertEqual(AttentionText.summary(Self.ref(count: 9, commits: commits)), "2 of 8 shown to look at")
    }

    func testAHistoryRewriteIsSaidEvenWithNothingFlagged() {
        let plain = (0..<3).map { Self.commit($0) }
        XCTAssertNil(AttentionText.summary(Self.ref(count: 3, commits: plain, force: true)))
        XCTAssertTrue(AttentionText.rewrites(Self.ref(count: 3, commits: plain, force: true)))
        XCTAssertTrue(AttentionText.rewrites(Self.ref(count: 3, commits: plain, forceUnknown: true)))
        XCTAssertFalse(AttentionText.rewrites(Self.ref(count: 3, commits: plain)))
    }

    func testTheEmailHeaderSaysHowManyToLookAt() {
        let messages = (0..<12).map { Self.mail("m\($0)", $0 == 3 ? Self.attention(.close, (.passwordReset, "Password reset")) : $0 == 8 ? Self.attention(.look, (.finance, "Bank or invoice")) : nil) }
        XCTAssertEqual(AttentionText.mailHeader(messages, service: "gmail"), "12 found · 2 to look at")
        XCTAssertEqual(AttentionText.mailHeader(messages, service: "gmail", shared: 10), "12 found · 10 shared · 2 to look at")
        XCTAssertEqual(AttentionText.mailHeader([Self.mail("a")], service: "gmail"), "1 found")
    }

    // MARK: The order

    func testFlaggedRowsComeFirstAndKeepTheirOriginalIndex() {
        let commits = (0..<8).map { Self.commit($0, $0 == 2 ? Self.deletes : $0 == 5 ? Self.ci : nil) }
        let rows = AttentionText.ordered(commits) { $0.attention?.level }
        // Close, then Look, then the rest in their order.
        XCTAssertEqual(rows.map(\.offset), [5, 2, 0, 1, 3, 4, 6, 7])
        // Each row opens the commit at its own index in the push, which is not reordered.
        for row in rows { XCTAssertEqual(commits[row.offset], row.element) }
        XCTAssertEqual(commits.map(\.shortSha), (0..<8).map { "c\($0)" })
    }

    func testTheOrderIsStableWithinABand() {
        let commits = [Self.commit(0, Self.deletes), Self.commit(1, Self.ci), Self.commit(2, Self.deletes), Self.commit(3, Self.ci)]
        XCTAssertEqual(AttentionText.ordered(commits) { $0.attention?.level }.map(\.offset), [1, 3, 0, 2])
    }

    // MARK: The chip

    func testTheChipJoinsTheReasonsAndVoiceOverSaysHowClosely() {
        let mark = AttentionText.mark(Self.attention(.close, (.changesCi, "Changes CI"), (.deletesLines, "Deletes 1,240 lines")))
        XCTAssertEqual(mark?.text, "Changes CI · Deletes 1,240 lines")
        XCTAssertEqual(mark?.spoken, "Look closer: Changes CI, Deletes 1,240 lines")
        XCTAssertEqual(AttentionText.mark(Self.deletes)?.spoken, "Worth a look: Deletes a file")
        XCTAssertNil(AttentionText.mark(nil))
        // The commit page keeps its own "Hidden characters" tag, so its chip leaves that reason out.
        let hidden = Self.attention(.close, (.hiddenChars, "Hidden characters"))
        XCTAssertNil(AttentionText.mark(hidden, shown: [.hiddenChars]))
    }

    func testHiddenCharactersStayShownWhenHeavierReasonsFillTheChip() {
        // The core keeps three reasons: a secret file, a key and CI outweigh hidden characters, which drop out.
        let full = Self.attention(.close, (.secretFile, "Secret file"), (.looksLikeKey, "Looks like a key"), (.changesCi, "Changes CI"))
        XCTAssertTrue(AttentionText.hiddenApart(true, full), "the row keeps its own Hidden tag")
        XCTAssertFalse(AttentionText.mark(full)?.labels.contains("Hidden characters") ?? true)
        // When the chip names them, one tag is enough; without hidden characters, none.
        XCTAssertFalse(AttentionText.hiddenApart(true, Self.attention(.close, (.changesCi, "Changes CI"), (.hiddenChars, "Hidden characters"))))
        XCTAssertTrue(AttentionText.hiddenApart(true, nil))
        XCTAssertFalse(AttentionText.hiddenApart(false, full))
    }

    func testACodeRowHasOneChip() {
        let code = Self.mail("c", sensitive: true, Self.attention(.close, (.signInCode, "Looks like a sign-in code")))
        XCTAssertEqual(AttentionText.mail(code, service: "gmail")?.labels, ["Code"])
        let both = Self.mail("b", sensitive: true, Self.attention(.close, (.signInCode, "Looks like a sign-in code"), (.passwordReset, "Password reset")))
        XCTAssertEqual(AttentionText.mail(both, service: "gmail")?.text, "Code · Password reset")
        let vault = Self.mail("v", sensitive: true, Self.attention(.close, (.password, "Password")))
        XCTAssertEqual(AttentionText.mail(vault, service: "vault")?.labels, ["Password"])
        // A sensitive message from an older core, with no reasons, keeps its chip.
        XCTAssertEqual(AttentionText.mail(Self.mail("o", sensitive: true), service: "telegram")?.labels, ["Code"])
        XCTAssertNil(AttentionText.mail(Self.mail("p"), service: "gmail"))
    }

    // MARK: Ticks

    func testAFinanceEmailStaysTickedAndASensitiveOneDoesNot() {
        let view = ApprovalLogicTests.view(kind: .search, messages: [
            Self.mail("bank", Self.attention(.look, (.finance, "Bank or invoice"))),
            Self.mail("reset", Self.attention(.close, (.passwordReset, "Password reset"))),
            Self.mail("code", sensitive: true, Self.attention(.close, (.signInCode, "Looks like a sign-in code"))),
            Self.mail("plain"),
        ], count: 4)
        XCTAssertEqual(ApprovalDraft.initial(for: view).selected, ["bank", "reset", "plain"])
    }
}
