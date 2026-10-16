import XCTest
@testable import Reins

/// Activity's days and search: the river grouped by local day across midnight, time zones and daylight saving, each
/// day's count, the words set apart in a title (cleaned first, word starts only), and the query the filters make.
final class ActivitySearchTests: XCTestCase {
    private func calendar(_ zone: String) -> Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: zone)!
        return c
    }

    private func at(_ c: Calendar, _ y: Int, _ m: Int, _ d: Int, _ h: Int = 0, _ min: Int = 0, _ s: Int = 0) -> Int64 {
        Int64(c.date(from: DateComponents(year: y, month: m, day: d, hour: h, minute: min, second: s))!.timeIntervalSince1970)
    }

    private func stamp(_ id: Int64, _ at: Int64, _ outcome: ActivityOutcome = .approved) -> ActivityStamp {
        ActivityStamp(id: id, at: at, outcome: outcome, decider: .you)
    }

    // MARK: Days

    func testAnEntryAtOneSecondToMidnightIsTheDayBefore() {
        let ny = calendar("America/New_York")
        let days = ActivityDays.group([
            stamp(4, at(ny, 2026, 3, 9, 0, 0, 0)),
            stamp(3, at(ny, 2026, 3, 8, 23, 59, 59)),
            stamp(2, at(ny, 2026, 3, 8, 0, 0, 0)),
            stamp(1, at(ny, 2026, 3, 7, 23, 59, 59)),
        ], calendar: ny)
        XCTAssertEqual(days.map(\.key), ["2026-03-09", "2026-03-08", "2026-03-07"], "newest day first")
        XCTAssertEqual(days.map(\.count), [1, 2, 1])
    }

    func testADaylightSavingDayIsTwentyThreeOrTwentyFiveHours() {
        let ny = calendar("America/New_York")
        // Clocks go forward on 2026-03-08 and back on 2026-11-01.
        let spring = ActivityDays.group([stamp(1, at(ny, 2026, 3, 8, 12))], calendar: ny)[0]
        XCTAssertEqual(spring.key, "2026-03-08")
        XCTAssertEqual(spring.until - spring.since, 23 * 3_600)
        XCTAssertEqual(spring.since, at(ny, 2026, 3, 8))
        XCTAssertEqual(spring.until, at(ny, 2026, 3, 9))
        let autumn = ActivityDays.group([stamp(1, at(ny, 2026, 11, 1, 12))], calendar: ny)[0]
        XCTAssertEqual(autumn.until - autumn.since, 25 * 3_600)
        // An entry after the jump still belongs to the day it happened on, and the next midnight starts the next.
        XCTAssertTrue(spring.holds(at(ny, 2026, 3, 8, 3, 30)))
        XCTAssertFalse(spring.holds(at(ny, 2026, 3, 9)))
        XCTAssertTrue(spring.holds(at(ny, 2026, 3, 8, 23, 59, 59)))
    }

    func testDaysAreTheLocalDayOfTheTimeZone() {
        let kolkata = calendar("Asia/Kolkata")
        let utc = calendar("UTC")
        // 18:29:59 and 18:30:00 UTC are 23:59:59 and midnight in India (UTC+5:30).
        let before = at(utc, 2026, 10, 14, 18, 29, 59)
        let after = at(utc, 2026, 10, 14, 18, 30, 0)
        let india = ActivityDays.group([stamp(2, after), stamp(1, before)], calendar: kolkata)
        XCTAssertEqual(india.map(\.key), ["2026-10-15", "2026-10-14"])
        XCTAssertEqual(india[0].since, after, "the Indian day starts at 18:30 UTC")
        XCTAssertEqual(india[0].until - india[0].since, 86_400)
        let london = ActivityDays.group([stamp(2, after), stamp(1, before)], calendar: utc)
        XCTAssertEqual(london.map(\.key), ["2026-10-14"])
        XCTAssertEqual(london[0].count, 2)
    }

    func testTheDaySaysHowManyAndHowManyWereDeniedOrFailed() {
        let utc = calendar("UTC")
        let noon = at(utc, 2026, 10, 15, 12)
        var stamps = (1...11).map { stamp(Int64($0), noon) }
        stamps += [stamp(12, noon, .denied), stamp(13, noon, .denied), stamp(14, noon, .failed)]
        let day = ActivityDays.group(stamps, calendar: utc)[0]
        XCTAssertEqual(ActivityDays.summary(day), "14 · 2 denied · 1 failed")
        XCTAssertEqual(ActivityDays.summary(ActivityDay(key: "", start: .now, end: .now, count: 14, denied: 2)), "14 · 2 denied")
        XCTAssertEqual(ActivityDays.summary(ActivityDay(key: "", start: .now, end: .now, count: 3, failed: 1)), "3 · 1 failed")
        XCTAssertEqual(ActivityDays.summary(ActivityDay(key: "", start: .now, end: .now, count: 14)), "14")
        XCTAssertEqual(ActivityDays.spoken("Today", day), "Today, 14 entries, 2 denied, 1 failed")
    }

    func testAnEntryIsStampedAsTheCoreStampsIt() {
        let auto = DemoData.entry(1, at: 0, action: "read", outcome: "released", detail: "", decidedBy: "overnight")
        XCTAssertEqual(ActivityDays.stamp(auto).decider, .autopilot, "Overnight counts as Autopilot")
        XCTAssertEqual(ActivityDays.stamp(auto).outcome, .approved)
        let failed = DemoData.entry(2, at: 0, action: "read", outcome: "error", detail: "", grantId: "g1")
        XCTAssertEqual(ActivityDays.stamp(failed).outcome, .failed)
        XCTAssertEqual(ActivityDays.stamp(failed).decider, .you, "a grant counts as you")
    }

    func testADaysRowsAreItsPagesAndWhatIsLoadedNewestFirstOnceEach() {
        let utc = calendar("UTC")
        let day = ActivityDays.group([stamp(1, at(utc, 2026, 10, 15, 12))], calendar: utc)[0]
        let noon = at(utc, 2026, 10, 15, 12)
        let pages = [
            DemoData.entry(3, at: noon + 60, action: "read", outcome: "released", detail: ""),
            DemoData.entry(2, at: noon, action: "read", outcome: "released", detail: ""),
        ]
        let recent = [
            DemoData.entry(5, at: day.until, action: "read", outcome: "released", detail: ""),
            DemoData.entry(4, at: noon + 120, action: "read", outcome: "denied", detail: ""),
            DemoData.entry(3, at: noon + 60, action: "read", outcome: "released", detail: ""),
        ]
        XCTAssertEqual(ActivityDays.rows(day, pages: pages, recent: recent).map(\.id), [4, 3, 2], "the next day's entry is not this day's")
    }

    func testTheDaysOfWhatIsLoadedFillInADayTheStampsDoNotHaveYet() {
        let utc = calendar("UTC")
        let yesterday = ActivityDays.group([stamp(1, at(utc, 2026, 10, 14, 12))], calendar: utc)
        let recent = [DemoData.entry(2, at: at(utc, 2026, 10, 15, 0, 5), action: "read", outcome: "released", detail: "")]
        XCTAssertEqual(ActivityDays.merged(yesterday, recent: recent, calendar: utc).map(\.key), ["2026-10-15", "2026-10-14"])
        XCTAssertEqual(ActivityDays.merged(nil, recent: recent, calendar: utc).map(\.key), ["2026-10-15"])
    }

    // MARK: Highlights

    func testOnlyTheStartOfAWordMatches() {
        XCTAssertEqual(TitleHighlight.matches("Read 2 emails from Stripe", terms: ["str"]), ["Str"])
        XCTAssertEqual(TitleHighlight.matches("Destroy the branch", terms: ["str"]), [], "never inside a word")
        XCTAssertEqual(TitleHighlight.matches("reinvoice invoice", terms: ["inv"]), ["inv"])
        XCTAssertEqual(TitleHighlight.matches("INVOICE", terms: ["invoice"]), ["INVOICE"], "case apart")
        XCTAssertEqual(TitleHighlight.matches("Searched for invoices", terms: ["in", "invoice"]), ["invoice"], "the longest term wins")
        XCTAssertEqual(TitleHighlight.matches("Pushed 3 commits", terms: []), [])
    }

    func testMatchesApartByPunctuationJoin() {
        XCTAssertEqual(TitleHighlight.matches("Send to alice@example.com", terms: ["alice", "example", "com"]), ["alice@example.com"])
        XCTAssertEqual(TitleHighlight.matches("Pushed to octo/app", terms: ["octo", "app"]), ["octo/app"])
        XCTAssertEqual(TitleHighlight.matches("Read notes.txt", terms: ["notes", "txt"]), ["notes.txt"])
        XCTAssertEqual(TitleHighlight.matches("feature/-/login", terms: ["feature", "login"]), ["feature", "login"], "three apart stay apart")
        XCTAssertEqual(TitleHighlight.matches("octo app", terms: ["octo", "app"]), ["octo", "app"], "a space never joins")
    }

    func testHiddenCharactersNeitherMakeNorShiftAMatch() {
        // Without cleaning, the zero-width space or the soft hyphen would make "pal" and "voice" start a word.
        XCTAssertEqual(TitleHighlight.matches("Pay\u{200B}pal invoice", terms: ["pal"]), [])
        XCTAssertEqual(TitleHighlight.matches("Pay\u{200B}pal invoice", terms: ["paypal"]), ["Paypal"])
        XCTAssertEqual(TitleHighlight.matches("Read in\u{00AD}voice", terms: ["voice"]), [])
        XCTAssertEqual(TitleHighlight.matches("evil\u{202E}moc.elgoog", terms: ["moc"]), [])
        // Removed before the title is read, so the match lands where the shown title has it.
        let raw = "\u{202E}\u{200B}Invoice due"
        let shown = untrusted(raw)
        XCTAssertEqual(shown, "Invoice due")
        XCTAssertEqual(TitleHighlight.hits(shown, terms: ["due"]), [8..<11])
        XCTAssertEqual(TitleHighlight.matches(raw, terms: ["due"]), ["due"])
    }

    @MainActor
    func testARowsTitleSetsApartItsMatchesAcrossTheSlashJoiner() {
        let runs = [AskText.Run(text: "Pushed 3 commits to "), AskText.Run(text: "acme/\u{2060}web", target: true)]
        let title = AskRunsText.attributed(runs, size: 18, terms: ["acme", "web"])
        XCTAssertEqual(TitleHighlight.marked(title), ["acme/\u{2060}web"])
        XCTAssertEqual(TitleHighlight.marked(AskRunsText.attributed(runs, size: 18, terms: ["web"])), ["web"])
        XCTAssertEqual(TitleHighlight.marked(AskRunsText.attributed(runs, size: 18, terms: ["ommits"])), [])
        let label = DemoData.requestLabel("gmail", "search", "Search Gmail for", "in\u{200B}voice")
        XCTAssertEqual(TitleHighlight.marked(LabelText.attributed(label, size: 15, color: .primary, terms: ["voice"])), [])
        XCTAssertEqual(TitleHighlight.marked(LabelText.attributed(label, size: 15, color: .primary, terms: ["invoice"])), ["invoice"])
    }

    // MARK: Query

    func testTheFiltersMakeTheQuery() {
        let ny = calendar("America/New_York")
        let now = Date(timeIntervalSince1970: TimeInterval(at(ny, 2026, 3, 8, 15)))
        var f = ActivityFilters()
        XCTAssertTrue(f.isEmpty)
        XCTAssertEqual(f.query(text: "  invoice  ", now: now, calendar: ny), ActivityQuery(text: "invoice"))
        f.toggle(.denied)
        f.toggle(.approved)
        XCTAssertEqual(f.outcomes, [.approved, .denied], "in the menu's order")
        f.toggle(.approved)
        XCTAssertEqual(f.outcomes, [.denied], "a second pick takes it off")
        f.toggle(.autopilot)
        f.toggle(service: "github")
        f.toggle(action: "send")
        f.toggle(connection: "c2")
        f.toggle(connection: "c1")
        f.when = .today
        XCTAssertFalse(f.isEmpty)
        XCTAssertEqual(
            f.query(text: "", now: now, calendar: ny),
            ActivityQuery(
                text: "", outcomes: [.denied], deciders: [.autopilot], services: ["github"], actions: ["send"], connections: ["c1", "c2"],
                since: at(ny, 2026, 3, 8), until: nil
            )
        )
        f.when = .week
        XCTAssertEqual(f.query(text: "", now: now, calendar: ny).since, Int64(now.timeIntervalSince1970) - 7 * 86_400)
        f.when = .month
        XCTAssertEqual(f.query(text: "", now: now, calendar: ny).since, Int64(now.timeIntervalSince1970) - 30 * 86_400)
        f.when = .any
        XCTAssertNil(f.query(text: "", now: now, calendar: ny).since)
    }

    func testSearchingIsWordsOrAFilter() {
        XCTAssertFalse(ActivityFilters.searching(text: "", filters: ActivityFilters()))
        XCTAssertFalse(ActivityFilters.searching(text: "   ", filters: ActivityFilters()))
        XCTAssertTrue(ActivityFilters.searching(text: "inv", filters: ActivityFilters()))
        XCTAssertTrue(ActivityFilters.searching(text: "", filters: ActivityFilters(when: .week)))
        // The old filters' equivalents: Autopilot is Who, Denied is Outcome; their menu rows keep the old ids.
        XCTAssertEqual(ActivitySearchText.id(ActivityDecider.autopilot), "filter:autopilot")
        XCTAssertEqual(ActivitySearchText.id(ActivityOutcome.denied), "filter:denied")
    }

    func testTheChipsSayWhatIsChosen() {
        XCTAssertEqual(ActivitySearchText.chip("Outcome", []), "Outcome")
        XCTAssertEqual(ActivitySearchText.chip("Outcome", ["Denied"]), "Denied")
        XCTAssertEqual(ActivitySearchText.chip("Outcome", ["Denied", "Failed"]), "Denied +1")
        XCTAssertEqual(ActivitySearchText.kind("send"), "Send")
        XCTAssertEqual(ActivitySearchText.kind("grant"), "Permission")
        XCTAssertEqual(ActivitySearchText.field(.file), "FILE")
        XCTAssertEqual(ActivitySearchText.span(" found in "), " found in ")
        XCTAssertEqual(ActivitySearchText.span("a\u{202E}b"), "ab")
    }

    // MARK: The demo core

    func testTheDemoSearchesItsHistory() async throws {
        let core = DemoReinsCore(quiet: false)
        let all = try await core.activityStamps(query: ActivityQuery())
        XCTAssertFalse(all.isEmpty)
        let denied = try await core.searchActivity(query: ActivityQuery(outcomes: [.denied]), after: nil, limit: 2)
        XCTAssertTrue(denied.hits.allSatisfy { $0.entry.outcome == "denied" })
        XCTAssertEqual(denied.hits.count, min(2, Int(denied.total)))
        if let next = denied.next {
            let more = try await core.searchActivity(query: ActivityQuery(outcomes: [.denied]), after: next, limit: 50)
            XCTAssertTrue(Set(more.hits.map(\.entry.id)).isDisjoint(with: denied.hits.map(\.entry.id)))
        }
        let words = try await core.searchActivity(query: ActivityQuery(text: "Dinner"), after: nil, limit: 50)
        XCTAssertEqual(words.terms, ["dinner"])
        XCTAssertFalse(words.hits.isEmpty)
        let facets = try await core.activityFacets()
        XCTAssertTrue(facets.services.contains("gmail"))
    }
}
