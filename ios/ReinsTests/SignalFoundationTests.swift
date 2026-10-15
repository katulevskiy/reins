import SwiftUI
import UIKit
import XCTest
@testable import Reins

/// Signal's foundation: the tokens as the design package sets them (design/signal/tokens/signal.tokens.json), the
/// fonts and glyphs the app ships, and which field each kind of request gets.
final class SignalFoundationTests: XCTestCase {
    private func hex(_ color: Color, dark: Bool) -> UInt32 {
        let traits = UITraitCollection(userInterfaceStyle: dark ? .dark : .light)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(color).resolvedColor(with: traits).getRed(&r, green: &g, blue: &b, alpha: &a)
        return UInt32((r * 255).rounded()) << 16 | UInt32((g * 255).rounded()) << 8 | UInt32((b * 255).rounded())
    }

    private func assertColor(_ color: Color, _ light: UInt32, _ dark: UInt32, _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(hex(color, dark: false), light, "\(name) in light", file: file, line: line)
        XCTAssertEqual(hex(color, dark: true), dark, "\(name) in dark", file: file, line: line)
    }

    func testPaperAndInkFlipAndTheOldNamesFollow() {
        assertColor(Palette.paper, 0xF6F6F8, 0x121215, "paper")
        assertColor(Palette.paperRaised, 0xFFFFFF, 0x1C1C21, "paper-raised")
        assertColor(Palette.paperSunken, 0xECECF0, 0x26262C, "paper-sunken")
        assertColor(Palette.ink, 0x16151B, 0xEDEDF0, "ink")
        assertColor(Palette.ink2, 0x5F5E6B, 0xA6A6B0, "ink-2")
        assertColor(Palette.inkMeta, 0x6B6A78, 0x8E8E99, "ink-meta")
        assertColor(Palette.inkFaint, 0xA3A2AD, 0x63636E, "ink-faint")
        assertColor(Palette.rule, 0xE3E3EA, 0x2A2A30, "rule")
        assertColor(Palette.violet, 0x5B43E8, 0xA196FF, "violet")
        assertColor(Palette.violetFill, 0x5B43E8, 0x5B43E8, "violet-fill")
        assertColor(Palette.danger, 0xB91C1C, 0xF87171, "danger")
        // The aliases screens still use until phase 2.
        assertColor(Palette.background, 0xF6F6F8, 0x121215, "background → paper")
        assertColor(Palette.text, 0x16151B, 0xEDEDF0, "text → ink")
        assertColor(Palette.accent, 0x5B43E8, 0xA196FF, "accent → violet")
        assertColor(Palette.search, 0x2563EB, 0x60A5FA, "search → kind-read")
        assertColor(Palette.read, 0x2563EB, 0x60A5FA, "read → kind-read, no more teal")
        assertColor(Palette.send, 0xC2410C, 0xFB923C, "send → kind-push")
        assertColor(Palette.overnight, 0x5B43E8, 0xA196FF, "Overnight is violet")
    }

    func testFieldsAreTheSameInBothThemes() {
        let fields: [FieldKind: UInt32] = [
            .push: 0xC2410C, .read: 0x2563EB, .delete: 0xB91C1C, .grant: 0x5B43E8, .pair: 0x0E7490, .accounts: 0xBE185D, .warn: 0xA16207,
        ]
        for (kind, value) in fields {
            assertColor(kind.fill, value, value, "field-\(kind.rawValue)")
        }
        assertColor(FieldKind.push.ink, 0xC2410C, 0xFB923C, "kind-push")
        assertColor(FieldKind.pair.ink, 0x0E7490, 0x22D3EE, "kind-pair")
    }

    func testEachKindOfRequestGetsItsField() {
        XCTAssertEqual(FieldKind.of(.search), .read)
        XCTAssertEqual(FieldKind.of(.read), .read, "reads and searches are both read")
        XCTAssertEqual(FieldKind.of(.headlines), .read)
        XCTAssertEqual(FieldKind.of(.upload), .read)
        XCTAssertEqual(FieldKind.of(.write), .push)
        XCTAssertEqual(FieldKind.of(.send), .push)
        XCTAssertEqual(FieldKind.of(.grant), .grant)
        XCTAssertEqual(FieldKind.of(.accounts), .accounts)
        XCTAssertEqual(FieldKind.of(.join), .pair)
        XCTAssertEqual(FieldKind.of(.send, danger: true), .delete, "what can't be undone is red")
        XCTAssertNil(FieldKind.of(.other))
        XCTAssertEqual(HeroTone.write.field, .push)
        XCTAssertEqual(HeroTone.send.field, .push)
        XCTAssertEqual(HeroTone.search.field, .read)
        XCTAssertEqual(HeroTone.grant.field, .grant)
        XCTAssertEqual(HeroTone.danger.field, .delete)
    }

    func testTheHoldTakesItsFullTimeAndDrainsQuickly() {
        XCTAssertEqual(HoldButton.defaultDuration, 1.2, accuracy: 0.0001)
        XCTAssertEqual(Motion.holdDrain, 0.18, accuracy: 0.0001)
        XCTAssertEqual(Motion.menuOpen, 0.18, accuracy: 0.0001)
        XCTAssertEqual(Motion.menuClose, 0.12, accuracy: 0.0001)
    }

    func testTheTypeScale() {
        let scale: [(TextRole, CGFloat, CGFloat?)] = [
            (.ask, 45, 46), (.askSmall, 25, 26), (.title, 44, 44), (.stat, 40, 40), (.sheetTitle, 32, 34), (.mood, 24, 30),
            (.button, 16.5, nil), (.row, 15.5, nil), (.body, 15, 22), (.meta, 13, 18), (.tag, 11.5, nil), (.label, 11.5, 14),
            (.value, 13.5, nil),
        ]
        for (role, size, lineHeight) in scale {
            XCTAssertEqual(role.size, size, "\(role)")
            XCTAssertEqual(role.lineHeight, lineHeight, "\(role)")
        }
        XCTAssertEqual(TextRole.label.trackingEm, 0.16)
        XCTAssertTrue(TextRole.mood.italic)
        XCTAssertEqual(TextRole.ask.family, .serif)
        XCTAssertEqual(TextRole.label.family, .mono)
    }

    func testNoWeightIsBold() {
        XCTAssertEqual(RFont.SansFamily.instrument.name(.bold), "InstrumentSans-SemiBold")
        XCTAssertEqual(RFont.SansFamily.geist.name(.bold), "Geist-SemiBold")
        XCTAssertEqual(RFont.Weight.bold.monoName, "GeistMono-SemiBold")
    }

    func testTheFontsAreRegistered() {
        var names = RFont.SansFamily.allCases.flatMap { family in [RFont.Weight.regular, .medium, .semibold].map(family.name) }
        names += [RFont.serifName(false), RFont.serifName(true), "GeistMono-Regular", "GeistMono-Medium", "GeistMono-SemiBold"]
        for name in names {
            XCTAssertNotNil(UIFont(name: name, size: 12), "\(name) is in UIAppFonts and the bundle")
        }
    }

    func testEveryGlyphAndTextureShips() {
        for glyph in Glyph.allCases {
            XCTAssertNotNil(UIImage(named: glyph.rawValue), "glyph \(glyph.rawValue)")
        }
        for name in ["grain", "grain-widget", "pond", "kite"] {
            XCTAssertNotNil(UIImage(named: name), name)
        }
    }
}
