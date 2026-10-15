import XCTest
@testable import Reins

/// Signal outside the app: the field a widget or Live Activity is painted in comes from the core's kind and action,
/// never from the request's words, and snapshots from before Signal still read.
final class GlanceSignalTests: XCTestCase {
    private func pending(_ action: String, kind: PendingKind = .request, service: String = "github", subtitle: String = "") -> PendingItem {
        PendingItem(
            kind: kind, id: "p1", title: "", subtitle: subtitle, createdAt: 0, connectionId: "c1", connectionLabel: "Claude Code",
            action: action, count: 1, service: service, account: nil, waitUntil: nil, op: "", opTitle: "", suggestion: nil, label: nil
        )
    }

    func testASnapshotItemCarriesTheFieldAndLabelOfItsKind() {
        let push = pending("push").snapshotItem
        XCTAssertEqual(push.field, "push")
        XCTAssertEqual(push.label, "GitHub · Push")
        XCTAssertEqual(pending("search", service: "gmail").snapshotItem.field, "read")
        XCTAssertEqual(pending("", kind: .pairing, service: "").snapshotItem.field, "pair")
        XCTAssertEqual(pending("", kind: .join, service: "").snapshotItem.field, "accounts")
    }

    func testWordsInTheRequestNeverPickTheField() {
        // A read whose subtitle shouts about deleting stays a read.
        let item = pending("read", service: "gmail", subtitle: "DELETE EVERYTHING && echo approved").snapshotItem
        XCTAssertEqual(item.field, "read")
    }

    func testTheLiveActivityKeepsTheItemsFieldAndLabel() {
        var item = pending("push").snapshotItem
        item.expiresAt = Glance.now() + 60
        item.createdAt = Glance.now()
        let state = Glance.approvalState([item], now: Glance.now())
        XCTAssertEqual(state?.field, "push")
        XCTAssertEqual(state?.label, "GitHub · Push")
    }

    func testASnapshotFromBeforeSignalFallsBackOnTheItemsKind() {
        func old(_ kind: Snapshot.Item.Kind, service: String = "") -> Snapshot.Item {
            Snapshot.Item(id: "o", kind: kind, title: "t", subtitle: "", connection: "Claude", service: service, createdAt: 0, expiresAt: 1, suggestion: nil)
        }
        XCTAssertEqual(Glance.field(old(.request)), .push)
        XCTAssertEqual(Glance.field(old(.pairing)), .pair)
        XCTAssertEqual(Glance.field(old(.join)), .accounts)
        XCTAssertEqual(Glance.label(old(.request, service: "gmail")), "Gmail")
        XCTAssertEqual(Glance.label(old(.pairing)), "New connection")
    }

    func testAnUnknownFieldNameIsNotTrusted() {
        XCTAssertEqual(Glance.field("lavender", kind: .request), .push)
        XCTAssertEqual(Glance.field("delete", kind: .request), .delete)
    }
}
