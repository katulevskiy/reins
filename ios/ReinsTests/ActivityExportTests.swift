import UniformTypeIdentifiers
import XCTest
@testable import Reins

/// What exporting the decision history says, names and writes.
final class ActivityExportTests: XCTestCase {
    func testTheCountSaysOneDecisionOrMany() {
        XCTAssertEqual(ActivityExportText.exported(1), "Exported 1 decision")
        XCTAssertEqual(ActivityExportText.exported(2), "Exported 2 decisions")
        XCTAssertEqual(ActivityExportText.exported(5000).replacingOccurrences(of: ",", with: ""), "Exported 5000 decisions")
    }

    func testEachFormatHasItsType() {
        XCTAssertEqual(ActivityExportText.contentType(.csv), .commaSeparatedText)
        XCTAssertEqual(ActivityExportText.contentType(.json), .json)
    }

    func testTheSuggestedNameLosesItsExtensionSoTheSheetDoesNotDoubleIt() {
        XCTAssertEqual(ActivityExportText.baseName("reins-decisions-2026-11-12.csv", format: .csv), "reins-decisions-2026-11-12")
        XCTAssertEqual(ActivityExportText.baseName("reins-decisions.JSON", format: .json), "reins-decisions")
        XCTAssertEqual(ActivityExportText.baseName("reins-decisions", format: .csv), "reins-decisions")
        XCTAssertEqual(ActivityExportText.baseName("a.csv", format: .json), "a.csv", "only the format's own extension goes")
        XCTAssertEqual(ActivityExportText.baseName(".csv", format: .csv), ".csv", "a name that is only an extension stays")
        XCTAssertEqual(ActivityExportText.baseName("  ", format: .csv), "reins-decisions")
    }

    func testTheDemoCoreExportsWhatTheRowSaves() async throws {
        let core = DemoReinsCore()
        let export = try await core.exportActivity(format: .csv, query: nil)
        XCTAssertEqual(ActivityExportText.baseName(export.fileName, format: .csv), "reins-decisions")
        XCTAssertEqual(export.contents.components(separatedBy: "\r\n").filter { !$0.isEmpty }.count, Int(export.entries) + 1, "a header and a row each")
        let json = try await core.exportActivity(format: .json, query: nil)
        XCTAssertEqual(json.entries, export.entries)
    }

    func testTheDocumentWritesUTF8() throws {
        let doc = ActivityExportDocument(contents: "id,name\r\n1,Zoë\r\n", type: .commaSeparatedText)
        XCTAssertEqual(String(data: Data(doc.contents.utf8), encoding: .utf8), "id,name\r\n1,Zoë\r\n")
        XCTAssertTrue(ActivityExportDocument.readableContentTypes.contains(.json))
    }
}
