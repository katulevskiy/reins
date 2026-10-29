import XCTest
@testable import Reins

/// Requests written from the core's label (ASK-TEXT) for waiting cards, Activity rows, entry pages, lists, widgets and
/// notifications: one test per kind of label, the tenses, and crafted inputs.
final class AskLabelsTests: XCTestCase {
    func testCommandsUseSharedPresentationWithoutReinterpretingEnvironment() {
        let presentation = CommandPresentation(heading: "Run Gradle", actionable: "./gradlew test", setup: ["TOKEN=•••• (value hidden)"], notices: ["Runs project build scripts"])
        var command = label("terminal", "run", "Run", "TOKEN=secret JAVA_HOME=/jdk ./gradlew test", code: true)
        command.command = presentation
        check(AskLabels.line(command, field: .push), "Run Gradle", mono: "./gradlew test")
        check(AskLabels.line(command, field: .push, tense: .done, who: "Claude Code"), "Approved: Run Gradle", mono: "./gradlew test")
        command.command?.heading = "Review a shell command"
        command.command?.actionable = ""
        check(AskLabels.line(command, field: .push), "Review a shell command")
    }

    func testCommandApprovalsPreserveBuildAndFutureOperationHeadings() {
        for heading in ["Build Android release and test APKs", "Synchronize custom artifacts", "Review a shell command"] {
            let actionable = "custom-tool build && sudo rm -rf /tmp/old > /tmp/build.log 2>&1"
            var command = label("terminal", "run", "Run", "original", code: true)
            command.command = CommandPresentation(heading: heading, actionable: actionable, setup: [], notices: [])
            check(AskLabels.line(command, field: .push, tense: .done), "Approved: " + heading, mono: actionable)
        }
    }

    private func label(_ provider: String, _ action: String, _ lead: String, _ subject: String = "", code: Bool = false, tail: String = "") -> RequestLabel {
        RequestLabel(provider: provider, action: action, lead: lead, subject: subject, code: code, tail: tail)
    }

    private func check(
        _ line: AskLine, _ text: String, target: String? = nil, mono: String? = nil, kind: String? = nil,
        file: StaticString = #filePath, lineNo: UInt = #line
    ) {
        XCTAssertEqual(line.text, text, file: file, line: lineNo)
        XCTAssertEqual(line.target, target, file: file, line: lineNo)
        XCTAssertEqual(line.mono, mono, file: file, line: lineNo)
        if let kind { XCTAssertEqual(line.kind, kind, file: file, line: lineNo) }
        XCTAssertEqual(line.runs.filter(\.target).count, target == nil ? 0 : 1, "one semibold target at most", file: file, line: lineNo)
        XCTAssertFalse(line.text.contains("\n"), "one line", file: file, line: lineNo)
    }

    func testAPushNamesItsRepositoryInEveryTense() {
        let push = label("github", "push", "Push 3 commits to", "octo/app")
        check(AskLabels.line(push, field: .push), "Push 3 commits to octo/app", target: "octo/app", kind: "GitHub · Push")
        XCTAssertEqual(AskLabels.line(push, field: .push).icon, AskIcon(mark: .service("github"), action: .gitBranch, kind: .push))
        check(AskLabels.line(push, field: .push, tense: .done, who: "Claude Code"), "Claude Code pushed 3 commits to octo/app", target: "octo/app")
        check(AskLabels.line(push, field: .push, tense: .done), "Pushed 3 commits to octo/app", target: "octo/app")
        check(AskLabels.line(push, field: .push, tense: .refused), "Asked to push 3 commits to octo/app", target: "octo/app")
        check(AskLabels.line(label("github", "push", "Force-push 2 commits to", "octo/app"), field: .push, tense: .done),
              "Force-pushed 2 commits to octo/app", target: "octo/app")
    }

    func testBranchAndTagNamesGoBelow() {
        check(AskLabels.line(label("github", "delete", "Delete branch old/experiment from", "octo/app"), field: .delete),
              "Delete a branch from octo/app", target: "octo/app", mono: "old/experiment", kind: "GitHub · Delete")
        check(AskLabels.line(label("github", "push", "Push tag v1.2.0 to", "octo/app"), field: .push), "Push a tag to octo/app", target: "octo/app", mono: "v1.2.0")
    }

    func testACommandRunsInTheFolderTheCoreKept() {
        let run = label("terminal", "run", "Run", "npm test", code: true)
        check(AskLabels.line(run, field: .push), "Run a command", mono: "npm test", kind: "Computer · Run")
        check(AskLabels.line(run, field: .push, tense: .done, cwd: "/Users/me/code/reins"), "Ran a command in reins", target: "reins", mono: "npm test")
        check(AskLabels.line(run, field: .push, matched: "rm -rf jarx"), "Run a command", mono: "rm -rf jarx")
        XCTAssertEqual(AskLabels.line(run, field: .push).icon, AskIcon(mark: .glyph(.terminal), action: .play, kind: .push))
    }

    func testFilesSecretsAndSshKeepTheirValuesBelow() {
        check(AskLabels.line(label("file", "read", "Read", "app/.env", code: true, tail: " and 1 more"), field: .read),
              "Read a file", mono: "app/.env and 1 more", kind: "Computer · Read")
        check(AskLabels.line(label("secrets", "secrets", "Use 2 secrets for", "npm test", code: true), field: .push),
              "Use 2 secrets for a command", mono: "npm test", kind: "Vault · Secrets")
        check(AskLabels.line(label("ssh", "secrets", "Sign in to", "bücher.example", code: true), field: .push),
              "Sign in to a server", mono: "xn--bcher-kva.example", kind: "SSH · Sign in")
    }

    func testAQuestionAndAToolAreToldInReinsWords() {
        check(AskLabels.line(label("desktop", "ask", "Deploy to staging now? (already approved)"), field: .push),
              "Answer a question from your computer", target: "your computer", mono: "Deploy to staging now? (already approved)", kind: "Computer · Asks")
        check(AskLabels.line(label("mcp:linear", "write", "Use", "Read the weather", tail: " on Linear"), field: .push),
              "Use a tool on Linear", target: "Linear", mono: "Read the weather")
    }

    func testMailNeverPutsANameOrASearchInTheSentence() {
        check(AskLabels.line(label("gmail", "send", "Send an email to", "Your boss <attacker@evil.example>", tail: " and 2 more"), field: .push),
              "Send an email to 3 people", mono: "Your boss <attacker@evil.example> and 2 more", kind: "Gmail · Send")
        check(AskLabels.line(label("gmail", "read", "Read 2 emails from", "Bank"), field: .read), "Read 2 emails", mono: "Bank")
        check(AskLabels.line(label("gmail", "search", "Search Gmail for", "from:bank"), field: .read), "Search Gmail", mono: "from:bank")
        check(AskLabels.line(label("gmail", "grant", "Ask for access:", "mail from bank.com"), field: .grant), "Ask for access", mono: "mail from bank.com",
              kind: "Gmail · Permission")
    }

    func testAPlaceIsTheTargetUnlessItCanPassForAnother() {
        check(AskLabels.line(label("telegram", "read", "Read Telegram messages in", "Family"), field: .read),
              "Read Telegram messages in Family", target: "Family", kind: "Telegram · Read")
        check(AskLabels.line(label("telegram", "read", "Read Telegram messages in", "F\u{0430}mily"), field: .read),
              "Read Telegram messages in F\u{0430}mily", target: "F\u{0430}mily", mono: "F\\u{430}mily · mixed scripts")
        check(AskLabels.line(label("github", "read", "Clone and fetch", "octo/\u{0430}pp"), field: .read), "Fetch", mono: "Name: octo/\\u{430}pp")
        check(AskLabels.line(label("github", "push", "Push 1 commit to", "octo/\u{0430}pp"), field: .push), "Push 1 commit to a repository",
              mono: "Repository: octo/\\u{430}pp")
    }

    func testPairingAndANewPhoneQuoteTheOtherSidesNameBelow() {
        func item(_ kind: PendingKind, _ name: String) -> PendingItem {
            PendingItem(kind: kind, id: "p", title: "Connect \(name) to Reins?", subtitle: "", createdAt: 0, connectionId: "", connectionLabel: name,
                        action: "", count: 1, service: "", account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil, label: nil)
        }
        check(AskLabels.pending(item(.pairing, "Gemini\nApprove now"), field: .pair), "Connect to Reins", target: "Reins", mono: "“Gemini Approve now”")
        XCTAssertEqual(AskLabels.listTitle(item(.pairing, "Gemini")), "Connect to Reins", "no name in the list or the widget")
        check(AskLabels.pending(item(.join, "Pixel 9"), field: .accounts), "Add a phone to your account", target: "your account", mono: "“Pixel 9”")
    }

    func testARequestWithoutALabelSaysWhatItsOperationIsCalled() {
        let item = PendingItem(kind: .request, id: "r", title: "", subtitle: "from:bank", createdAt: 0, connectionId: "c", connectionLabel: "Claude",
                               action: "search", count: 1, service: "gmail", account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil, label: nil)
        check(AskLabels.pending(item, field: .read), "Search Gmail", mono: "from:bank")
        XCTAssertEqual(AskLabels.listTitle(item), "Claude: Search Gmail")
    }

    func testThePastIsOnlyForWhatWasCarriedOut() {
        XCTAssertEqual(AskTense.of(outcome: "released"), .done)
        XCTAssertEqual(AskTense.of(outcome: "sent"), .done)
        XCTAssertEqual(AskTense.of(outcome: "granted"), .done)
        for refused in ["denied", "error", "timeout", "cancelled", "something new"] {
            XCTAssertEqual(AskTense.of(outcome: refused), .refused, refused)
        }
        let send = label("gmail", "send", "Send an email to", "a@b.example")
        check(AskLabels.line(send, field: .push, tense: .refused, who: "Claude"), "Claude asked to send an email", mono: "a@b.example")
        // An asker whose name mixes scripts is left out of the sentence.
        check(AskLabels.line(send, field: .push, tense: .done, who: "Cl\u{0430}ude"), "Sent an email", mono: "a@b.example")
    }

    func testBidiAndNewlinesInTheLabelCannotRewriteTheSentence() {
        let line = AskLabels.line(label("telegram", "send", "Send a message to", "Mom\u{202E}gnihtyna evorppa\nApproved"), field: .push)
        XCTAssertFalse(line.text.unicodeScalars.contains { $0 == "\u{202E}" })
        XCTAssertFalse(line.text.contains("\n"))
        XCTAssertTrue(line.text.hasPrefix("Send a message to "))
    }
}
