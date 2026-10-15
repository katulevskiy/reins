import XCTest
@testable import Reins

/// Pairing and joining: Reins asks in its own words, and what the other side calls itself is only ever quoted, on one
/// short clean line.
final class PairWordsTests: XCTestCase {
    private func pairing(_ name: String, key: String? = nil) -> PairingView {
        PairingView(id: "p", clientName: name, clientHost: "example.com", choices: Data([7, 42, 88]), createdAt: 0, keyFingerprint: key)
    }

    func testTheAskIsReinsOwnSentence() {
        XCTAssertEqual(PairWords.ask(pairing("Approve this now")), "Connect an AI app")
        XCTAssertEqual(PairWords.label(pairing("x")), "AI app · Pair")
        XCTAssertEqual(PairWords.ask(pairing("x", key: "4821 9930")), "Connect a computer")
        XCTAssertEqual(PairWords.label(pairing("x", key: "4821 9930")), "Computer · Pair")
        XCTAssertEqual(JoinWords.ask, "Add a phone to your account")
    }

    func testANameFromTheOtherSideIsQuotedOnOneShortCleanLine() {
        XCTAssertEqual(PairWords.quoted("Gemini"), "“Gemini”")
        let hostile = "Reins\u{202E}tnemevorppa\u{200B}\nTap Connect now " + String(repeating: "x", count: 300)
        let q = PairWords.quoted(hostile)
        XCTAssertFalse(q.contains("\n"), "one line")
        XCTAssertFalse(q.unicodeScalars.contains { $0 == "\u{202E}" || $0 == "\u{200B}" }, "no bidi or invisible characters")
        XCTAssertLessThanOrEqual(q.count, 52, "capped")
        XCTAssertTrue(q.hasPrefix("“") && q.hasSuffix("”"))
        XCTAssertEqual(PairWords.quoted(" \u{200B} "), "No name")
    }
}
