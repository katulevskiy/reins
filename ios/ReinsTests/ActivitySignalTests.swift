import XCTest
@testable import Reins

/// Activity in Signal: the mood line and the day's count, what a waiting card says and is painted in, Autopilot's pick
/// as its first button, the past tense of what was done, the river's tiles, the reason line and where a fold lands.
final class ActivitySignalTests: XCTestCase {
    private func item(
        _ id: String = "r1", kind: PendingKind = .request, action: String = "send", service: String = "gmail", subtitle: String = "",
        label: RequestLabel? = nil
    ) -> PendingItem {
        PendingItem(
            kind: kind, id: id, title: "", subtitle: subtitle, createdAt: 0, connectionId: "c1", connectionLabel: "Claude",
            action: action, count: 1, service: service, account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil, label: label
        )
    }

    func testAWaitingCardSaysItsKindAndAsksWithItsTargetInItalic() {
        let send = item(label: DemoData.requestLabel("gmail", "send", "Send an email to", "Anna Petrova"))
        XCTAssertEqual(ActivityText.label(send), "Gmail · Send")
        // A recipient's name is the AI's own words: it goes below the sentence (AskLabelsTests has every kind).
        XCTAssertEqual(ActivityText.ask(WaitGroup(items: [send])), [AskText.Run(text: "Send an email")])
        let push = item(label: DemoData.requestLabel("github", "push", "Push 3 commits to", "acme/web"))
        XCTAssertEqual(ActivityText.ask(WaitGroup(items: [push])).filter(\.target).map(\.text), ["acme/\u{2060}web"])
        XCTAssertEqual(ActivityText.label(item(kind: .pairing)), "Reins · Connect")
        XCTAssertEqual(ActivityText.label(item(kind: .join)), "Account · New phone")
        // Without a label: the AI's wish as what it would do.
        let runs = ActivityText.runs([WantsText.Piece(text: "Wants to "), WantsText.Piece(text: "search Gmail", strong: true),
                                      WantsText.Piece(text: " for "), WantsText.Piece(text: "“invoice”", strong: true)])
        XCTAssertEqual(runs.map(\.text).joined(), "Search Gmail for “invoice”")
        XCTAssertEqual(runs.filter(\.target).map(\.text), ["“invoice”"])
    }

    func testTheCardsFieldIsItsKind() {
        XCTAssertEqual(ActivityText.field(item(action: "send"), danger: false), .push)
        XCTAssertEqual(ActivityText.field(item(action: "search"), danger: false), .read)
        XCTAssertEqual(ActivityText.field(item(action: "send"), danger: true), .delete)
        XCTAssertEqual(ActivityText.field(item(kind: .pairing), danger: false), .pair)
        XCTAssertEqual(ActivityText.field(item(kind: .join), danger: false), .accounts, "a new phone changes the account")
        XCTAssertEqual(ActivityText.field(item(kind: .blob), danger: false), .push, "a file goes out")
    }

    func testAutopilotsPickIsTheFirstButton() {
        XCTAssertEqual(ActivityText.pick(.allow, nil), .allow(100))
        XCTAssertEqual(ActivityText.pick(.unsure(0.4), nil), .unsure)
        XCTAssertEqual(ActivityText.pick(.alwaysAsks, nil), .alwaysAsks)
        XCTAssertNil(ActivityText.pick(.review, nil))
        XCTAssertEqual(AutopilotPick.allow(97).title, "Allow 97%")
        XCTAssertTrue(AutopilotPick.deny(91).firm)
        XCTAssertFalse(AutopilotPick.unsure.firm)
    }

    func testWhatWasDoneReadsInThePast() {
        XCTAssertEqual(AskLabels.past("Push 3 commits to"), "Pushed 3 commits to")
        XCTAssertEqual(AskLabels.past("Send an email to"), "Sent an email to")
        XCTAssertEqual(AskLabels.past("Read 2 emails from"), "Read 2 emails from")
        XCTAssertEqual(AskLabels.past("Search"), "Searched")
        XCTAssertEqual(AskLabels.past("Frobnicate the thing"), "Frobnicate the thing", "an unknown verb stays as asked")
        let pushed = DemoData.entry(1, at: 0, action: "write", outcome: "sent", detail: "", service: "github",
                                    requestLabel: DemoData.requestLabel("github", "push", "Push 3 commits to", "acme/web"))
        XCTAssertEqual(HistoryRow.title(pushed), "Pushed 3 commits to acme/web")
        let denied = DemoData.entry(2, at: 0, action: "write", outcome: "denied", detail: "", service: "github",
                                    requestLabel: DemoData.requestLabel("github", "delete", "Delete branch", "old/experiment"))
        XCTAssertEqual(HistoryRow.title(denied), "Asked to delete branch old/experiment", "what was denied says what was asked")
    }

    func testTwoJoinedVerbsBothGoIntoThePast() {
        XCTAssertEqual(AskLabels.past("Create and push a branch"), "Created and pushed a branch")
        XCTAssertEqual(AskLabels.past("Push and frobnicate"), "Pushed and frobnicate", "only a verb it knows changes")
    }

    func testAGitFetchIsOneVerbInEveryTense() {
        func entry(_ outcome: String) -> ActivityEntry {
            DemoData.entry(1, at: 0, action: "read", outcome: outcome, detail: "", service: "github",
                           requestLabel: DemoData.requestLabel("github", "read", "Clone and fetch", "octo/app"))
        }
        XCTAssertEqual(HistoryRow.title(entry("released")), "Fetched octo/app", "never \"Cloned and fetch\"")
        XCTAssertEqual(HistoryRow.title(entry("denied")), "Asked to fetch octo/app")
        XCTAssertEqual(HistoryRow.title(entry("expired")), "Asked to fetch octo/app", "past tense only for what was carried out")
        XCTAssertEqual(AskLabels.oneVerb("Clone and fetch octo/app"), "Fetch octo/app")
        XCTAssertEqual(AskLabels.oneVerb("clone and fetch octo/app"), "fetch octo/app")
        XCTAssertEqual(AskLabels.oneVerb("Clone octo/app"), "Clone octo/app")
    }

    func testTheRiversTilesAreTheKindsGlyphs() {
        let push = DemoData.entry(1, at: 0, action: "write", outcome: "sent", detail: "", service: "github", op: "git_push",
                                  requestLabel: DemoData.requestLabel("github", "push", "Push 3 commits to", "acme/web"))
        XCTAssertEqual(EntryLook.field(push), .push)
        XCTAssertEqual(EntryLook.glyph(push), .gitBranch)
        let delete = DemoData.entry(2, at: 0, action: "write", outcome: "denied", detail: "", service: "github",
                                    requestLabel: DemoData.requestLabel("github", "delete", "Delete branch", "old"))
        XCTAssertEqual(EntryLook.field(delete), .delete)
        XCTAssertEqual(EntryLook.glyph(delete), .trash)
        let read = DemoData.entry(3, at: 0, action: "read", outcome: "released", detail: "")
        XCTAssertEqual(EntryLook.field(read), .read)
        XCTAssertEqual(EntryLook.glyph(read), .mail)
    }

    func testTheReasonLineCountsYourDecisionsAndNamesTheBranch() {
        let push = ReasonText.noun(action: "write", op: "git_push", service: "github")
        XCTAssertEqual(push.many, "pushes")
        XCTAssertEqual(ReasonText.line(count: 11, noun: push), "Like 11 pushes you approved.")
        XCTAssertEqual(ReasonText.line(count: 11, noun: push, repo: "acme/web", branch: .feature), "Like 11 pushes to feature branches of acme/web you approved.")
        XCTAssertEqual(ReasonText.line(count: 1, noun: push, repo: "acme/web", branch: .main), "Like 1 push to the default branch of acme/web you approved.")
        XCTAssertNil(ReasonText.line(count: 0, noun: push))
        XCTAssertEqual(ReasonText.line(count: 3, noun: ReasonText.noun(action: "search", op: "", service: "gmail")), "Like 3 searches you approved.")
        XCTAssertEqual(ReasonText.branchKind("main", defaultBranch: nil), .main)
        XCTAssertEqual(ReasonText.branchKind("feature/login", defaultBranch: nil), .feature)
        XCTAssertEqual(ReasonText.branchKind("develop", defaultBranch: "develop"), .main, "the repository's own default")
    }

    @MainActor
    func testAFoldLandsOnTheEntryItsDecisionWrote() {
        let request = FoldRequest(field: .push, connectionId: "c1", action: "send", after: 10, fromSheet: true)
        let entries = [
            DemoData.entry(13, at: 0, conn: "c1", action: "send", outcome: "sent", detail: ""),
            DemoData.entry(12, at: 0, conn: "c2", action: "send", outcome: "sent", detail: ""),
            DemoData.entry(10, at: 0, conn: "c1", action: "send", outcome: "sent", detail: ""),
        ]
        XCTAssertEqual(FoldState.target(request, in: entries)?.id, 13)
        XCTAssertNil(FoldState.target(request, in: Array(entries.suffix(1))), "nothing newer: the fold only fades")
        let other = FoldRequest(field: .push, connectionId: "c2", action: "read", after: 10, fromSheet: true)
        XCTAssertEqual(FoldState.target(other, in: entries)?.id, 12, "else the connection's newest")
    }

    func testTheAIMenuRowsNameAConnectionOrAny() {
        XCTAssertEqual(ActivityFilterBar.connection("ai:c1"), "c1")
        XCTAssertEqual(ActivityFilterBar.connection("ai:"), "")
        XCTAssertEqual(ActivityFilterBar.name(ActivityConnection(id: "c1", label: "Clau\u{202E}de\n")), "Claude")
        XCTAssertEqual(ActivityFilterBar.name(ActivityConnection(id: "c1", label: "")), "Unnamed AI")
    }
}
