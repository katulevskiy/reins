import SwiftUI
import UIKit

/// Reins Signal (design/signal/tokens/signal.tokens.json): paper, ink and hairlines for everything you read, and seven
/// fields of colour for the moment something needs a decision. Fields are the same in light and dark; only paper and
/// ink flip. Colour only; nothing here affects layout. The names from before Signal are gone: use these.
enum Palette {
    static func dynamic(_ light: UInt32, _ dark: UInt32, alpha: CGFloat = 1) -> Color {
        Color(uiColor: UIColor { traits in
            UIColor(hex: traits.userInterfaceStyle == .dark ? dark : light, alpha: alpha)
        })
    }

    static func dual(_ light: UIColor, _ dark: UIColor) -> Color {
        Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? dark : light })
    }

    // MARK: Paper and ink

    /// The ground of every screen. Cool paper in light, graphite in dark.
    static let paper = dynamic(0xF6F6F8, 0x121215)
    /// Sheets, inputs, header buttons, mode tiles.
    static let paperRaised = dynamic(0xFFFFFF, 0x1C1C21)
    /// Segmented wells, steppers, soft buttons, tags, a switch that is off.
    static let paperSunken = dynamic(0xECECF0, 0x26262C)
    /// Titles, asks on paper, row titles, values.
    static let ink = dynamic(0x16151B, 0xEDEDF0)
    /// Row keys, body copy, values that aren't the point.
    static let ink2 = dynamic(0x5F5E6B, 0xA6A6B0)
    /// Small meta: senders, dates, section labels, hints.
    static let inkMeta = dynamic(0x6B6A78, 0x8E8E99)
    /// Chevrons, unchecked rings, the sheet grabber. Decoration only, never text.
    static let inkFaint = dynamic(0xA3A2AD, 0x63636E)
    /// The hairline under every row and around raised plates.
    static let rule = dynamic(0xE3E3EA, 0x2A2A30)
    /// The stronger hairline: outline buttons, input borders.
    static let rule2 = dynamic(0xD3D3DC, 0x3A3A42)
    /// The main button and the floating tab bar: ink turned into a plate.
    static let inverse = dynamic(0x16151B, 0xEDEDF0)
    /// Text and glyphs on `inverse`.
    static let onInverse = dynamic(0xFFFFFF, 0x121215)

    // MARK: Violet and status

    /// Reins violet as text or a glyph on paper: links, Auto tags, Autopilot's voice.
    static let violet = dynamic(0x5B43E8, 0xA196FF)
    /// Violet as a fill: switches that are on, the Auto chip, count badges. White text on it.
    static let violetFill = dynamic(0x5B43E8, 0x5B43E8)
    /// Additions in diffs, Ready, allowed outcomes.
    static let success = dynamic(0x15803D, 0x34D399)
    /// Deletions in diffs, destructive rows, End now, red hold buttons on paper.
    static let danger = dynamic(0xB91C1C, 0xF87171)
    /// The fill behind a red button on paper (End now, Bypass, Hold to delete).
    static let dangerWash = dual(UIColor(hex: 0xFBE3E3), UIColor(hex: 0xF87171, alpha: 0.14))
    /// The progress that fills a hold button as you hold.
    static let dangerFill2 = dual(UIColor(hex: 0xF6C9C9), UIColor(hex: 0xF87171, alpha: 0.28))
    /// Warning text on paper: a flagged code, an expiring item.
    static let warning = dynamic(0xA16207, 0xFACC15)
    /// Behind sheets; what is under them stays visible, dimmed.
    static let scrim = dual(UIColor(hex: 0x16151B, alpha: 0.32), UIColor(white: 0, alpha: 0.55))

    // MARK: Fields (the same in both themes; white text on each at 4.9:1 or more)

    /// The AI wants to write or send (push, send email, comment).
    static let fieldPush = Color(uiColor: UIColor(hex: 0xC2410C))
    /// The AI wants to read or search.
    static let fieldRead = Color(uiColor: UIColor(hex: 0x2563EB))
    /// Irreversible (delete, force-push, pay) and Bypass.
    static let fieldDelete = Color(uiColor: UIColor(hex: 0xB91C1C))
    /// A grant: power already handed over. Also Overnight while it runs.
    static let fieldGrant = Color(uiColor: UIColor(hex: 0x5B43E8))
    /// Pair a device or connect an AI.
    static let fieldPair = Color(uiColor: UIColor(hex: 0x0E7490))
    /// Account changes (passwords, recovery, sign-in).
    static let fieldAccounts = Color(uiColor: UIColor(hex: 0xBE185D))
    /// Reins is broken and needs you (notifications off, another phone approves).
    static let fieldWarn = Color(uiColor: UIColor(hex: 0xA16207))
    /// Text, glyphs and the Review button on any field.
    static let onField = Color.white
    /// Secondary text on a field: the label line, the device, the verdict's detail (14 pt and up only).
    static let onField2 = Color.white.opacity(0.82)
    /// Translucent buttons and the close button on a field.
    static let onFieldWash = Color.white.opacity(0.18)
    /// Ink on a field's white button or pill (Review): light ink in both themes, because the button stays white.
    static let onFieldInk = Color(uiColor: UIColor(hex: 0x16151B))

    // MARK: Kind inks (a kind on paper: tiles, row glyphs; the tile is this at 13 %, lighter in dark)

    static let kindPush = dynamic(0xC2410C, 0xFB923C)
    static let kindRead = dynamic(0x2563EB, 0x60A5FA)
    static let kindDelete = dynamic(0xB91C1C, 0xF87171)
    static let kindGrant = dynamic(0x5B43E8, 0xA196FF)
    static let kindPair = dynamic(0x0E7490, 0x22D3EE)
    static let kindAccounts = dynamic(0xBE185D, 0xF472B6)
    static let kindWarn = dynamic(0xA16207, 0xFACC15)

    // MARK: Washes

    /// Violet at 13 %: a chosen chip's or a soft violet button's fill on paper.
    static let violetWash = dual(UIColor(hex: 0x5B43E8, alpha: 0.13), UIColor(hex: 0xA196FF, alpha: 0.13))
}

extension UIColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(
            red: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}
