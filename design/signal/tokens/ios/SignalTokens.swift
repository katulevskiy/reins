import SwiftUI
import UIKit

/// Reins Signal colour tokens. Generated from signal.tokens.json.
/// Fields are identical in light and dark; only paper and ink flip.
enum Signal {
    /// The ground of every screen. Cool paper in light, graphite in dark.
    static let paper = dynamic(0xF6F6F8, 1.0, 0x121215, 1.0)
    /// Sheets, inputs, header buttons, mode tiles, the Android nav bar.
    static let paperRaised = dynamic(0xFFFFFF, 1.0, 0x1C1C21, 1.0)
    /// Segmented wells, steppers, soft buttons, tags, a switch that is off.
    static let paperSunken = dynamic(0xECECF0, 1.0, 0x26262C, 1.0)
    /// Titles, asks on paper, row titles, values (16.8:1 light, 16:1 dark on `paper`).
    static let ink = dynamic(0x16151B, 1.0, 0xEDEDF0, 1.0)
    /// Row keys, body copy, values that aren't the point (5.9:1 light, 7.8:1 dark).
    static let ink2 = dynamic(0x5F5E6B, 1.0, 0xA6A6B0, 1.0)
    /// Small meta: senders, dates, section labels, hints (4.9:1 light, 5.8:1 dark on `paper`).
    static let inkMeta = dynamic(0x6B6A78, 1.0, 0x8E8E99, 1.0)
    /// Chevrons, unchecked rings, the sheet grabber. Decoration only, never text.
    static let inkFaint = dynamic(0xA3A2AD, 1.0, 0x63636E, 1.0)
    /// The hairline under every row and around raised plates.
    static let rule = dynamic(0xE3E3EA, 1.0, 0x2A2A30, 1.0)
    /// The main button and the floating tab bar: ink turned into a plate.
    static let inverse = dynamic(0x16151B, 1.0, 0xEDEDF0, 1.0)
    /// Text and glyphs on `inverse` (18:1 light, 16:1 dark).
    static let onInverse = dynamic(0xFFFFFF, 1.0, 0x121215, 1.0)
    /// Reins violet as text or a glyph on paper: links, Auto tags, Autopilot's voice (5.7:1 light, 7.4:1 dark).
    static let violet = dynamic(0x5B43E8, 1.0, 0xA196FF, 1.0)
    /// Violet as a fill: switches that are on, the Auto chip, count badges. White text on it is 6.2:1.
    static let violetFill = dynamic(0x5B43E8, 1.0, 0x5B43E8, 1.0)
    /// Additions in diffs, Ready, allowed outcomes.
    static let success = dynamic(0x15803D, 1.0, 0x34D399, 1.0)
    /// Deletions in diffs, destructive rows, end now, red hold buttons on paper.
    static let danger = dynamic(0xB91C1C, 1.0, 0xF87171, 1.0)
    /// Fill behind a red button on paper (End now, Bypass, Hold to delete).
    static let dangerWash = dynamic(0xFBE3E3, 1.0, 0xF87171, 0.14)
    /// The progress that fills a hold button as you hold.
    static let dangerFill2 = dynamic(0xF6C9C9, 1.0, 0xF87171, 0.28)
    /// Warning text on paper: a flagged code, an expiring item.
    static let warning = dynamic(0xA16207, 1.0, 0xFACC15, 1.0)
    /// Behind sheets; the field stays visible, dimmed.
    static let scrim = dynamic(0x16151B, 0.32, 0x000000, 0.55)
    /// Field: the AI wants to write or send (push, send email, comment). White text 5.2:1.
    static let fieldPush = dynamic(0xC2410C, 1.0, 0xC2410C, 1.0)
    /// Field: the AI wants to read or search. White text 5.2:1.
    static let fieldRead = dynamic(0x2563EB, 1.0, 0x2563EB, 1.0)
    /// Field: irreversible (delete, force-push, pay) and Bypass. White text 6.5:1.
    static let fieldDelete = dynamic(0xB91C1C, 1.0, 0xB91C1C, 1.0)
    /// Field: a grant, power you already handed over. White text 6.2:1.
    static let fieldGrant = dynamic(0x5B43E8, 1.0, 0x5B43E8, 1.0)
    /// Field: pair a device or connect an AI. White text 5.4:1.
    static let fieldPair = dynamic(0x0E7490, 1.0, 0x0E7490, 1.0)
    /// Field: account changes (passwords, recovery, sign-in). White text 6.0:1.
    static let fieldAccounts = dynamic(0xBE185D, 1.0, 0xBE185D, 1.0)
    /// Field: something is broken and needs you (notifications off, phone not approving). White text 4.9:1.
    static let fieldWarn = dynamic(0xA16207, 1.0, 0xA16207, 1.0)
    /// Text, glyphs and the Review button on any field.
    static let onField = dynamic(0xFFFFFF, 1.0, 0xFFFFFF, 1.0)
    /// Secondary text on a field: label line, device, verdict detail (14px+ only).
    static let onField2 = dynamic(0xFFFFFF, 0.82, 0xFFFFFF, 0.82)
    /// Translucent buttons and the close button on a field.
    static let onFieldWash = dynamic(0xFFFFFF, 0.18, 0xFFFFFF, 0.18)
    /// Stronger hairline: outline buttons, input borders.
    static let rule2 = dynamic(0xD3D3DC, 1.0, 0x3A3A42, 1.0)
    /// Kind tile ink on paper (tile fill is this at 13%, 14% in dark).
    static let kindPush = dynamic(0xC2410C, 1.0, 0xFB923C, 1.0)
    /// Kind tile ink.
    static let kindRead = dynamic(0x2563EB, 1.0, 0x60A5FA, 1.0)
    /// Kind tile ink.
    static let kindDelete = dynamic(0xB91C1C, 1.0, 0xF87171, 1.0)
    /// Kind tile ink.
    static let kindGrant = dynamic(0x5B43E8, 1.0, 0xA196FF, 1.0)
    /// Kind tile ink.
    static let kindPair = dynamic(0x0E7490, 1.0, 0x22D3EE, 1.0)
    /// Kind tile ink.
    static let kindAccounts = dynamic(0xBE185D, 1.0, 0xF472B6, 1.0)
    /// Kind tile ink.
    static let kindWarn = dynamic(0xA16207, 1.0, 0xFACC15, 1.0)

    static func dynamic(_ l: UInt32, _ la: CGFloat, _ d: UInt32, _ da: CGFloat) -> Color {
        Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? UIColor(signalHex: d, alpha: da) : UIColor(signalHex: l, alpha: la) })
    }
}

extension UIColor {
    convenience init(signalHex hex: UInt32, alpha: CGFloat = 1) {
        self.init(red: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255, blue: CGFloat(hex & 0xFF) / 255, alpha: alpha)
    }
}
