import CryptoKit
import XCTest
@testable import Reins

/// The port must draw exactly what blobatar draws. These are blobatar's own golden fixtures (the Android app's
/// `BlobatarGoldenTest` reads the same files): every default seed of its 1000-name corpus plus its backdrop variants,
/// checked by the same SHA-256 tripwire the library uses, and a few complete markups so a failure reads.
final class BlobatarGoldenTests: XCTestCase {
    private func lines(_ name: String) throws -> [[String]] {
        let file = name + ".tsv"
        let url = Bundle(for: Self.self).url(forResource: name, withExtension: "tsv")
            ?? URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Fixtures/\(file)")
        let text = try String(contentsOf: url, encoding: .utf8)
        return text.split(separator: "\n", omittingEmptySubsequences: true)
            .filter { !$0.hasPrefix("#") }
            .map { $0.split(separator: "\t", omittingEmptySubsequences: false).map(String.init) }
    }

    private func backdrop(_ label: String) -> Blobatar.Backdrop {
        switch label {
        case "", "bg:none": .none
        case "bg:square", "square": .square
        case "bg:circle": .circle
        case "bg:squircle", "squircle": .squircle
        default: .none
        }
    }

    private func sha16(_ text: String) -> String {
        SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined().prefix(16).description
    }

    func testThePortRendersTheLibraryMarkupForItsWholeGoldenCorpus() throws {
        let rows = try lines("blobatar-gen2-hashes")
        XCTAssertGreaterThanOrEqual(rows.count, 1000, "fixture looks truncated")
        let wrong = rows.filter { $0.count >= 3 && sha16(Blobatar.svg($0[0], backdrop: backdrop($0[1]))) != $0[2] }
        XCTAssertEqual(wrong.prefix(10).map { "\($0[0])|\($0[1])" }, [], "names whose blobatar differs from the library's (\(wrong.count))")
    }

    func testCompleteMarkupsMatchByteForByte() throws {
        for row in try lines("blobatar-gen2-markup") where row.count >= 3 {
            XCTAssertEqual(Blobatar.svg(row[0], backdrop: backdrop(row[1])), row[2], "\(row[0])/\(row[1])")
        }
    }

    func testNamesAreNormalisedLikeTheLibraryDoes() {
        XCTAssertEqual(Blobatar.svg("alain@x.com"), Blobatar.svg("  Alain@X.com "))
        XCTAssertEqual(Blobatar.svg("caf\u{E9}"), Blobatar.svg("cafe\u{301}"))
    }

    func testJavaScriptRounding() {
        XCTAssertEqual(Blobatar.jsRound(2.5), 3)
        XCTAssertEqual(Blobatar.jsRound(-2.5), -2)
        XCTAssertEqual(Blobatar.jsRound(0.49999999999999994), 0)
        XCTAssertEqual(Blobatar.num(-0.004), "0")
        XCTAssertEqual(Blobatar.num(12.5), "12.5")
        XCTAssertEqual(Blobatar.num(-3.07), "-3.07")
        XCTAssertEqual(Blobatar.num(40), "40")
    }
}
