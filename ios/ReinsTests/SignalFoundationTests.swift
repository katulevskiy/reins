import SwiftUI
import UIKit
import XCTest
@testable import Reins

/// Colour and motion as `design/signal/tokens/signal.tokens.json` sets them, the Next type scale
/// (`Typography.swift`: Sora, Geologica for Cyrillic, JetBrains Mono), the glyphs the app ships, and which field each
/// kind of request gets.
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
        assertColor(Palette.paper, 0xF6F6F8, 0x121215, "background → paper")
        assertColor(Palette.ink, 0x16151B, 0xEDEDF0, "text → ink")
        assertColor(Palette.violet, 0x5B43E8, 0xA196FF, "accent → violet")
        assertColor(Palette.kindRead, 0x2563EB, 0x60A5FA, "search → kind-read")
        assertColor(Palette.kindRead, 0x2563EB, 0x60A5FA, "read → kind-read, no more teal")
        assertColor(Palette.kindPush, 0xC2410C, 0xFB923C, "send → kind-push")
        assertColor(Palette.violet, 0x5B43E8, 0xA196FF, "Overnight is violet")
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
        // The Type board: ask 33, title 30, card ask 22, row 16, meta 12.5, section 13, code 11.5 on 19, codes 24.
        let scale: [(TextRole, CGFloat, CGFloat?)] = [
            (.ask, 33, 34), (.cardAsk, 22, 23), (.title, 30, 31), (.quietTitle, 24, 25), (.button, 16.5, nil), (.row, 16, nil),
            (.body, 16, 21.5), (.meta, 12.5, nil), (.section, 13, nil), (.tag, 11.5, nil), (.value, 14, nil), (.detail, 13, nil),
            (.code, 11.5, 19), (.codes, 24, nil),
        ]
        for (role, size, lineHeight) in scale {
            XCTAssertEqual(role.size, size, "\(role)")
            XCTAssertEqual(role.lineHeight, lineHeight, "\(role)")
        }
        XCTAssertEqual(TextRole.ask.trackingEm, -0.03)
        XCTAssertEqual(TextRole.cardAsk.trackingEm, -0.02)
        XCTAssertEqual(TextRole.section.trackingEm, 0.015)
        XCTAssertEqual(TextRole.codes.trackingEm, 0.08)
        XCTAssertEqual(TextRole.ask.family, .display)
        XCTAssertEqual(TextRole.value.family, .mono)
        XCTAssertEqual(RFont.displayTrackingEm(30), -0.03, accuracy: 0.0001)
        XCTAssertEqual(RFont.displayTrackingEm(11.5), 0, accuracy: 0.0001)
    }

    func testNoWeightIsBold() {
        XCTAssertEqual(RFont.Weight.bold.sansName, "Sora-SemiBold")
        XCTAssertEqual(RFont.Weight.semibold.monoName, "JetBrainsMono-Medium")
        XCTAssertEqual(RFont.displayName(false), "Sora-Medium")
        XCTAssertEqual(RFont.displayName(true), "Sora-SemiBold")
    }

    func testTheFontsAreRegistered() {
        let names = ["Sora-Regular", "Sora-Medium", "Sora-SemiBold", "JetBrainsMono-Regular", "JetBrainsMono-Medium"]
        for name in names {
            XCTAssertNotNil(UIFont(name: name, size: 12), "\(name) is in UIAppFonts and the bundle")
        }
    }

    func testSoraCarriesCyrillic() throws {
        // Geologica's Cyrillic is merged into each Sora weight, so Russian text never falls back to SF.
        let font = try XCTUnwrap(UIFont(name: "Sora-Regular", size: 16)) as CTFont
        var glyphs = [CGGlyph](repeating: 0, count: 2)
        XCTAssertTrue(CTFontGetGlyphsForCharacters(font, Array("Жa".utf16), &glyphs, 2))
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
