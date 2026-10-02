import XCTest
@testable import Reins

final class TitlesTests: XCTestCase {
    func testOperationTitles() {
        XCTAssertEqual(operationTitle(action: "read", count: 3, service: "gmail"), "Read 3 emails")
        XCTAssertEqual(operationTitle(action: "send", count: 2, service: ""), "Send email to 2")
        XCTAssertEqual(fullTitle(label: "Claude", action: "search", count: 1, service: "gmail"), "Claude: Search Gmail")
    }

    func testUntrustedStripsBidiControls() {
        XCTAssertEqual(untrusted("a\u{202E}b\u{0007}c"), "ab c")
    }

    func testDeepLinksRoundTrip() {
        let link = DeepLink.item(kind: .request, id: "abc-123")
        XCTAssertEqual(DeepLink(url: link.url), link)
        XCTAssertNil(DeepLink(url: URL(string: "reins://item?kind=request&id=../x")!))
    }

    /// Calls name an MCP server `mcp:<id>`, its permissions `mcp_<id>` with `-` as `_`: both read as its name.
    func testMcpServersAreNamedInCallsAndPermissions() {
        McpNames.update(["my-notes": McpNames.Server(name: "Notes", tools: [:])])
        defer { McpNames.update([:]) }
        XCTAssertEqual(serviceName("mcp:my-notes"), "Notes")
        XCTAssertEqual(serviceName("mcp_my_notes"), "Notes")
        XCTAssertEqual(serviceName("mcp_gone"), "MCP server")
        XCTAssertEqual(serviceName("gmail"), "Gmail")
    }
}
