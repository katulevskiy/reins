import SwiftUI
import XCTest
@testable import Reins

final class AvatarTests: XCTestCase {
    func testANameSuggestsItsProvider() {
        XCTAssertEqual(Providers.infer("My Claude")?.key, "claude")
        XCTAssertEqual(Providers.infer("ChatGPT at work")?.key, "openai")
        XCTAssertEqual(Providers.infer("Le Chat")?.key, "mistral")
        XCTAssertEqual(Providers.infer("llama box")?.key, "meta")
        XCTAssertNil(Providers.infer("Build bot"))
    }

    func testAPickWinsAndBlobKeepsTheBlobatar() {
        XCTAssertEqual(Providers.resolve(label: "My Claude", pick: nil)?.key, "claude")
        XCTAssertEqual(Providers.resolve(label: "My Claude", pick: "gemini")?.key, "gemini")
        XCTAssertNil(Providers.resolve(label: "My Claude", pick: Providers.blob))
        XCTAssertNil(Providers.resolve(label: "My Claude", pick: "gone"), "an unknown pick draws the blobatar")
    }

    func testEveryProviderAndServiceHasItsLogo() {
        XCTAssertEqual(Set(Providers.all.map(\.key)).count, Providers.all.count)
        for p in Providers.all { XCTAssertNotNil(UIImage(named: p.asset), p.key) }
        for s in ["gmail", "telegram", "github", "gitlab", "codeberg", "bitbucket", "mcp", "gcalendar", "gcontacts",
                  "device_calendar", "device_contacts", "sms", "vault"] {
            XCTAssertNotNil(UIImage(named: "service-\(s)"), s)
        }
    }

    func testTheBlobatarDrawsInsideItsBox() {
        for name in ["alain", "Claude Desktop", "x", "Team Rocket 3", " "] {
            let marks = BlobatarDrawing.marks(for: name)
            XCTAssertGreaterThanOrEqual(marks.count, 3, name)
            for mark in marks {
                let box = mark.path.boundingRect
                XCTAssertFalse(box.isEmpty, name)
                XCTAssertTrue(CGRect(x: -5, y: -5, width: 110, height: 110).contains(box), "\(name) \(box)")
            }
        }
    }

    func testThePathParserReadsTheRendererCommands() {
        let lines = SVGPath.parse("M10 20H30V40H10ZM50 50L60 -5.5L80 80Z").boundingRect
        XCTAssertEqual(lines.minX, 10, accuracy: 0.01)
        XCTAssertEqual(lines.minY, -5.5, accuracy: 0.01)
        XCTAssertEqual(lines.maxX, 80, accuracy: 0.01)
        XCTAssertEqual(lines.maxY, 80, accuracy: 0.01)
        // Curves end where they say; their control points pull them, not the end points.
        let curves = SVGPath.parse("M0 0Q50 100 100 0C100 50 0 50 0 0Z")
        XCTAssertEqual(curves.currentPoint?.x ?? -1, 0, accuracy: 0.01)
        XCTAssertEqual(curves.boundingRect.maxX, 100, accuracy: 0.01)
        XCTAssertEqual(curves.boundingRect.maxY, 50, accuracy: 0.5)
    }
}
