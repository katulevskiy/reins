import XCTest
@testable import Reins

/// How each kind of request is written (design/signal/ASK-TEXT.md): the kind label, a verb-first sentence with exactly
/// one italic run (the target), machine details only on the mono line, and the icon's mark and action. One test per
/// kind.
final class AskFormatTests: XCTestCase {
    private typealias T = ApprovalLogicTests

    /// The sentence, its target, its mono line; and the rules every ask keeps.
    private func check(
        _ line: AskLine, _ text: String, target: String?, mono: String? = nil, kind: String? = nil,
        icon: RequestIcon.Spec? = nil, file: StaticString = #filePath, lineNo: UInt = #line
    ) {
        XCTAssertEqual(line.text, text, file: file, line: lineNo)
        XCTAssertEqual(line.target, target, file: file, line: lineNo)
        XCTAssertEqual(line.mono, mono, file: file, line: lineNo)
        if let kind { XCTAssertEqual(line.kind, kind, file: file, line: lineNo) }
        if let icon { XCTAssertEqual(line.icon, icon, file: file, line: lineNo) }
        XCTAssertLessThanOrEqual(line.runs.filter(\.target).count, 1, "one italic run at most", file: file, line: lineNo)
        XCTAssertEqual(line.runs.filter(\.target).count, target == nil ? 0 : 1, file: file, line: lineNo)
        XCTAssertFalse(line.text.hasPrefix("Wants"), "verb first: the asker is in the top bar", file: file, line: lineNo)
    }

    private func ref(_ name: String, kind: String = "branch", change: String = "update", force: Bool = false, commits: UInt32 = 3) -> GitRefView {
        GitRefView(
            name: "refs/heads/\(name)", kind: kind, shortName: name, change: change, force: force, forceUnknown: false,
            commitCount: commits, commits: [], filesChanged: 0, files: [], additions: nil, deletions: nil
        )
    }

    private func push(_ refs: [GitRefView]) -> ApprovalView {
        var v = T.view("g", kind: .write, service: "github", op: "git_push", opTitle: "Push with git", action: "write")
        v.git = GitPushView(repo: "octo/app", packBytes: 0, notes: [], refs: refs, defaultBranch: "main")
        return v
    }

    private func ask(question: String, detail: String?, topic: String? = nil, matched: String? = nil, destructive: Bool = false) -> ApprovalView {
        var v = T.view("q", kind: .write, service: "", op: "", action: "write")
        let rule = matched.map { AskRuleView(label: "Deletes files", pattern: "rm *", matched: $0, group: nil, destructive: destructive, reported: false) }
        v.ask = AskView(question: question, detail: detail, topic: topic, rule: rule, uncertain: false, destructive: destructive)
        return v
    }

    func testAPushNamesItsRepositoryAndPutsTheBranchInMono() {
        let line = AskFormat.waiting(push([ref("feature/passkeys", change: "create", commits: 8)]))
        check(line, "Push 8 commits to octo/app", target: "octo/app", mono: "feature/passkeys · new branch", kind: "GitHub · Push",
              icon: RequestIcon.Spec(mark: .service("github"), action: .gitBranch, kind: .push))
        XCTAssertEqual(line.runs.filter(\.target).map(\.text), ["octo/\u{2060}app"], "a slash never ends a line")
        XCTAssertFalse(line.text.contains("feature/passkeys"), "a branch never goes into the serif")
        check(AskFormat.waiting(push([ref("main", force: true, commits: 2)])), "Push 2 commits to octo/app", target: "octo/app", mono: "main · force")
    }

    func testADeletedBranchIsDeletedFromItsRepository() {
        check(AskFormat.waiting(push([ref("old-experiment", change: "delete", commits: 0)])), "Delete a branch of octo/app", target: "octo/app",
              mono: "old-experiment · delete", icon: RequestIcon.Spec(mark: .service("github"), action: .trash, kind: .push))
    }

    func testTagsArePushedToTheRepository() {
        check(AskFormat.waiting(push([ref("v1.2.0", kind: "tag", change: "create", commits: 0)])), "Push a tag to octo/app", target: "octo/app",
              mono: "v1.2.0 · new tag")
    }

    /// The core's label for a question from the computer, as the pending list carries it.
    private func core(_ provider: String, _ action: String, _ subject: String, tail: String = "") -> RequestLabel {
        RequestLabel(provider: provider, action: action, lead: "Run", subject: subject, code: true, tail: tail)
    }

    func testACommandRunsOnTheComputerWithWhatTheCoreMatchedInMono() {
        let v = ask(question: "Claude Code wants to run: rm -rf jarx", detail: "Command:\nrm -rf jarx\nnpm test\n\nIn: /Users/me/code/reins",
                    matched: "rm -rf jarx", destructive: true)
        let line = AskFormat.waiting(v, core: core("terminal", "delete", "rm -rf jarx"), computer: "laptop")
        check(line, "Run a command on laptop", target: "laptop", mono: "rm -rf jarx", kind: "Computer · Run",
              icon: RequestIcon.Spec(mark: .glyph(.terminal), action: .trash, kind: .delete))
        XCTAssertFalse(line.text.contains("rm -rf"), "a command never goes into the serif")
        // No guard match: the core's own reading of the command.
        let plain = ask(question: "q", detail: "Command:\nnpm test")
        check(AskFormat.waiting(plain, core: core("terminal", "run", "npm test", tail: " and 1 more")), "Run a command on your computer",
              target: "your computer", mono: "npm test and 1 more", icon: RequestIcon.Spec(mark: .glyph(.terminal), action: .play, kind: .push))
    }

    func testAFileQuestionSaysWhatTheCoreReadAndShowsThePath() {
        let v = ask(question: "Claude Code wants to read .env", detail: "Files:\napp/.env")
        check(AskFormat.waiting(v, core: core("file", "read", "app/.env", tail: " and 1 more"), computer: "laptop"), "Read a file on laptop",
              target: "laptop", mono: "app/.env and 1 more", kind: "Computer · Files", icon: RequestIcon.Spec(mark: .glyph(.file), action: .eye, kind: .push))
        check(AskFormat.waiting(v, core: core("file", "write", "app/.env")), "Change a file on your computer", target: "your computer", mono: "app/.env")
    }

    func testWithoutTheCoresReadingAQuestionIsGeneric() {
        // Nothing resolved by the core (or a question in its own words): the question goes below, never into the sentence.
        let v = ask(question: "Claude Code wants to run: rm -rf ~", detail: "Command:\nrm -rf ~")
        check(AskFormat.waiting(v, computer: "laptop"), "Answer a question from laptop", target: "laptop",
              mono: "Claude Code wants to run: rm -rf ~\nCommand:\nrm -rf ~", kind: "Computer · Asks",
              icon: RequestIcon.Spec(mark: .glyph(.help), action: .help, kind: .push))
        check(AskFormat.waiting(ask(question: "Deploy to staging now?", detail: nil), core: RequestLabel(
            provider: "desktop", action: "ask", lead: "Deploy to staging now?", subject: "", code: false, tail: ""
        )), "Answer a question from your computer", target: "your computer", mono: "Deploy to staging now?")
    }

    func testAToolNamesItsServerAndShowsTheToolInMono() {
        var v = T.view("m", kind: .write, service: "mcp:linear", op: "create_issue", action: "write")
        v.mcp = McpCallView(serverName: "Linear", serverUrl: "https://mcp.linear.app", tool: "create_issue", title: "Create issue",
                            description: "", argumentsJson: "{}", readOnly: false, destructive: false)
        check(AskFormat.waiting(v), "Use a tool on Linear", target: "Linear", mono: "create_issue", kind: "Linear · MCP tool",
              icon: RequestIcon.Spec(mark: .service("mcp"), action: .pencil, kind: .push))
    }

    func testAReleaseNamesTheEnvironmentAndShowsTheCommand() {
        var v = T.view("e", kind: .write, service: "vault", op: "env_release", action: "write")
        v.envRelease = EnvReleaseView(
            project: "web", env: "dev", runner: "agent:claude-code", runnerLabel: "Claude Code", agent: true, root: "~/code/web",
            command: "npm run dev -- --port 3000", program: "/usr/local/bin/npm", programWarning: nil, variables: ["DATABASE_URL", "STRIPE_KEY"],
            allVariables: false, leaseSecs: 3_600, purpose: nil, commands: [], print: false, alwaysAsk: false, alwaysAskSet: false
        )
        check(AskFormat.waiting(v), "Release 2 variables of web · dev", target: "web · dev", mono: "npm run dev -- --port 3000", kind: "Vault · Secrets",
              icon: RequestIcon.Spec(mark: .glyph(.lock), action: .key, kind: .push))
    }

    func testSecretsSavedInTheVault() {
        var v = T.view("s", kind: .write, service: "vault", op: "secrets_store", noStanding: true, action: "write")
        v.secretsStore = SecretsStoreView(
            envs: [EnvStoreView(project: "web", env: "staging", newEnv: true, added: ["A"], replaced: [], removed: [], renamed: [], copiedFrom: nil, copied: 0)],
            check: "4821", total: 6, ssh: [], tokens: []
        )
        check(AskFormat.waiting(v), "Save 6 secrets in your vault", target: "your vault", mono: "web · staging", kind: "Vault · Save")
    }

    func testAnSshSignInNamesTheKeyAndShowsTheServer() {
        var v = T.view("h", kind: .write, service: "ssh", op: "ssh_sign", action: "write")
        v.ssh = SshSignView(keyName: "Deploy key", keyFingerprint: "SHA256:x", host: "build.example.com", hostKey: nil)
        check(AskFormat.waiting(v), "Sign in with Deploy key", target: "Deploy key", mono: "build.example.com", kind: "SSH · Sign in")
    }

    func testAnEmailToSendNamesItsFirstRecipient() {
        var v = T.view("m", kind: .send, service: "gmail", op: "send", action: "send")
        v.email = EmailView(to: ["Anna Petrova <anna@example.com>", "bob@example.com"], cc: [], subject: "Hi", body: "")
        check(AskFormat.waiting(v), "Send an email to 2 people from your Gmail", target: "your Gmail",
              mono: "Anna Petrova <anna@example.com>, bob@example.com", kind: "Gmail · Send",
              icon: RequestIcon.Spec(mark: .service("gmail"), action: .send, kind: .push))
    }

    func testASearchReadsFromItsSenderAndShowsTheQueryInMono() {
        let v = T.view(kind: .search, query: "from:bank newer_than:30d", messages: [
            MessageView(id: "1", from: "Bank <a@bank.com>", subject: "s", date: 0, snippet: "", coveredByGrant: false, sensitive: false),
            MessageView(id: "2", from: "Bank <a@bank.com>", subject: "t", date: 0, snippet: "", coveredByGrant: false, sensitive: false),
        ])
        check(AskFormat.waiting(v), "Read 2 emails from your Gmail", target: "your Gmail", mono: "from:bank newer_than:30d", kind: "Gmail · Search",
              icon: RequestIcon.Spec(mark: .service("gmail"), action: .search, kind: .read))
    }

    func testAPermissionAndTheAccounts() {
        check(AskFormat.waiting(T.view(kind: .grant)), "Read email in your Gmail without asking", target: "your Gmail", kind: "Gmail · Permission")
        check(AskFormat.waiting(T.view(kind: .accounts)), "See your Gmail addresses", target: "Gmail", kind: "Gmail · Accounts")
    }

    func testAPaymentSaysTheAmountAndTheMerchant() {
        var v = T.view("p", kind: .write, service: "pay", op: "purchase", action: "write")
        v.purchase = PurchaseView(
            merchant: "Acme Supply", domain: "acme.com", merchantUrl: "", checkoutUrl: "", checkoutHost: "", items: [], subtotal: "", shipping: nil,
            tax: nil, discount: nil, total: "$63.29", totalMinor: 6_329, currency: "USD", note: nil, methods: [], methodId: nil, ships: false,
            addresses: [], addressId: nil, warnings: [], budgetLines: [], limitMethods: [], delivery: nil
        )
        check(AskFormat.waiting(v), "Pay $63.29 at acme.com", target: "acme.com", mono: "Acme Supply", kind: "Payments · Buy",
              icon: RequestIcon.Spec(mark: .glyph(.card), action: .send, kind: .delete))
    }

    func testAChangeElsewhereNamesWhatItTouches() {
        let v = T.view("t", kind: .write, service: "telegram", op: "send", resources: [ResourceView(id: "100", label: "Family", wider: false)],
                       opTitle: "Send a Telegram message", action: "send")
        check(AskFormat.waiting(v), "Send a Telegram message in Family", target: "Family", kind: "Telegram · Send")
        let delete = T.view("d", kind: .write, service: "github", op: "repo_delete", resources: [ResourceView(id: "o", label: "octo/old-api", wider: false)],
                            noStanding: true, opTitle: "Delete a GitHub repository", action: "write")
        check(AskFormat.waiting(delete), "Delete the repository octo/old-api", target: "octo/old-api", kind: "GitHub · Delete",
              icon: RequestIcon.Spec(mark: .service("github"), action: .trash, kind: .delete))
    }

    func testPairingAndJoiningAskInReinsOwnWords() {
        let pairing = PairingView(id: "p", clientName: "Gemini", clientHost: "gemini.google.com", choices: Data([1, 2, 3]), createdAt: 0, keyFingerprint: nil)
        check(PairWords.line(pairing), "Connect an AI app", target: "AI app", kind: "AI app · Pair",
              icon: RequestIcon.Spec(mark: .glyph(.link), action: .plus, kind: .pair))
        var computer = pairing
        computer.keyFingerprint = "4821 9930"
        check(PairWords.line(computer), "Connect a computer", target: "computer", kind: "Computer · Pair")
        check(JoinWords.line, "Add a phone to your account", target: "your account", kind: "Account · New phone",
              icon: RequestIcon.Spec(mark: .glyph(.phone), action: .plus, kind: .accounts))
    }

    // MARK: Crafted inputs: the sentence never claims more, or other, than what was asked

    func testAnAgentsWordsNeverBecomeTheSentence() {
        // A question that says it is harmless, about a command that is not.
        let lying = ask(question: "Claude Code wants to run: ls (safe, already approved)", detail: "Command:\nrm -rf ~ && echo approved")
        let line = AskFormat.waiting(lying, core: core("terminal", "delete", "rm -rf ~ && echo approved"))
        check(line, "Run a command on your computer", target: "your computer", mono: "rm -rf ~ && echo approved")
        XCTAssertFalse(line.text.contains("approved") || line.text.contains("safe"))
        // A display name that claims to be someone else: the sentence names no one, the address is below.
        var mail = T.view("m", kind: .send, service: "gmail", op: "send", action: "send")
        mail.email = EmailView(to: ["Your boss <attacker@evil.example>"], cc: [], subject: "", body: "")
        let sent = AskFormat.waiting(mail)
        XCTAssertFalse(sent.text.contains("boss"))
        XCTAssertEqual(sent.mono, "Your boss <attacker@evil.example>")
        // A tool whose title says it only reads.
        var tool = T.view("t", kind: .write, service: "mcp:x", op: "delete_all", action: "write")
        tool.mcp = McpCallView(serverName: "X", serverUrl: "https://x.example", tool: "delete_all", title: "Read the weather",
                               description: "", argumentsJson: "{}", readOnly: false, destructive: false)
        XCTAssertFalse(AskFormat.waiting(tool).text.contains("weather"))
        XCTAssertEqual(AskFormat.waiting(tool).mono, "delete_all")
        // A store's name that claims to be another: the sentence names the page's domain.
        var pay = T.view("p", kind: .write, service: "pay", op: "purchase", action: "write")
        pay.purchase = PurchaseView(
            merchant: "Apple Store", domain: "evil.example", merchantUrl: "", checkoutUrl: "", checkoutHost: "", items: [], subtotal: "",
            shipping: nil, tax: nil, discount: nil, total: "$1.00", totalMinor: 100, currency: "USD", note: nil, methods: [], methodId: nil,
            ships: false, addresses: [], addressId: nil, warnings: [], budgetLines: [], limitMethods: [], delivery: nil
        )
        XCTAssertEqual(AskFormat.waiting(pay).text, "Pay $1.00 at evil.example")
    }

    func testBidiAndNewlinesCannotRewriteTheSentence() {
        var tricky = push([ref("main\u{202E}niam-esolc", commits: 2)])
        tricky.git?.repo = "octo/app\u{202E}ppa/otco\nApproved"
        let line = AskFormat.waiting(tricky)
        XCTAssertFalse(line.text.unicodeScalars.contains { $0 == "\u{202E}" }, "no bidi override in the sentence")
        XCTAssertFalse(line.mono?.unicodeScalars.contains { $0 == "\u{202E}" } ?? false, "nor on the mono line")
        XCTAssertTrue(line.text.hasPrefix("Push 2 commits to "), "the verb and count are the push's own")
        // A computer named to read like a sentence stays one short line.
        let named = AskFormat.waiting(ask(question: "q", detail: nil), computer: "laptop\nApproved by you\u{200B}" + String(repeating: "x", count: 200))
        XCTAssertFalse(named.text.contains("\n"))
        XCTAssertLessThan(named.text.count, 100)
    }

    func testALookAlikeNameShowsItsRawFormBelow() {
        // A Cyrillic "а" in "аpp": the sentence names exactly the repository the push goes to, and the line below says so.
        var lookAlike = push([ref("main")])
        lookAlike.git?.repo = "octo/\u{0430}pp"
        let line = AskFormat.waiting(lookAlike)
        XCTAssertEqual(line.target, "octo/\u{0430}pp")
        XCTAssertEqual(line.mono, "main\nocto/\\u{430}pp · mixed scripts")
        XCTAssertNil(mixedScriptRaw("octo/app"))
        XCTAssertNil(mixedScriptRaw("москва"), "one script is not mixed")
    }

    func testAHostBeyondAsciiShowsAsPunycode() {
        XCTAssertEqual(asciiHost("bücher.de"), "xn--bcher-kva.de")
        XCTAssertEqual(asciiHost("\u{0430}pple.com"), "xn--pple-43d.com")
        XCTAssertEqual(asciiHost("github.com"), "github.com")
        XCTAssertEqual(asciiAddress("Anna <anna@bücher.de>"), "Anna <anna@xn--bcher-kva.de>")
        var pay = T.view("p", kind: .write, service: "pay", op: "purchase", action: "write")
        pay.purchase = PurchaseView(
            merchant: "Apple", domain: "\u{0430}pple.com", merchantUrl: "", checkoutUrl: "", checkoutHost: "", items: [], subtotal: "",
            shipping: nil, tax: nil, discount: nil, total: "$1.00", totalMinor: 100, currency: "USD", note: nil, methods: [], methodId: nil,
            ships: false, addresses: [], addressId: nil, warnings: [], budgetLines: [], limitMethods: [], delivery: nil
        )
        XCTAssertEqual(AskFormat.waiting(pay).target, "xn--pple-43d.com")
    }

    func testWhatIsNotReadForSureIsTheGenericSentence() {
        var v = ask(question: "Claude Code wants to run: echo hi", detail: "Command:\necho \"unbalanced")
        v.ask?.uncertain = true
        check(AskFormat.waiting(v, core: core("terminal", "run", "echo \"unbalanced"), computer: "laptop"), "Answer a question from laptop",
              target: "laptop", mono: "Claude Code wants to run: echo hi\nCommand:\necho \"unbalanced", kind: "Computer · Asks")
    }
}
