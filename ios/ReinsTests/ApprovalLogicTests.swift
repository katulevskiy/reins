import XCTest
@testable import Reins

/// The Android app's ApprovalLogicTest, case for case.
final class ApprovalLogicTests: XCTestCase {
    // MARK: Builders (the Android tests' TestData)

    static func message(_ id: String, _ from: String, covered: Bool = false, sensitive: Bool = false) -> MessageView {
        MessageView(id: id, from: from, subject: "Subject \(id)", date: 1_700_000_000, snippet: "snippet of \(id)", coveredByGrant: covered, sensitive: sensitive)
    }

    static func view(
        _ id: String = "r1", label: String = "Claude", kind: ApprovalKind, query: String? = nil, messages: [MessageView] = [],
        email: EmailView? = nil, service: String = "gmail", account: String? = nil, grant: GrantRequestView? = nil, count: UInt32 = 1,
        accounts: [String] = [], shared: [String] = [], op: String = "", resources: [ResourceView] = [], preview: [String] = [],
        noStanding: Bool = false, opTitle: String = "", action: String = "search", klass: String = "", classes: [ClassOption] = []
    ) -> ApprovalView {
        ApprovalView(
            requestId: id, connectionId: "c1", connectionLabel: label, kind: kind, query: query, messages: messages, email: email,
            createdAt: 0, service: service, account: account, waitUntil: nil, grant: grant, count: count, accounts: accounts,
            sharedAccounts: shared, op: op, resources: resources, preview: preview, noStanding: noStanding, opTitle: opTitle,
            action: action, class: klass, classes: classes, git: nil, blob: nil, mcp: nil, ask: nil, secrets: nil, ssh: nil
        )
    }

    let readView = view(kind: .search, query: "from:bank", messages: [
        message("m1", "Bank <alerts@bank.com>"),
        message("m2", "Other <x@gmail.com>"),
        message("m3", "old@bank.com", covered: true),
    ], count: 3)

    let sendView = view("r2", label: "ChatGPT", kind: .send, email: EmailView(to: ["Ann <ann@corp.com>"], cc: ["bob@corp.com"], subject: "Hi", body: "body"), count: 2)

    static func accountsView(_ addresses: [String] = ["me@gmail.com", "work@corp.example"], shared: [String] = []) -> ApprovalView {
        view("req5", kind: .accounts, count: UInt32(addresses.count - shared.count), accounts: addresses, shared: shared, action: "accounts")
    }

    static func grantView(duration: UInt64 = 3600) -> ApprovalView {
        view("req3", kind: .grant, account: "me@gmail.com", grant: GrantRequestView(
            action: "read", summary: "read emails from alerts@bank.com for 1 hour", reason: "Summarise this week's statements",
            durationSecs: duration, maxUses: nil, breadth: "narrow", lines: ["From alerts@bank.com"]
        ), action: "grant")
    }

    static func fetchView() -> ApprovalView {
        view("req7", kind: .fetch, messages: [
            MessageView(id: "100:2", from: "Anna", subject: "", date: 1_700_000_050, snippet: "Dinner at eight?", coveredByGrant: false, sensitive: false),
            MessageView(id: "100:1", from: "Bob", subject: "", date: 1_700_000_000, snippet: "I am late", coveredByGrant: false, sensitive: false),
            MessageView(id: "100:9", from: "Telegram", subject: "", date: 1_700_000_090, snippet: "Login code: 48151", coveredByGrant: false, sensitive: true),
        ], service: "telegram", account: "+15550100", count: 3, op: "read", resources: [ResourceView(id: "100", label: "Family", wider: false)],
        opTitle: "Read Telegram messages", action: "read")
    }

    static func vaultView() -> ApprovalView {
        view("req8", kind: .fetch, messages: [MessageView(id: "git:password", from: "octo", subject: "GitHub", date: 0, snippet: "Password for GitHub", coveredByGrant: false, sensitive: true)],
             service: "vault", account: "me@example.com", op: "get", resources: [ResourceView(id: "git", label: "GitHub", wider: false)],
             noStanding: true, opTitle: "Get a login from the vault", action: "read")
    }

    static func writeView() -> ApprovalView {
        view("req9", kind: .write, service: "telegram", account: "+15550100", op: "send", resources: [ResourceView(id: "100", label: "Family", wider: false)],
             preview: ["Send to Family", "Dinner at eight works"], opTitle: "Send a Telegram message", action: "send")
    }

    static let repoClasses = [
        ClassOption(id: "issues", label: "Issues"), ClassOption(id: "pulls", label: "Pull requests"),
        ClassOption(id: "code", label: "Code"), ClassOption(id: "releases", label: "Releases"),
    ]

    static func repoWriteView() -> ApprovalView {
        var v = writeView()
        v.requestId = "req10"
        v.service = "github"
        v.account = "octo-cat"
        v.op = "file_put"
        v.resources = [
            ResourceView(id: "octo/app@main", label: "Branch main of octo/app", wider: false),
            ResourceView(id: "octo/app", label: "Any branch of octo/app", wider: true),
            ResourceView(id: "octo", label: "Every repository of octo", wider: true),
        ]
        v.preview = ["Commit README.md to main of octo/app", "Fix the typo in the title"]
        v.opTitle = "Commit a file to GitHub"
        v.action = "write"
        v.class = "code"
        v.classes = repoClasses
        return v
    }

    static func onceOnlyWriteView() -> ApprovalView {
        var v = repoWriteView()
        v.requestId = "req11"
        v.op = "repo_delete"
        v.resources = [ResourceView(id: "octo/app", label: "octo/app", wider: false), ResourceView(id: "octo", label: "Every repository of octo", wider: true)]
        v.noStanding = true
        v.class = "settings"
        v.classes = repoClasses + [ClassOption(id: "settings", label: "Settings")]
        return v
    }

    static func repoReadView() -> ApprovalView {
        var v = fetchView()
        v.requestId = "req12"
        v.service = "github"
        v.account = "octo-cat"
        v.op = "contents_get"
        v.messages = [MessageView(id: "README.md", from: "octo/app", subject: "README.md", date: 0, snippet: "Hello", coveredByGrant: false, sensitive: false)]
        v.resources = [
            ResourceView(id: "octo/app@main", label: "Branch main of octo/app", wider: false),
            ResourceView(id: "octo/app", label: "Any branch of octo/app", wider: true),
        ]
        v.opTitle = "Read a file from GitHub"
        return v
    }

    private func ok(_ result: BuildResult, file: StaticString = #filePath, line: UInt = #line) -> ApprovalChoice {
        guard case let .ok(choice) = result else {
            XCTFail("expected a choice, got \(result)", file: file, line: line)
            return ApprovalChoice(selectedMessageIds: [], standing: nil)
        }
        return choice
    }

    private func invalid(_ result: BuildResult) -> Bool {
        if case .invalid = result { return true }
        return false
    }

    // MARK: Accounts and permissions

    func testShowingAccountsIsOnceOrForAPeriodNeverOpenEnded() {
        let view = Self.accountsView()
        let all = Set(view.accounts)
        let month = ok(buildChoice(view, ApprovalDraft(selected: all, lifetime: .month)))
        XCTAssertEqual(month.standing?.durationSecs, 2_592_000)
        XCTAssertEqual(month.selectedMessageIds, view.accounts)
        XCTAssertNil(ok(buildChoice(view, ApprovalDraft(selected: all, lifetime: .once))).standing)
        XCTAssertEqual(ok(buildChoice(view, ApprovalDraft(selected: all, lifetime: .hour))).standing?.durationSecs, 3_600)
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(selected: all, lifetime: .untilRevoked))))
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(selected: all, lifetime: .uses))))
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(lifetime: .month))), "nothing ticked")
        XCTAssertEqual(ok(buildChoice(view, ApprovalDraft(selected: ["me@gmail.com", "stranger@x.com"], lifetime: .month))).selectedMessageIds, ["me@gmail.com"])
        let more = Self.accountsView(shared: ["me@gmail.com"])
        XCTAssertEqual(ok(buildChoice(more, ApprovalDraft(selected: all, lifetime: .month))).selectedMessageIds, ["work@corp.example"], "already shared ones are not picks")
    }

    func testAPermissionRequestIsAllowedAsAskedOrMadeShorterNeverLonger() {
        let grantView = Self.grantView(duration: 3600)
        let asIs = ok(buildChoice(grantView, ApprovalDraft()))
        XCTAssertNil(asIs.standing)
        XCTAssertTrue(asIs.selectedMessageIds.isEmpty)
        XCTAssertEqual(ok(buildChoice(grantView, ApprovalDraft(grantSeconds: 600))).standing?.durationSecs, 600)
        XCTAssertNil(ok(buildChoice(grantView, ApprovalDraft(grantSeconds: 7200))).standing, "asking for longer than requested is ignored")
    }

    func testShortenStepsEndWithWhatWasAsked() {
        XCTAssertEqual(shortenSteps(asked: 3_600), [60, 600, 3_600])
        XCTAssertEqual(shortenSteps(asked: 5_400), [60, 600, 3_600, 5_400])
        XCTAssertEqual(shortenSteps(asked: 30), [30])
    }

    // MARK: Gmail

    func testAddressesArePulledOutOfDisplayNamesAndLowerCased() {
        XCTAssertEqual(addressOf("Name <A@B.com>"), "a@b.com")
        XCTAssertEqual(addressOf("a@b.com"), "a@b.com")
        XCTAssertNil(addressOf("(unknown sender)"))
        XCTAssertNil(addressOf("Name <not an address>"))
        XCTAssertNil(addressOf("a@b@c.com"))
    }

    func testOnceCreatesNoStandingGrantAndAlwaysIncludesCoveredMessages() {
        let choice = ok(buildChoice(readView, ApprovalDraft(selected: ["m1"])))
        XCTAssertEqual(choice.selectedMessageIds, ["m1", "m3"])
        XCTAssertNil(choice.standing)
    }

    func testReadsNeedAtLeastOneMessage() {
        var plain = readView
        plain.messages = [Self.message("m1", "a@b.com")]
        XCTAssertTrue(invalid(buildChoice(plain, ApprovalDraft())))
    }

    func testLifetimesMapToSecondsOrUses() {
        func standing(_ kind: LifetimeKind, uses: Int = 5) -> StandingGrant? {
            ok(buildChoice(readView, ApprovalDraft(selected: ["m1"], lifetime: kind, uses: uses))).standing
        }
        XCTAssertEqual(standing(.hour)?.durationSecs, 3_600)
        XCTAssertEqual(standing(.day)?.durationSecs, 86_400)
        XCTAssertEqual(standing(.week)?.durationSecs, 604_800)
        XCTAssertNotNil(standing(.untilRevoked))
        XCTAssertNil(standing(.untilRevoked)?.durationSecs)
        XCTAssertNil(standing(.untilRevoked)?.maxUses)
        let uses = standing(.uses, uses: 7)
        XCTAssertEqual(uses?.maxUses, 7)
        XCTAssertNil(uses?.durationSecs)
    }

    func testUsesAreBounded() {
        var draft = ApprovalDraft(selected: ["m1"], lifetime: .uses)
        draft.uses = 0
        XCTAssertTrue(invalid(buildChoice(readView, draft)))
        draft.uses = 1001
        XCTAssertTrue(invalid(buildChoice(readView, draft)))
        draft.uses = 1000
        XCTAssertFalse(invalid(buildChoice(readView, draft)))
    }

    func testAStandingReadGrantDefaultsToTheSelectedMessagesOnly() {
        let scope = ok(buildChoice(readView, ApprovalDraft(selected: ["m1"], lifetime: .day))).standing?.scope
        XCTAssertEqual(scope?.selectedMessagesOnly, true)
        XCTAssertEqual(scope?.senderAddresses, [])
        XCTAssertEqual(scope?.senderDomains, [])
        XCTAssertNil(scope?.subjectPattern)
    }

    func testSimilarMailNeedsARuleAndCarriesIt() {
        let base = ApprovalDraft(selected: ["m1"], lifetime: .week, similar: true)
        XCTAssertTrue(invalid(buildChoice(readView, base)))
        var draft = base
        draft.senderDomains = ["bank.com"]
        draft.subject = "  statement "
        let scope = ok(buildChoice(readView, draft)).standing?.scope
        XCTAssertEqual(scope?.selectedMessagesOnly, false)
        XCTAssertEqual(scope?.senderDomains, ["bank.com"])
        XCTAssertEqual(scope?.subjectPattern, "statement")
    }

    func testAStandingSendGrantAllowsEveryRecipientExactlyByDefault() {
        let choice = ok(buildChoice(sendView, ApprovalDraft(lifetime: .hour)))
        XCTAssertTrue(choice.selectedMessageIds.isEmpty)
        XCTAssertEqual(choice.standing?.scope.recipientAddresses, ["ann@corp.com", "bob@corp.com"])
        XCTAssertEqual(choice.standing?.scope.recipientDomains, [])
    }

    func testARecipientCanBeWidenedToItsDomain() {
        let scope = ok(buildChoice(sendView, ApprovalDraft(lifetime: .hour, domainRecipients: ["ann@corp.com"]))).standing?.scope
        XCTAssertEqual(scope?.recipientAddresses, ["bob@corp.com"])
        XCTAssertEqual(scope?.recipientDomains, ["corp.com"])
    }

    func testAllowingAllMailIsExplicitTimeBoxedAndReleasesEverythingShown() {
        let choice = ok(buildChoice(readView, ApprovalDraft(selected: ["m1", "m2"], allMail: .hour)))
        XCTAssertEqual(choice.selectedMessageIds, ["m1", "m2", "m3"])
        let standing = choice.standing
        XCTAssertEqual(standing?.durationSecs, 3_600)
        XCTAssertNil(standing?.maxUses)
        XCTAssertEqual(standing?.scope.allMail, true)
        XCTAssertEqual(standing?.scope.selectedMessagesOnly, false)
        XCTAssertEqual(standing?.scope.senderDomains, [])
        XCTAssertEqual(ok(buildChoice(readView, ApprovalDraft(selected: ["m1"], allMail: .week))).standing?.durationSecs, 604_800)
    }

    func testAllMailRefusesOpenEndedLifetimesAndSending() {
        for kind in [LifetimeKind.once, .untilRevoked, .uses] {
            XCTAssertTrue(invalid(buildChoice(readView, ApprovalDraft(selected: ["m1"], allMail: kind))), "\(kind)")
        }
        XCTAssertTrue(invalid(buildChoice(sendView, ApprovalDraft(allMail: .hour))))
    }

    func testAllMailNeedsNoTicksBecauseEverythingShownIsReleased() {
        XCTAssertEqual(ok(buildChoice(readView, ApprovalDraft(allMail: .day))).selectedMessageIds, ["m1", "m2", "m3"])
    }

    func testPublicMailDomainsAreFlagged() {
        XCTAssertTrue(PublicMailDomains.isPublic("Gmail.com"))
        XCTAssertTrue(PublicMailDomains.isPublic("@outlook.com"))
        XCTAssertFalse(PublicMailDomains.isPublic("bank.com"))
        let choice = ok(buildChoice(readView, ApprovalDraft(selected: ["m2"], lifetime: .day, similar: true, senderDomains: ["gmail.com", "bank.com"])))
        XCTAssertEqual(publicDomainWarnings(choice), ["gmail.com"])
        XCTAssertTrue(publicDomainWarnings(ok(buildChoice(readView, ApprovalDraft(selected: ["m2"])))).isEmpty)
    }

    // MARK: Another integration

    func testAFetchReleasesTheTickedItemsAndTheOnesAGrantAlreadyCovers() {
        let view = Self.fetchView()
        let once = ok(buildChoice(view, ApprovalDraft(selected: ["100:2"])))
        XCTAssertEqual(once.selectedMessageIds, ["100:2"])
        XCTAssertNil(once.standing)
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft())))
    }

    func testAFetchCanBeRememberedForTheChatsNamedOrForEverythingForAShortTime() {
        let view = Self.fetchView()
        let named = ok(buildChoice(view, ApprovalDraft(selected: ["100:2"], lifetime: .day, resources: ["100"])))
        XCTAssertEqual(named.standing?.scope.resources, ["100"])
        XCTAssertEqual(named.standing?.durationSecs, 86_400)
        XCTAssertEqual(named.standing?.scope.allMail, false)
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(selected: ["100:2"], lifetime: .day, resources: ["nope"]))))
        let everything = ok(buildChoice(view, ApprovalDraft(selected: ["100:2"], allMail: .week)))
        XCTAssertEqual(everything.standing?.scope.allMail, true)
        XCTAssertEqual(everything.standing?.scope.resources, [])
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(selected: ["100:2"], allMail: .month))))
        let uses = ok(buildChoice(view, ApprovalDraft(selected: ["100:2"], lifetime: .uses, uses: 3, resources: ["100"])))
        XCTAssertEqual(uses.standing?.maxUses, 3)
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(selected: ["100:2"], lifetime: .uses, uses: 0, resources: ["100"]))))
    }

    func testAPasswordIsNeverRememberedWhateverTheDraftSays() {
        let choice = ok(buildChoice(Self.vaultView(), ApprovalDraft(selected: ["git:password"], lifetime: .week, resources: ["git"])))
        XCTAssertNil(choice.standing)
        XCTAssertEqual(choice.selectedMessageIds, ["git:password"])
    }

    func testAChangeIsApprovedAsShownAndCanBeRememberedForItsChatButNotEverywhere() {
        let view = Self.writeView()
        let once = ok(buildChoice(view, ApprovalDraft()))
        XCTAssertTrue(once.selectedMessageIds.isEmpty)
        XCTAssertNil(once.standing)
        XCTAssertEqual(ok(buildChoice(view, ApprovalDraft(lifetime: .hour, resources: ["100"]))).standing?.scope.resources, ["100"])
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(allMail: .hour))))
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(lifetime: .hour))))
    }

    // MARK: Kinds of change and wider permissions

    func testTheKindOfTheRequestStartsTickedAndTheLastKindCannotBeUnticked() {
        XCTAssertEqual(defaultClasses(Self.repoWriteView()), ["code"])
        XCTAssertEqual(toggleClass(["code"], "code", on: false), ["code"])
        XCTAssertEqual(toggleClass(["code"], "issues", on: true), ["code", "issues"])
        XCTAssertEqual(toggleClass(["code", "issues"], "code", on: false), ["issues"])
        XCTAssertTrue(defaultClasses(Self.fetchView()).isEmpty)
    }

    func testTheKindsTickedTravelWithThePermissionInTheOrderTheyAreOffered() {
        let view = Self.repoWriteView()
        let choice = ok(buildChoice(view, ApprovalDraft(lifetime: .day, resources: ["octo/app@main"], classes: ["releases", "code"])))
        XCTAssertEqual(choice.standing?.scope.classes, ["code", "releases"])
        XCTAssertEqual(choice.standing?.scope.resources, ["octo/app@main"])
        // Without a permission nothing about kinds is sent.
        XCTAssertNil(ok(buildChoice(view, ApprovalDraft(classes: ["code"]))).standing)
        // Reads have no kinds.
        let read = ok(buildChoice(Self.repoReadView(), ApprovalDraft(selected: ["README.md"], lifetime: .hour, resources: ["octo/app"], classes: ["code"])))
        XCTAssertEqual(read.standing?.scope.classes, [])
    }

    func testTheIdsFollowTheRuleThatACoversAAtXAndASlashX() {
        XCTAssertTrue(resourceCovers("octo/app", "octo/app"))
        XCTAssertTrue(resourceCovers("octo/app", "octo/app@main"))
        XCTAssertTrue(resourceCovers("octo", "octo/app"))
        XCTAssertTrue(resourceCovers("git", "git/login/7"))
        XCTAssertFalse(resourceCovers("octo/app", "octo/app2"))
        XCTAssertFalse(resourceCovers("octo/app@main", "octo/app"))
        XCTAssertFalse(resourceCovers("oct", "octo/app"))
    }

    func testOnlyWhatTheRequestTouchesStartsTickedTheWiderThingsAreNot() {
        XCTAssertEqual(defaultResources(Self.repoWriteView()), ["octo/app@main"])
        XCTAssertEqual(defaultResources(Self.fetchView()), ["100"])
    }

    func testTickingAWiderThingUnticksTheNarrowerOnesAndTheOtherWayRound() {
        XCTAssertEqual(toggleResource(["octo/app@main"], "octo/app", on: true), ["octo/app"])
        XCTAssertEqual(toggleResource(["octo/app@main", "octo/app"], "octo", on: true), ["octo"])
        XCTAssertEqual(toggleResource(["octo/app"], "octo/app@main", on: true), ["octo/app@main"])
        XCTAssertEqual(toggleResource(["octo/app@main"], "octo/other", on: true), ["octo/app@main", "octo/other"])
        XCTAssertEqual(toggleResource(["octo/app@main"], "octo/app@main", on: false), [])
    }

    func testAWiderPermissionIsSentAloneEvenIfANarrowerOneIsStillInTheDraft() {
        let view = Self.repoWriteView()
        XCTAssertEqual(ok(buildChoice(view, ApprovalDraft(lifetime: .hour, resources: ["octo/app@main", "octo/app"]))).standing?.scope.resources, ["octo/app"])
        XCTAssertTrue(invalid(buildChoice(view, ApprovalDraft(lifetime: .hour, resources: ["elsewhere/repo"]))))
    }

    func testAChangeThatIsAskedForEveryTimeIsNeverRemembered() {
        let choice = ok(buildChoice(Self.onceOnlyWriteView(), ApprovalDraft(lifetime: .week, resources: ["octo"], classes: ["settings"])))
        XCTAssertNil(choice.standing)
        XCTAssertTrue(choice.selectedMessageIds.isEmpty)
    }

    // MARK: What the sheet starts with

    func testEverythingFoundStartsTickedExceptCodesAndNothingForSendsOrChanges() {
        XCTAssertEqual(ApprovalDraft.initial(for: readView).selected, ["m1", "m2", "m3"])
        XCTAssertEqual(ApprovalDraft.initial(for: Self.fetchView()).selected, ["100:2", "100:1"])
        XCTAssertEqual(ApprovalDraft.initial(for: sendView).selected, [])
        XCTAssertEqual(ApprovalDraft.initial(for: Self.repoWriteView()).selected, [])
        XCTAssertEqual(ApprovalDraft.initial(for: Self.repoWriteView()).resources, ["octo/app@main"])
        let accounts = ApprovalDraft.initial(for: Self.accountsView(shared: ["me@gmail.com"]))
        XCTAssertEqual(accounts.selected, ["work@corp.example"])
        XCTAssertEqual(accounts.lifetime, .month, "showing accounts is remembered for a month to start with")
        XCTAssertEqual(ApprovalDraft.initial(for: readView).lifetime, .once)
    }
}
