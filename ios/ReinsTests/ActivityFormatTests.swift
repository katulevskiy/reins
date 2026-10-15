import XCTest
@testable import Reins

/// Untrusted text (the Android app's UntrustedTest), the countdown, and times and files in words.
final class ActivityFormatTests: XCTestCase {
    private func c(_ code: UInt32) -> String { String(UnicodeScalar(code)!) }

    func testBidiControlsAreRemoved() {
        XCTAssertEqual(untrusted(c(0x202E) + "evil.com" + c(0x202C)), "evil.com")
        XCTAssertEqual(untrusted("a" + c(0x202E) + "b" + c(0x2066) + "c" + c(0x200F)), "abc")
        XCTAssertEqual(untrusted(c(0x061C) + "x" + c(0x2069)), "x")
    }

    func testControlCharactersBecomeSpacesAndEdgesAreTrimmed() {
        XCTAssertEqual(untrusted("a" + c(0) + "b"), "a b")
        XCTAssertEqual(untrusted("a" + c(0x2028) + "b"), "a b")
        XCTAssertEqual(untrusted("  line1\nline2 " + c(7)), "line1\nline2")
    }

    func testTheCountdownEmptiesTurnsUrgentThenStale() {
        XCTAssertNil(Urgency.of(createdAt: 100, waitUntil: nil, now: 120))
        let fresh = Urgency.of(createdAt: 100, waitUntil: 160, now: 100)
        XCTAssertEqual(fresh?.fraction, 1)
        XCTAssertEqual(fresh?.remainingSeconds, 60)
        XCTAssertEqual(fresh?.urgent, false)
        let half = Urgency.of(createdAt: 100, waitUntil: 160, now: 130)
        XCTAssertEqual(half?.fraction ?? 0, 0.5, accuracy: 0.001)
        XCTAssertEqual(Urgency.of(createdAt: 100, waitUntil: 160, now: 146)?.urgent, true)
        XCTAssertEqual(Urgency.of(createdAt: 100, waitUntil: 160, now: 146)?.remainingSeconds, 14)
        let late = Urgency.of(createdAt: 100, waitUntil: 160, now: 170)
        XCTAssertEqual(late?.stale, true)
        XCTAssertEqual(late?.urgent, false)
        XCTAssertEqual(late?.fraction, 0)
        XCTAssertEqual(late?.remainingSeconds, 0)
    }

    func testRelativeTimesReadAsWords() {
        let now: Int64 = 1_700_000_000
        XCTAssertEqual(TimeText.relative(now - 10, now: now), "just now")
        XCTAssertEqual(TimeText.relative(now - 60 * 5, now: now), "5 min ago")
        XCTAssertEqual(TimeText.relative(now - 3_600 * 3, now: now), "3 h ago")
        XCTAssertEqual(TimeText.relative(now - 86_400 - 10, now: now), "yesterday")
        XCTAssertEqual(TimeText.relative(now - 86_400 * 4, now: now), "4 days ago")
        XCTAssertEqual(TimeText.expiry(nil, now: now), "no time limit")
        XCTAssertEqual(TimeText.expiry(now - 1, now: now), "expired")
        XCTAssertEqual(TimeText.expiry(now + 30, now: now), "expires in 1 min")
        XCTAssertEqual(TimeText.expiry(now + 7_300, now: now), "expires in 2 h")
        XCTAssertEqual(TimeText.duration(600), "10 min")
        XCTAssertEqual(TimeText.duration(6 * 3_600), "6 h")
        XCTAssertEqual(TimeText.duration(86_400), "24 hours")
        XCTAssertEqual(TimeText.duration(7 * 86_400), "7 days")
    }

    func testFilesAreDescribedBySizeShortHashAndKind() {
        XCTAssertEqual(FileText.size(812), "812 bytes")
        XCTAssertEqual(FileText.size(18_432), "18.0 KB")
        XCTAssertEqual(FileText.size(1_468_006), "1.4 MB")
        XCTAssertEqual(FileText.size(2_147_483_648), "2.0 GB")
        XCTAssertEqual(FileText.shortSha("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"), "9f86d081884c7d65…0a08")
        XCTAssertEqual(FileText.shortSha("abc"), "abc")
        XCTAssertTrue(FileText.isText("text/plain"))
        XCTAssertTrue(FileText.isText("text/csv; charset=utf-8"))
        XCTAssertTrue(FileText.isText("application/json"))
        XCTAssertFalse(FileText.isText("application/pdf"))
        XCTAssertFalse(FileText.isText("image/png"))
    }

    func testAnEntryFromBeforeIdsWereKeptFindsItsConnectionByAUniqueLabel() {
        let a = ConnectionView(id: "c1", label: "Claude", clientHost: "", createdAt: 0, lastUsedAt: nil, icon: "claude", keyFingerprint: nil)
        let b = ConnectionView(id: "c2", label: "Twin", clientHost: "", createdAt: 0, lastUsedAt: nil, icon: nil, keyFingerprint: nil)
        let c = ConnectionView(id: "c3", label: "Twin", clientHost: "", createdAt: 0, lastUsedAt: nil, icon: nil, keyFingerprint: nil)
        XCTAssertEqual(findConnection([a, b], id: "c2", label: "x")?.id, "c2")
        XCTAssertEqual(findConnection([a, b], id: "", label: "Claude")?.id, "c1")
        XCTAssertNil(findConnection([a, b, c], id: "", label: "Twin"), "two connections carry the label")
        XCTAssertNil(findConnection([a], id: "gone", label: "Claude"), "an id that is gone is not guessed")
    }

    func testSuggestionHeadlinesSayWhatAutopilotWouldDo() {
        func s(_ verdict: Verdict, floor: Bool = false, judged: Bool = true) -> SuggestionView {
            SuggestionView(requestId: "r", verdict: verdict, mode: .manual, pApprove: 0.97, pDeny: 0.02, confidence: 0.9, reason: "", neighbours: [],
                           profileId: "p", profileName: "Personal", classKey: "k", novel: false, floor: floor, judged: judged)
        }
        XCTAssertEqual(SuggestionText.headline(s(.approve)), "Autopilot would approve · 97%")
        XCTAssertEqual(SuggestionText.headline(s(.deny)), "Autopilot would deny · 2%")
        XCTAssertEqual(SuggestionText.headline(s(.ask)), "Autopilot would ask you · approve 97%")
        XCTAssertEqual(SuggestionText.headline(s(.approve, floor: true)), "Autopilot always asks you for this")
        XCTAssertEqual(SuggestionText.headline(s(.approve, judged: false)), "Autopilot did not judge this")
    }

    // MARK: Waiting cards and history

    private func pending(_ action: String, count: UInt32 = 1, subtitle: String = "", service: String = "gmail", kind: PendingKind = .request) -> PendingItem {
        PendingItem(
            kind: kind, id: "r", title: "Pixel 9", subtitle: subtitle, createdAt: 0, connectionId: "c1", connectionLabel: "Claude",
            action: action, count: count, service: service, account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil,
            headline: "", quick: true
        )
    }

    func testAWaitingRequestIsOneSentenceWithItsKeyNounsStrong() {
        XCTAssertEqual(WantsText.plain(WantsText.of(pending("read", count: 2, subtitle: "2 messages found"))), "Wants to read 2 emails")
        XCTAssertEqual(WantsText.plain(WantsText.of(pending("search", subtitle: "invoice"))), "Wants to search Gmail for “invoice”")
        XCTAssertEqual(WantsText.plain(WantsText.of(pending("send", subtitle: "Hello"))), "Wants to send an email: “Hello”")
        XCTAssertEqual(WantsText.of(pending("read")).filter(\.strong).map(\.text), ["read an email"])
        XCTAssertEqual(WantsText.plain(WantsText.of(pending("join", kind: .join))), "Claude asks to join your account")
        XCTAssertEqual(WantsText.plain(WantsText.of(burst: [pending("read", count: 2), pending("read", count: 3)])), "Wants to read 5 emails")
        XCTAssertEqual(WantsText.plain(WantsText.of(burst: [pending("search"), pending("read")])), "Wants to make 2 requests")
    }

    func testAHistoryRowEndsWithWhoDecided() {
        let at: Int64 = 1_700_000_000
        XCTAssertEqual(EntryText.mark(DemoData.entry(1, at: at, action: "read", outcome: "released", detail: "")), .approved)
        XCTAssertEqual(EntryText.mark(DemoData.entry(2, at: at, action: "read", outcome: "denied", detail: "")), .denied)
        XCTAssertEqual(EntryText.mark(DemoData.entry(3, at: at, action: "read", outcome: "released", detail: "", decidedBy: "autopilot")), .auto)
        XCTAssertEqual(EntryText.mark(DemoData.entry(4, at: at, action: "read", outcome: "released", detail: "", grantId: "g1")), .grant)
        XCTAssertEqual(EntryText.mark(DemoData.entry(5, at: at, action: "send", outcome: "sent", detail: "", decidedBy: "bypass")), .bypass)
        let auto = DemoData.entry(6, at: at, action: "read", outcome: "released", detail: "", decidedBy: "autopilot", autopilot: DemoData.note(pApprove: 0.96))
        XCTAssertEqual(EntryOutcome.text(auto), "Allowed by Autopilot · 96%")
        XCTAssertEqual(EntryOutcome.whyTitle(auto), "Why Autopilot allowed it")
        let overnight = DemoData.entry(7, at: at, action: "write", outcome: "sent", detail: "", decidedBy: "overnight", autopilot: DemoData.note(mode: .overnight))
        XCTAssertEqual(EntryText.mark(overnight), .overnight)
        XCTAssertEqual(EntryOutcome.text(overnight), "Allowed by Overnight")
        XCTAssertEqual(EntryOutcome.symbol(overnight), "moon.stars.fill")
        XCTAssertEqual(EntryOutcome.whyTitle(overnight), "Why Autopilot allowed it")
        XCTAssertEqual(EntryOutcome.text(DemoData.entry(8, at: at, action: "write", outcome: "denied", detail: "", decidedBy: "overnight")), "Denied by Overnight")
    }

    func testTheHistoryListsARiskAboveLowOrOneOvernightHeld() {
        XCTAssertFalse(RiskLook.listed(nil), "no rating, nothing shown")
        XCTAssertFalse(RiskLook.listed(RiskView(level: .benign, pCatastrophic: 0, why: "", held: false)))
        XCTAssertTrue(RiskLook.listed(RiskView(level: .benign, pCatastrophic: 0, why: "", held: true)))
        XCTAssertTrue(RiskLook.listed(RiskView(level: .severe, pCatastrophic: 0.1, why: "force push", held: false)))
        XCTAssertEqual(ApprovalRecord.of(DemoData.activity(1_700_000_000).first { $0.id == 13 }!).risk?.level, .severe,
                       "an entry shown with its approval keeps the risk")
    }

    func testHistoryIsGroupedByDay() {
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date(timeIntervalSince1970: 1_700_000_000)).addingTimeInterval(12 * 3_600)
        let now = Int64(today.timeIntervalSince1970)
        let entries = [
            DemoData.entry(3, at: now - 60, action: "read", outcome: "released", detail: ""),
            DemoData.entry(2, at: now - 120, action: "read", outcome: "released", detail: ""),
            DemoData.entry(1, at: now - 86_400, action: "read", outcome: "released", detail: ""),
        ]
        let days = EntryText.days(entries)
        XCTAssertEqual(days.map { $0.entries.map(\.id) }, [[3, 2], [1]])
        XCTAssertEqual(EntryText.day(days[0].day, now: today), "Today")
        XCTAssertEqual(EntryText.day(days[1].day, now: today), "Yesterday")
        XCTAssertEqual(TimerPill.ago(now - 300, now: now), "5m")
    }
}
