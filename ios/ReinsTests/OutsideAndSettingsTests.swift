import SwiftUI
import XCTest
@testable import Reins

/// The Next look outside the app (widgets, Live Activities, notifications) and on the Settings pages: who asks stays
/// visible wherever something can be approved, and the pure helpers behind the pages.
final class OutsideAndSettingsTests: XCTestCase {
    private let now: Int64 = 1_800_000_000

    private func item(kind: Snapshot.Item.Kind = .request, connection: String = "Claude Code", quick: Bool? = nil, label: String? = "GitHub · Push") -> Snapshot.Item {
        var i = Snapshot.Item(
            id: "r1", kind: kind, title: "\(connection): Push 8 commits to octo/app", subtitle: "feature/passkeys", connection: connection,
            service: "github", createdAt: now - 20, expiresAt: now + 100, suggestion: nil, field: "push", label: label
        )
        i.quick = quick
        return i
    }

    // MARK: Who asks, wherever Allow is offered

    func testTheLiveActivityOffersApproveOnlyForARoutineRequestThatStillWaits() throws {
        let routine = try XCTUnwrap(Glance.approvalState([item(quick: true)], now: now))
        XCTAssertTrue(ApprovalLiveViews(state: routine, stale: false).offersApprove)
        XCTAssertFalse(ApprovalLiveViews(state: routine, stale: true).offersApprove, "no longer waiting: nothing to approve")
        let asked = try XCTUnwrap(Glance.approvalState([item(quick: false)], now: now))
        XCTAssertFalse(ApprovalLiveViews(state: asked, stale: false).offersApprove, "asked every time: Deny and Review only")
        let pairing = try XCTUnwrap(Glance.approvalState([item(kind: .pairing, quick: true, label: nil)], now: now))
        XCTAssertFalse(ApprovalLiveViews(state: pairing, stale: false).offersApprove, "a pairing is answered in the app")
    }

    func testTheWhoLineBesideApproveNamesTheRequesterOnItsOwnRun() throws {
        let state = try XCTUnwrap(Glance.approvalState([item(quick: true)], now: now))
        let views = ApprovalLiveViews(state: state, stale: false)
        let who = views.who(ring: .black)
        XCTAssertEqual(who.connection, "Claude Code", "who asks is drawn beside the ask, not only spoken")
        XCTAssertEqual(who.service, "github")
        XCTAssertEqual(who.glyph, .gitBranch)
        // The ask itself never carries the name.
        XCTAssertEqual(views.title, "Push 8 commits to octo/app")
    }

    func testARequestersNameIsCleanedAndCappedOnTheWhoLine() {
        let sneaky = "Claude\u{202E}edoC " + String(repeating: "x", count: 60)
        let shown = Glance.who(sneaky)
        XCTAssertFalse(shown.unicodeScalars.contains { $0.value == 0x202E }, "bidirectional controls are dropped")
        XCTAssertLessThanOrEqual(shown.count, 28)
        XCTAssertTrue(shown.hasSuffix("…"))
        XCTAssertEqual(Glance.who("Claude\nCode"), "Claude Code", "one line")
    }

    func testARoutineNotificationKeepsWhoAsksInItsTitle() {
        var p = PendingItem(
            kind: .request, id: "r1", title: "Read emails", subtitle: "me@gmail.com", createdAt: now, connectionId: "c1",
            connectionLabel: "Claude\u{202E} with a very long name that goes on and on and on", action: "read", count: 3, service: "gmail",
            account: "me@gmail.com", waitUntil: nil, op: "", opTitle: "", suggestion: nil
        )
        p.quick = true
        let c = NotificationContent.pending(p, settings: FeedbackSettings())
        XCTAssertEqual(c.categoryIdentifier, NotificationCategory.quickRequest.rawValue, "it offers Approve")
        XCTAssertTrue(c.title.hasPrefix("\(untrustedName(p.connectionLabel, limit: 40)): "), "who asks leads the bold line: \(c.title)")
    }

    // MARK: Badges, ages, controls

    func testTheRequestIconsBadgeFollowsTheAction() {
        XCTAssertEqual(Glance.actionGlyph(item()), .gitBranch)
        XCTAssertEqual(Glance.actionGlyph("read", field: nil), .eye)
        XCTAssertEqual(Glance.actionGlyph("search", field: nil), .search)
        XCTAssertEqual(Glance.actionGlyph(Glance.verb("Gmail · Send"), field: .push), .send)
        XCTAssertEqual(Glance.actionGlyph(nil, field: .delete), .trash, "no verb: the field's own")
        XCTAssertEqual(Glance.actionGlyph(item(kind: .pairing, label: nil)), .link)
        XCTAssertEqual(Glance.actionGlyph(item(kind: .join, label: nil)), .phone)
    }

    func testShortAges() {
        XCTAssertEqual(Glance.shortAgo(now - 10, now: now), "now")
        XCTAssertEqual(Glance.shortAgo(now - 120, now: now), "2m")
        XCTAssertEqual(Glance.shortAgo(now - 7_200, now: now), "2h")
        XCTAssertEqual(Glance.shortAgo(now - 3 * 86_400, now: now), "3d")
        XCTAssertEqual(SettingsText.compactAgo(now - 300, now: now), "5m")
        XCTAssertEqual(SettingsText.compactAgo(now - 3_600, now: now), "1h")
    }

    func testAControlIsColouredOnlyWhileOn() {
        XCTAssertNil(ControlLook.modeTint(.manual))
        XCTAssertNil(ControlLook.modeTint(.auto))
        XCTAssertEqual(ControlLook.modeTint(.bypass), Palette.fieldDelete)
        var s = Snapshot()
        s.signedIn = true
        XCTAssertEqual(ControlLook.stopState(s, now: now), "Off")
        s.bypassUntil = now + 47 * 60
        s.anyBypassUntil = s.bypassUntil
        XCTAssertEqual(ControlLook.stopState(s, now: now), "47 min left")
    }

    func testOlderSnapshotEntriesWithoutAServiceStillRead() throws {
        let old = #"{"id":1,"title":"Claude read 3 emails","outcome":"Released","approved":true,"at":1}"#
        let entry = try JSONDecoder().decode(Snapshot.Entry.self, from: Data(old.utf8))
        XCTAssertNil(entry.service)
        XCTAssertNil(entry.action)
    }

    // MARK: Your photo

    private func photo(width: Int = 300, height: Int = 400) throws -> CGImage {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: {
            let f = UIGraphicsImageRendererFormat()
            f.scale = 1
            return f
        }())
        let image = renderer.image { ctx in
            for y in stride(from: 0, to: height, by: 20) {
                UIColor(white: CGFloat(y) / CGFloat(height), alpha: 1).setFill()
                ctx.fill(CGRect(x: 0, y: y, width: width, height: 20))
            }
        }
        return try XCTUnwrap(image.cgImage)
    }

    private func pixel(_ image: CGImage, _ x: Int, _ y: Int) throws -> (UInt8, UInt8, UInt8) {
        let px = try XCTUnwrap(Pixels(fill: image, width: image.width, height: image.height))
        let i = px.index(x, y)
        return (px.bytes[i], px.bytes[i + 1], px.bytes[i + 2])
    }

    func testEveryEffectMakesAWallpaperOfTheRightSizeFadedIntoThePaper() throws {
        let source = try photo()
        for effect in WallpaperEffect.allCases {
            for dark in [true, false] {
                let out = try XCTUnwrap(WallpaperRender.render(source, effect: effect, fade: 0.55, dark: dark, size: CGSize(width: 78, height: 100)), "\(effect)")
                XCTAssertEqual(out.width, 78)
                XCTAssertEqual(out.height, 100)
                let bottom = try pixel(out, 40, 99)
                let paper = WallpaperRender.paper(dark: dark)
                XCTAssertLessThanOrEqual(abs(Int(bottom.0) - Int(paper.r)), 6, "\(effect) fades into the page at its bottom")
                XCTAssertLessThanOrEqual(abs(Int(bottom.2) - Int(paper.b)), 6)
            }
        }
    }

    /// Hues across, light to dark down: red, yellow, green, cyan, blue, magenta.
    private func colourful(width: Int = 390, height: Int = 500) throws -> CGImage {
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: {
            let f = UIGraphicsImageRendererFormat()
            f.scale = 1
            return f
        }())
        let image = renderer.image { ctx in
            for x in 0..<width {
                for y in stride(from: 0, to: height, by: 10) {
                    UIColor(hue: CGFloat(x) / CGFloat(width), saturation: 0.85, brightness: 1 - 0.7 * CGFloat(y) / CGFloat(height), alpha: 1).setFill()
                    ctx.fill(CGRect(x: x, y: y, width: 1, height: 10))
                }
            }
        }
        return try XCTUnwrap(image.cgImage)
    }

    /// Every pixel's colour, as 0xRRGGBB.
    private func colours(_ image: CGImage) throws -> [Int] {
        let px = try XCTUnwrap(Pixels(fill: image, width: image.width, height: image.height))
        return stride(from: 0, to: px.bytes.count, by: 4).map { Int(px.bytes[$0]) << 16 | Int(px.bytes[$0 + 1]) << 8 | Int(px.bytes[$0 + 2]) }
    }

    /// The share of pixels whose channels differ by more than 40: how much of a picture is in colour.
    private func coloured(_ colours: [Int]) -> Double {
        let count = colours.filter { c in
            let r = c >> 16 & 0xFF, g = c >> 8 & 0xFF, b = c & 0xFF
            return max(r, g, b) - min(r, g, b) > 40
        }.count
        return Double(count) / Double(colours.count)
    }

    func testDitherScanlinesAndHalftoneKeepTheColours() throws {
        let source = try colourful()
        XCTAssertGreaterThan(coloured(try colours(source)), 0.9)
        for dark in [true, false] {
            for effect in [WallpaperEffect.dither, .scanlines, .halftone] {
                let out = try XCTUnwrap(WallpaperRender.render(source, effect: effect, fade: 0, dark: dark, size: CGSize(width: 390, height: 500)))
                // Halftone leaves paper between its dots; the rest is in colour.
                XCTAssertGreaterThan(coloured(try colours(out)), effect == .halftone ? 0.25 : 0.6, "\(effect) (dark \(dark)) is not greyscale")
            }
        }
    }

    func testTheColoursStayWhereTheyWere() throws {
        let source = try colourful()
        for effect in [WallpaperEffect.dither, .scanlines, .halftone] {
            let out = try XCTUnwrap(WallpaperRender.render(source, effect: effect, fade: 0, dark: true, size: CGSize(width: 390, height: 500)))
            let px = try XCTUnwrap(Pixels(fill: out, width: out.width, height: out.height))
            // Halftone's dots, not the paper between them.
            let paper = WallpaperRender.paper(dark: true)
            func mean(_ x0: Int, _ x1: Int, _ channel: Int) -> Double {
                var sum = 0.0, n = 0.0
                for y in 20..<200 {
                    for x in x0..<x1 {
                        let i = px.index(x, y)
                        guard (px.bytes[i], px.bytes[i + 1], px.bytes[i + 2]) != (paper.r, paper.g, paper.b) else { continue }
                        sum += Double(px.bytes[i + channel]); n += 1
                    }
                }
                return sum / n
            }
            XCTAssertGreaterThan(mean(4, 30, 0), mean(4, 30, 2) + 40, "\(effect): red stays red")
            // 240° of 360: blue, at about two thirds across.
            XCTAssertGreaterThan(mean(250, 270, 2), mean(250, 270, 0) + 40, "\(effect): blue stays blue")
        }
    }

    func testDitherUsesThreeLevelsAChannel() throws {
        let out = try XCTUnwrap(WallpaperRender.render(try colourful(), effect: .dither, fade: 0, dark: true, size: CGSize(width: 390, height: 500)))
        let levels = Set(try colours(out).flatMap { [$0 >> 16 & 0xFF, $0 >> 8 & 0xFF, $0 & 0xFF] })
        XCTAssertEqual(levels, [0, 128, 255])
    }

    func testMonoUsesTheThemesTwoTones() throws {
        for (dark, tones) in [(true, Set([0x000000, 0xE8ECFF])), (false, Set([0xF6F6F8, 0x46464E]))] {
            let out = try XCTUnwrap(WallpaperRender.render(try photo(), effect: .mono, fade: 0, dark: dark, size: CGSize(width: 260, height: 333)))
            XCTAssertEqual(Set(try colours(out)), tones, "dark \(dark)")
        }
    }

    func testHalftoneDrawsOnThePaper() throws {
        let black = UIGraphicsImageRenderer(size: CGSize(width: 200, height: 250)).image { ctx in
            UIColor.black.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: 200, height: 250))
        }
        let out = try XCTUnwrap(WallpaperRender.render(try XCTUnwrap(black.cgImage), effect: .halftone, fade: 0, dark: true, size: CGSize(width: 200, height: 250)))
        let paper = WallpaperRender.paper(dark: true)
        XCTAssertEqual(Set(try colours(out)), [Int(paper.r) << 16 | Int(paper.g) << 8 | Int(paper.b)], "no dots in the dark, the paper all over")
    }

    func testMoreFadeStartsHigher() {
        XCTAssertLessThan(WallpaperRender.fadeStart(1), WallpaperRender.fadeStart(0))
    }

    func testNoFadeLeavesTheBottomAsItIs() throws {
        XCTAssertEqual(WallpaperRender.fadeStart(0), 1)
        let plain = try XCTUnwrap(WallpaperRender.render(try photo(), effect: .original, fade: 0, dark: true, size: CGSize(width: 78, height: 100)))
        let bottom = try pixel(plain, 40, 99)
        XCTAssertGreaterThan(Int(bottom.0), 0x80, "the photo's light bottom, not the dark paper")
    }

    func testAnEffectAlwaysFadesItsFootIntoThePaper() throws {
        for effect in WallpaperEffect.allCases where effect != .original {
            let art = try XCTUnwrap(WallpaperRender.render(try photo(), effect: effect, fade: 0, dark: true, size: CGSize(width: 78, height: 100)))
            let bottom = try pixel(art, 40, 99)
            let paper = WallpaperRender.paper(dark: true)
            XCTAssertLessThanOrEqual(abs(Int(bottom.0) - Int(paper.r)), 3, "\(effect) ends on the paper, not a hard line")
            XCTAssertLessThanOrEqual(abs(Int(bottom.2) - Int(paper.b)), 3, "\(effect)")
        }
    }

    // MARK: Wallpaper: image, effect and fade apart

    private func scratchDefaults() throws -> UserDefaults {
        let name = "wallpaper-tests-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    private func scratchDirectory() throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("wallpaper-tests-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    func testAChoiceKeepsImageEffectAndFadeApart() throws {
        let choice = WallpaperChoice(image: .crescent, effect: .halftone, fade: 0.404)
        XCTAssertEqual(choice.rawValue, "crescent|halftone|40")
        XCTAssertEqual(WallpaperChoice(rawValue: choice.rawValue), choice)
        XCTAssertNil(WallpaperChoice(rawValue: "torii-dither|original|0"), "Torii dither is Torii with Mono now")
        XCTAssertNil(WallpaperChoice(rawValue: "eyes|colour|0"), "Colour is gone")
        XCTAssertNil(WallpaperChoice(rawValue: "halo"), "the old key's bare image is not a choice")
        XCTAssertNil(WallpaperChoice(rawValue: "halo|sepia|0"))
        var faded = choice
        faded.fade = 1.7
        XCTAssertEqual(faded.fade, 1, "the fade stays in 0...1")
        XCTAssertEqual(WallpaperChoice.defaultFor(.dark), WallpaperChoice(image: .halo, effect: .original, fade: 0))
        XCTAssertEqual(WallpaperChoice.defaultFor(.light), WallpaperChoice(image: .torii, effect: .mono, fade: 0))
    }

    func testPickingAnImageGivesItsOwnEffectAndKeepsTheFade() {
        let current = WallpaperChoice(image: .halo, effect: .original, fade: 0.4)
        XCTAssertEqual(current.with(image: .eyes), WallpaperChoice(image: .eyes, effect: .scanlines, fade: 0.4))
        XCTAssertEqual(current.with(image: .torii), WallpaperChoice(image: .torii, effect: .mono, fade: 0.4))
        XCTAssertEqual(WallpaperChoice(image: .eyes, effect: .dither, fade: 0.4).with(image: .ruins), WallpaperChoice(image: .ruins, effect: .original, fade: 0.4))
        XCTAssertEqual(current.with(image: .own), WallpaperChoice(image: .own, effect: .original, fade: 0.4))
        // The shipped images that come as they were take effects; the treated ones take none.
        XCTAssertEqual(Wallpaper.shipped.filter(\.takesEffects), [.hands, .eyes, .torii])
        XCTAssertEqual(WallpaperChoice(image: .halo, effect: .dither, fade: 0.2).drawn, WallpaperChoice(image: .halo, effect: .original, fade: 0.2))
    }

    func testChoicesKeptBeforeTheOriginalsLookAsTheyDid() throws {
        let cases: [(String, WallpaperChoice?)] = [
            // Original meant the picture as shipped: Eyes had its scanlines, Torii was the photo.
            ("eyes|original|30", WallpaperChoice(image: .eyes, effect: .scanlines, fade: 0.3)),
            ("torii|original|30", WallpaperChoice(image: .torii, effect: .original, fade: 0.3)),
            ("halo|original|0", WallpaperChoice(image: .halo)),
            // Torii dither is Torii with Mono; an effect on it now goes on the photo.
            ("torii-dither|original|30", WallpaperChoice(image: .torii, effect: .mono, fade: 0.3)),
            ("torii-dither|halftone|30", WallpaperChoice(image: .torii, effect: .halftone, fade: 0.3)),
            // Colour (a lift of the colour) is gone: the picture as shipped.
            ("eyes|colour|0", WallpaperChoice(image: .eyes, effect: .scanlines)),
            ("own|colour|65", WallpaperChoice(image: .own, effect: .original, fade: 0.65)),
            // An image that ships treated is not treated again.
            ("halo|dither|0", WallpaperChoice(image: .halo)),
            // Effects on an image that came as it was stay.
            ("hands|dither|10", WallpaperChoice(image: .hands, effect: .dither, fade: 0.1)),
            ("own|scanlines|65", WallpaperChoice(image: .own, effect: .scanlines, fade: 0.65)),
            // The oldest key's bare image.
            ("eyes", WallpaperChoice(image: .eyes, effect: .scanlines)),
            ("torii-dither", WallpaperChoice(image: .torii, effect: .mono)),
            ("sepia|original|0", nil),
        ]
        for (raw, expected) in cases {
            XCTAssertEqual(WallpaperChoice.fromBefore(raw), expected, raw)
        }
        // Moved once, at launch, with the art drawn by the old effects.
        let defaults = try scratchDefaults()
        let caches = try scratchDirectory()
        defaults.set("eyes|original|0", forKey: WallpaperChoice.previousKey(.dark))
        defaults.set("torii-dither|original|0", forKey: WallpaperChoice.previousKey(.light))
        XCTAssertEqual(WallpaperChoice.initial(.dark, defaults: defaults), WallpaperChoice(image: .eyes, effect: .scanlines), "read before the move: as it was")
        WallpaperChoice.migrate(defaults: defaults, caches: caches)
        XCTAssertEqual(defaults.string(forKey: WallpaperChoice.key(.dark)), "eyes|scanlines|0")
        XCTAssertEqual(defaults.string(forKey: WallpaperChoice.key(.light)), "torii|mono|0")
        XCTAssertNil(defaults.object(forKey: WallpaperChoice.previousKey(.dark)), "the earlier key is read once")
        XCTAssertFalse(FileManager.default.fileExists(atPath: caches.path), "art drawn with the old effects goes")
        // Eyes as it is, picked since, stays as it is.
        defaults.set(WallpaperChoice(image: .eyes).rawValue, forKey: WallpaperChoice.key(.dark))
        WallpaperChoice.migrate(defaults: defaults, caches: caches)
        XCTAssertEqual(WallpaperChoice.initial(.dark, defaults: defaults), WallpaperChoice(image: .eyes))
    }

    func testTheOldKeyBecomesAChoiceOnceWithNoEffectAndNoAddedFade() throws {
        let defaults = try scratchDefaults()
        defaults.set("own", forKey: Wallpaper.legacyKey(.dark))
        defaults.set("crescent", forKey: Wallpaper.legacyKey(.light))
        XCTAssertEqual(WallpaperChoice.initial(.dark, defaults: defaults), WallpaperChoice(image: .own), "read before the move: as it was")
        WallpaperChoice.migrate(defaults: defaults)
        XCTAssertEqual(defaults.string(forKey: WallpaperChoice.key(.dark)), "own|original|0", "a baked photo is drawn as it is")
        XCTAssertEqual(defaults.string(forKey: WallpaperChoice.key(.light)), "crescent|original|0")
        XCTAssertNil(defaults.object(forKey: Wallpaper.legacyKey(.dark)), "the old key is read once")
        XCTAssertNil(defaults.object(forKey: Wallpaper.legacyKey(.light)))
        // A choice made since is never overwritten by an old key.
        defaults.set(WallpaperChoice(image: .eyes, effect: .dither, fade: 0.3).rawValue, forKey: WallpaperChoice.key(.dark))
        defaults.set("halo", forKey: Wallpaper.legacyKey(.dark))
        WallpaperChoice.migrate(defaults: defaults)
        XCTAssertEqual(WallpaperChoice.initial(.dark, defaults: defaults), WallpaperChoice(image: .eyes, effect: .dither, fade: 0.3))
        XCTAssertNil(defaults.object(forKey: Wallpaper.legacyKey(.dark)))
    }

    func testWithNothingSavedEachThemeOpensOnItsShippedDefault() throws {
        let defaults = try scratchDefaults()
        WallpaperChoice.migrate(defaults: defaults)
        XCTAssertNil(defaults.object(forKey: WallpaperChoice.key(.dark)), "nothing to move, nothing written")
        XCTAssertEqual(WallpaperChoice.initial(.dark, defaults: defaults), .defaultFor(.dark))
        XCTAssertEqual(WallpaperChoice.initial(.light, defaults: defaults), .defaultFor(.light))
    }

    func testAnOldBakedPhotoIsDrawnFromUntilANewOneIsKept() throws {
        let dir = try scratchDirectory()
        XCTAssertNil(OwnWallpaper.url(.dark, in: dir))
        let baked = try XCTUnwrap(UIImage(cgImage: try photo()).jpegData(compressionQuality: 0.9))
        try baked.write(to: OwnWallpaper.legacyURL(.dark, in: dir))
        XCTAssertEqual(OwnWallpaper.url(.dark, in: dir), OwnWallpaper.legacyURL(.dark, in: dir))
        XCTAssertNil(OwnWallpaper.url(.light, in: dir), "one photo per theme")
        try OwnWallpaper.save(try photo(), .dark, in: dir)
        XCTAssertEqual(OwnWallpaper.url(.dark, in: dir), OwnWallpaper.sourceURL(.dark, in: dir))
        XCTAssertFalse(FileManager.default.fileExists(atPath: OwnWallpaper.legacyURL(.dark, in: dir).path), "the baked one goes")
    }

    func testTheCacheNamesEverythingThatChangesTheArt() {
        let base = WallpaperArt(choice: WallpaperChoice(image: .eyes, effect: .dither, fade: 0.2), dark: true)
        XCTAssertEqual(base.key, "dark_eyes_dither_f20_r2")
        let keys: Set<String> = [
            base.key,
            WallpaperArt(choice: WallpaperChoice(image: .torii, effect: .dither, fade: 0.2), dark: true).key,
            WallpaperArt(choice: WallpaperChoice(image: .eyes, effect: .halftone, fade: 0.2), dark: true).key,
            WallpaperArt(choice: WallpaperChoice(image: .eyes, effect: .dither, fade: 0.21), dark: true).key,
            WallpaperArt(choice: WallpaperChoice(image: .eyes, effect: .dither, fade: 0.2), dark: false).key,
        ]
        XCTAssertEqual(keys.count, 5, "image, effect, fade and theme each name another file")
        XCTAssertEqual(WallpaperArt(choice: base.choice, dark: true, photo: 7).key, base.key, "a shipped image has no photo revision")
        let own = WallpaperChoice(image: .own)
        XCTAssertNotEqual(WallpaperArt(choice: own, dark: true, photo: 1).key, WallpaperArt(choice: own, dark: true, photo: 2).key, "a new photo is drawn anew")
    }

    func testOnlyAShippedImageAsItShipsSkipsTheRender() {
        XCTAssertTrue(WallpaperArt(choice: .defaultFor(.dark), dark: true).isAsset)
        XCTAssertFalse(WallpaperArt(choice: .defaultFor(.light), dark: false).isAsset, "Torii with Mono is drawn")
        XCTAssertFalse(WallpaperArt(choice: WallpaperChoice(image: .eyes, effect: .scanlines), dark: true).isAsset)
        XCTAssertTrue(WallpaperArt(choice: WallpaperChoice(image: .halo, effect: .scanlines), dark: true).isAsset, "a treated image is drawn as it is")
        XCTAssertFalse(WallpaperArt(choice: WallpaperChoice(image: .halo, fade: 0.01), dark: true).isAsset)
        XCTAssertFalse(WallpaperArt(choice: WallpaperChoice(image: .own), dark: true).isAsset, "your photo is always drawn to the wallpaper's size")
    }

    func testAMissingPhotoFallsBackToTheThemesImageKeepingTheEffect() {
        let choice = WallpaperChoice(image: .own, effect: .halftone, fade: 0.5)
        XCTAssertEqual(WallpaperArt.shown(choice, photo: 3, scheme: .light, hasPhoto: false).choice, WallpaperChoice(image: .torii, effect: .halftone, fade: 0.5))
        XCTAssertEqual(WallpaperArt.shown(choice, photo: 3, scheme: .light, hasPhoto: true).choice, choice)
        // Halo ships treated: it takes no effect.
        XCTAssertEqual(WallpaperArt.shown(choice, photo: 3, scheme: .dark, hasPhoto: false).choice, WallpaperChoice(image: .halo, effect: .original, fade: 0.5))
    }

    func testTheCacheKeepsEachThemesNewestFew() throws {
        let dir = try scratchDirectory()
        for (i, name) in ["dark_a", "dark_b", "dark_c", "dark_d", "light_a"].enumerated() {
            let url = dir.appendingPathComponent(name + ".jpg")
            try Data([1]).write(to: url)
            try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: Double(1_000 + i))], ofItemAtPath: url.path)
        }
        WallpaperArt.prune(dir, dark: true, keeping: dir.appendingPathComponent("dark_a.jpg"))
        XCTAssertEqual(Set(try FileManager.default.contentsOfDirectory(atPath: dir.path)), ["dark_a.jpg", "dark_c.jpg", "dark_d.jpg", "light_a.jpg"])
    }

    // MARK: Settings words

    func testDeletingTheAccountListsWhatGoesInFiveWords() {
        XCTAssertEqual(SettingsText.deletionItems, ["Account and vault", "Connections and access", "Permissions and activity", "Sign-in", "Data on this phone"])
    }

    func testTheRecoveryCodeIsNumberedInReadingOrder() {
        let cells = RecoveryCodeSheet.cells("AAAA-BBBB-CCCC-DDDD")
        XCTAssertEqual(cells.map(\.number), [1, 2, 3, 4])
        XCTAssertEqual(cells.last?.group, "DDDD")
    }
}
