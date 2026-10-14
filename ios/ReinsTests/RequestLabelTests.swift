import XCTest
@testable import Reins

/// What a request is, the same on every screen (the Android app's `RequestLabels` and `answeredIn`): the plain title,
/// the tile's symbols and logos, the badge's symbols, an entry's "What it asked" words, the tab's count and an entry's
/// approval shown again read-only.
final class RequestLabelTests: XCTestCase {
    private func label(_ provider: String = "terminal", _ action: String = "run", lead: String, subject: String = "", code: Bool = false, tail: String = "") -> RequestLabel {
        RequestLabel(provider: provider, action: action, lead: lead, subject: subject, code: code, tail: tail)
    }

    // MARK: The title

    func testThePlainTitleIsTheLeadTheSubjectAndTheTail() {
        XCTAssertEqual(RequestLabelLook.plain(label(lead: "Run", subject: "git push --force origin main", code: true)), "Run git push --force origin main")
        XCTAssertEqual(RequestLabelLook.plain(label("github", "push", lead: "Push 3 commits to", subject: "acme/web")), "Push 3 commits to acme/web")
        XCTAssertEqual(RequestLabelLook.plain(label("secrets", "secrets", lead: "Use 3 secrets for", subject: "npm run dev", code: true, tail: " and 2 more")),
                       "Use 3 secrets for npm run dev and 2 more")
        XCTAssertEqual(RequestLabelLook.plain(label("desktop", "ask", lead: "Ask you a question")), "Ask you a question", "no subject, no space")
        XCTAssertEqual(RequestLabelLook.plain(label(lead: "", subject: "ls")), "ls", "no lead, no space")
    }

    func testTheTitleKeepsOutWhatCouldReorderIt() {
        XCTAssertEqual(RequestLabelLook.plain(label(lead: "Run", subject: "rm\u{202E} -rf", code: true)), "Run rm -rf")
    }

    // MARK: The tile

    func testEachProviderHasItsSymbolOrItsLogo() {
        XCTAssertEqual(RequestLabelLook.providerSymbol("terminal"), "apple.terminal")
        XCTAssertEqual(RequestLabelLook.providerSymbol("file"), "doc.text")
        XCTAssertEqual(RequestLabelLook.providerSymbol("secrets"), "key.fill")
        XCTAssertEqual(RequestLabelLook.providerSymbol("ssh"), "terminal")
        XCTAssertEqual(RequestLabelLook.providerSymbol("desktop"), "desktopcomputer")
        XCTAssertNil(RequestLabelLook.providerSymbol("github"))

        XCTAssertNil(RequestLabelLook.providerAsset("terminal"), "a symbol, not a logo")
        XCTAssertEqual(RequestLabelLook.providerAsset("github"), "github")
        XCTAssertEqual(RequestLabelLook.providerAsset("gmail"), "gmail")
        XCTAssertEqual(RequestLabelLook.providerAsset("mcp:linear"), "mcp", "every MCP server has the MCP logo")
        XCTAssertNil(RequestLabelLook.providerAsset(""))
    }

    func testEachActionHasItsBadge() {
        let symbols = ["run", "read", "write", "send", "delete", "secrets", "push", "search", "grant", "ask"].map(RequestLabelLook.actionSymbol)
        XCTAssertEqual(symbols, ["play.fill", "eye", "pencil", "paperplane", "trash", "key", "arrow.up", "magnifyingglass", "checkmark.shield", "questionmark"])
        XCTAssertEqual(RequestLabelLook.actionSymbol("something new"), "pencil", "an action this app does not know yet still has a badge")
    }

    func testTheTileReadsAsTheActionAndTheProvider() {
        XCTAssertEqual(RequestLabelLook.tileWords(label("github", "push", lead: "Push")), "Push, GitHub")
        XCTAssertEqual(RequestLabelLook.tileWords(label("ssh", "run", lead: "Sign in to")), "Run, SSH")
        XCTAssertEqual(RequestLabelLook.tileWords(label("mcp:nowhere", "write", lead: "Use")), "Write, MCP server")
    }

    // MARK: What it asked

    func testAnsweredInSaysHowLongTheAnswerTook() {
        XCTAssertEqual(AskedText.answeredIn(askedAt: 1_000, decidedAt: 1_009), " · answered in 9 s")
        XCTAssertEqual(AskedText.answeredIn(askedAt: 1_000, decidedAt: 1_000), " · answered in 0 s")
        XCTAssertEqual(AskedText.answeredIn(askedAt: 1_000, decidedAt: 1_125), " · answered in 2 min")
        XCTAssertEqual(AskedText.answeredIn(askedAt: 1_000, decidedAt: 1_000 + 7_300), " · answered in 2 h")
        XCTAssertEqual(AskedText.answeredIn(askedAt: 0, decidedAt: 1_009), "", "not known when it was asked")
        XCTAssertEqual(AskedText.answeredIn(askedAt: 2_000, decidedAt: 1_009), "", "never a negative time")
    }

    func testWhoIsTheAgentAndItsComputer() {
        XCTAssertEqual(AskedText.who(agent: "Claude Code", connection: "Reins desktop app on laptop", host: "laptop"), "Claude Code · laptop")
        XCTAssertEqual(AskedText.who(agent: "Claude Code", connection: "Work laptop", host: nil), "Claude Code · Work laptop")
        XCTAssertEqual(AskedText.who(agent: "", connection: "Claude", host: "claude.ai"), "Claude · claude.ai")
        XCTAssertEqual(AskedText.who(agent: "Claude", connection: "Claude", host: ""), "Claude", "said once")
        XCTAssertEqual(AskedText.who(agent: "", connection: "Claude", host: nil), "Claude")
    }

    func testTheFullRequestCopiesWithWhatItWasFor() {
        XCTAssertEqual(AskedText.fullText("git push --force origin main \\\n  --no-verify\n\n  "), "git push --force origin main \\\n  --no-verify")
        XCTAssertEqual(AskedText.copyText(full: "git push", purpose: "Update the PR"), "git push\n\nUpdate the PR")
        XCTAssertEqual(AskedText.copyText(full: "git push", purpose: ""), "git push")
        XCTAssertEqual(AskedText.copyText(full: "", purpose: "Update the PR"), "Update the PR")
        XCTAssertEqual(AskedText.purposeLine(who: "Claude Code", purpose: "Update the PR"), "# Claude Code: “Update the PR”")
    }

    // MARK: The tab's count

    func testTheCountIsRoundUpTo99ThenSays99Plus() {
        XCTAssertNil(CountBadge.text(0))
        XCTAssertNil(CountBadge.text(-3))
        XCTAssertEqual(CountBadge.text(7), "7")
        XCTAssertEqual(CountBadge.text(42), "42")
        XCTAssertEqual(CountBadge.text(99), "99")
        XCTAssertEqual(CountBadge.text(100), "99+")
        XCTAssertEqual(CountBadge.text(1_234), "99+")
    }

    // MARK: An entry's approval, read-only

    @MainActor
    func testAnEntrysApprovalIsReadOnlyWithWhatWasSharedTicked() throws {
        let views = DemoData.pending(DemoReinsCore.now()).views
        let view = try XCTUnwrap(views.values.first { !$0.messages.isEmpty })
        let shared: Set<String> = [view.messages[0].id]
        let vm = ApprovalModel(recorded: view, released: shared)
        XCTAssertTrue(vm.readOnly)
        XCTAssertFalse(vm.loading)
        XCTAssertEqual(vm.view?.requestId, view.requestId)
        XCTAssertEqual(vm.draft.selected, shared, "exactly what was shared is ticked")
        XCTAssertNil(vm.draft.allMail)
        XCTAssertFalse(ApprovalModel(requestId: "req1").readOnly)
    }
}
