import SwiftUI
import XCTest
@testable import Reins

/// How a connection looks: the shared blobatar palette, the port's shape and colour overrides, what Auto shows (a
/// computer's platform, an AI's provider, else the blobatar), changing the blobatar from the Icon sheet, and the
/// Activity line that names the agent and its computer.
@MainActor
final class ConnectionLookTests: XCTestCase {
    private func connection(
        _ label: String, icon: String? = nil, computer: Bool = false, autoIcon: String = "", platform: String? = nil,
        blobatar: Blobatar? = nil
    ) -> ConnectionView {
        ConnectionView(
            id: "c-\(label)", label: label, clientHost: "127.0.0.1", createdAt: 0, lastUsedAt: nil, icon: icon,
            keyFingerprint: computer ? "1234 5678" : nil, autoIcon: autoIcon, platform: platform, blobatar: blobatar
        )
    }

    // MARK: The palette

    private struct Fixture: Decodable {
        struct Colour: Decodable {
            let name: String
            let l: Double
            let c: Double
            let h: Double
        }

        let colours: [Colour]
        let shapes: [String]
    }

    /// The core, Android and this app keep a blobatar's colour and shape by index: all three check their copies against
    /// one file.
    func testThePaletteIsTheSharedOne() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("design/signal/tokens/blobatar-palette.json")
        let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
        XCTAssertEqual(BlobatarPalette.colours.map(\.name), fixture.colours.map(\.name))
        XCTAssertEqual(BlobatarPalette.colours.map(\.colour), fixture.colours.map { BlobatarArt.Colour(l: $0.l, c: $0.c, h: $0.h) })
        XCTAssertEqual(BlobatarPalette.shapes, fixture.shapes)
        XCTAssertEqual(BlobatarArt.shapeNames, fixture.shapes, "shape i is the library's i-th band")
        XCTAssertEqual(BlobatarPalette.colours[0].title, "Red")
    }

    // MARK: Overrides

    func testWithoutOverridesTheFigureIsTheLibrarys() {
        for seed in ["alain", "c4", "M2A"] {
            XCTAssertEqual(BlobatarArt.svg(seed, shape: nil, colour: nil), BlobatarArt.svg(seed))
        }
    }

    func testAShapeOverrideChangesOnlyTheBody() {
        let seed = "notes-bot"
        let shapes = Set(BlobatarPalette.shapes.indices.map { BlobatarArt.svg(seed, shape: $0) })
        XCTAssertEqual(shapes.count, BlobatarPalette.shapes.count, "ten different bodies")
        let plain = BlobatarArt.figure(seed)
        for i in BlobatarPalette.shapes.indices {
            let shaped = BlobatarArt.figure(seed, shape: i)
            XCTAssertEqual(shaped.marks.first?.fill, plain.marks.first?.fill, "the seed's colour stays")
        }
        XCTAssertEqual(BlobatarArt.svg(seed, shape: 99), BlobatarArt.svg(seed, shape: BlobatarPalette.shapes.count - 1), "out of range is clamped")
    }

    func testAColourOverrideChangesOnlyTheColours() throws {
        let seed = "notes-bot"
        let plain = BlobatarArt.figure(seed)
        let heads = Set(BlobatarPalette.colours.map { BlobatarArt.figure(seed, colour: $0.colour).marks.first?.fill ?? "" })
        XCTAssertEqual(heads.count, BlobatarPalette.colours.count, "twelve different heads")
        let red = try XCTUnwrap(BlobatarPalette.colour(0))
        let tinted = BlobatarArt.figure(seed, colour: red)
        XCTAssertEqual(tinted.marks.first?.fill, BlobatarArt.headHex(red))
        XCTAssertEqual(tinted.marks.map(markGeometry), plain.marks.map(markGeometry), "same figure, other paint")
    }

    func testTheDrawingIsKeptPerSeedAndOverrides() {
        let plain = BlobatarDrawing.marks(for: "c4")
        let shaped = BlobatarDrawing.marks(for: "c4", shape: 7, colour: 2)
        XCTAssertNotEqual(plain.map(\.path.boundingRect), shaped.map(\.path.boundingRect))
        XCTAssertEqual(BlobatarDrawing.marks(for: "c4", shape: 7, colour: 2).map(\.path.boundingRect), shaped.map(\.path.boundingRect))
        XCTAssertEqual(BlobatarDrawing.marks(for: "c4", shape: 40, colour: 40).map(\.path.boundingRect), plain.map(\.path.boundingRect),
                       "an index out of range draws the seed's own")
    }

    private func markGeometry(_ mark: BlobatarArt.Mark) -> String {
        switch mark {
        case let .path(d, _): d
        case let .circle(cx, cy, r, _): "\(cx) \(cy) \(r)"
        }
    }

    // MARK: What it shows

    func testTheIconPickedWinsThenAutoThenTheOlderRule() {
        let kept = Blobatar(seed: "k", shape: 3, colour: 5)
        // Picked.
        XCTAssertEqual(ConnectionLook.look(connection("Claude", icon: "grok", autoIcon: "claude")), .provider(Providers.byKey("grok")!))
        XCTAssertEqual(ConnectionLook.look(connection("M2A", icon: "blob", computer: true, autoIcon: "macos", blobatar: kept)),
                       .blob(seed: "k", shape: 3, colour: 5))
        XCTAssertEqual(ConnectionLook.look(connection("Claude", icon: "linux")), .platform(Platforms.byKey("linux")!))
        // Auto, as the core works it out.
        XCTAssertEqual(ConnectionLook.look(connection("Claude", autoIcon: "claude")), .provider(Providers.byKey("claude")!))
        XCTAssertEqual(ConnectionLook.look(connection("notes-bot", autoIcon: "blob", blobatar: kept)), .blob(seed: "k", shape: 3, colour: 5))
        XCTAssertEqual(ConnectionLook.look(connection("Claude box", autoIcon: "blob")), .blob(seed: "Claude box", shape: nil, colour: nil),
                       "the core's Auto, not the name, decides")
        // Older data without an Auto: the provider the name suggests, else a blobatar seeded by the name.
        XCTAssertEqual(ConnectionLook.look(connection("My Claude")), .provider(Providers.byKey("claude")!))
        XCTAssertEqual(ConnectionLook.look(connection("notes-bot")), .blob(seed: "notes-bot", shape: nil, colour: nil))
        XCTAssertEqual(ConnectionLook.look(connection("x", icon: "gone")), .blob(seed: "x", shape: nil, colour: nil), "an unknown pick draws the blobatar")
    }

    func testAComputerShowsItsPlatform() {
        let mac = connection("M2A", computer: true, autoIcon: "macos", platform: "macos")
        XCTAssertEqual(ConnectionLook.look(mac), .platform(Platforms.byKey("macos")!))
        XCTAssertEqual(ConnectionLook.look(connection("honeypot", computer: true, autoIcon: "linux", platform: "linux")), .platform(Platforms.byKey("linux")!))
        XCTAssertEqual(ConnectionLook.look(connection("DESKTOP-1", computer: true, autoIcon: "windows", platform: "windows")), .platform(Platforms.byKey("windows")!))
        XCTAssertEqual(Platforms.byKey("windows")?.brand, 0x0078D4)
        XCTAssertNil(Platforms.byKey("macos")?.brand, "single-colour marks take the text colour")
        XCTAssertFalse(ConnectionLook.showsBlob(mac))
        XCTAssertEqual(ConnectionLook.auto(mac), .platform(Platforms.byKey("macos")!))
        XCTAssertEqual(ConnectionLook.blob(connection("M2A", computer: true, autoIcon: "macos", blobatar: Blobatar(seed: "s", shape: 1, colour: 2))),
                       .blob(seed: "s", shape: 1, colour: 2))
        for p in Platforms.all { XCTAssertNotNil(UIImage(named: p.asset), p.key) }
    }

    func testTheDemoConnectionsLookDistinct() {
        let all = DemoData.connections(0)
        let looks = Dictionary(uniqueKeysWithValues: all.map { ($0.label, ConnectionLook.look($0)) })
        XCTAssertEqual(looks["M2A"], .platform(Platforms.byKey("macos")!))
        XCTAssertEqual(looks["honeypot"], .platform(Platforms.byKey("linux")!))
        XCTAssertEqual(looks["Claude"], .provider(Providers.byKey("claude")!))
        XCTAssertEqual(looks["Grok"], .provider(Providers.byKey("grok")!))
        XCTAssertTrue(looks["notes-bot"]?.isBlob ?? false)
        let colours = all.compactMap { $0.blobatar?.colour }
        XCTAssertEqual(colours.count, all.count)
        XCTAssertEqual(Set(colours).count, colours.count, "every connection a colour of its own")
    }

    // MARK: Changing the blobatar

    private func model() async -> AppModel {
        let model = AppModel(core: DemoReinsCore(signedIn: true, syncCap: 0.3), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        await model.refreshSession()
        await model.refreshConnections()
        return model
    }

    func testPickingAShapeOrAColourReachesTheCoreAndStays() async throws {
        let model = await model()
        let before = try XCTUnwrap(model.connection("c4")?.blobatar)
        try await model.changeBlobatar("c4", .shape(7))
        XCTAssertEqual(model.connection("c4")?.blobatar, Blobatar(seed: before.seed, shape: 7, colour: before.colour), "the seed is kept")
        try await model.changeBlobatar("c4", .colour(2))
        XCTAssertEqual(model.connection("c4")?.blobatar, Blobatar(seed: before.seed, shape: 7, colour: 2))
        let kept = try await model.core.connections().first { $0.id == "c4" }
        XCTAssertEqual(kept?.blobatar, Blobatar(seed: before.seed, shape: 7, colour: 2), "kept by the core")
        XCTAssertNil(kept?.icon, "it already showed its blobatar: Auto stays")
    }

    func testShuffleChangesTheSeedTheShapeAndTheColour() async throws {
        let model = await model()
        let before = try XCTUnwrap(model.connection("c4")?.blobatar)
        try await model.changeBlobatar("c4", .shuffle)
        let after = try XCTUnwrap(model.connection("c4")?.blobatar)
        XCTAssertNotEqual(after.seed, before.seed)
        XCTAssertNotEqual(after.shape, before.shape)
        XCTAssertNotEqual(after.colour, before.colour)
        let others = Set(model.connections.filter { $0.id != "c4" }.compactMap { $0.blobatar?.colour })
        XCTAssertFalse(others.contains(after.colour), "a colour no other connection has, while one is free")
    }

    func testChangingTheBlobatarOfALogoMakesItShowTheBlobatar() async throws {
        let model = await model()
        XCTAssertEqual(ConnectionLook.look(try XCTUnwrap(model.connection("c1"))), .provider(Providers.byKey("claude")!))
        try await model.changeBlobatar("c1", .colour(3))
        let claude = try XCTUnwrap(model.connection("c1"))
        XCTAssertEqual(claude.icon, Providers.blob)
        XCTAssertEqual(ConnectionLook.look(claude), .blob(seed: "c1", shape: 0, colour: 3))
        // A computer showing its platform too.
        try await model.changeBlobatar("c6", .shuffle)
        XCTAssertEqual(model.connection("c6")?.icon, Providers.blob)
        XCTAssertTrue(ConnectionLook.showsBlob(try XCTUnwrap(model.connection("c6"))))
    }

    func testTheWidgetsGetTheIconTheConnectionShows() async throws {
        let model = await model()
        await model.refreshPending()
        let item = try XCTUnwrap(model.pending.first { $0.connectionId == "c6" })
        let shown = model.snapshotItem(item)
        XCTAssertEqual(shown.connectionIcon, "macos")
        XCTAssertEqual(shown.connectionBlobatar, BlobatarLook(seed: "c6", shape: 4, colour: 6))
    }

    // MARK: Activity

    func testAnActivityRowNamesTheAgentAndItsComputer() {
        XCTAssertEqual(EntryText.byline(label: "M2A", agent: "Claude Code", time: "5 min"), "Claude Code · M2A · 5 min")
        XCTAssertEqual(EntryText.byline(label: "M2A", agent: nil, time: "5 min"), "M2A · 5 min")
        XCTAssertEqual(EntryText.byline(label: "M2A", agent: "  ", time: "5 min"), "M2A · 5 min", "a blank agent says nothing")
        XCTAssertEqual(EntryText.byline(label: "Claude", agent: "Claude", time: "now"), "Claude · now", "said once")
    }

    func testWhoAskedIsTheAgentAndTheComputersName() {
        XCTAssertEqual(AskedText.who(agent: "Claude Code", connection: "M2A", host: "127.0.0.1"), "Claude Code · M2A")
        XCTAssertEqual(AskedText.who(agent: "", connection: "Claude", host: "claude.ai"), "Claude · claude.ai", "no agent: as before")
    }
}
