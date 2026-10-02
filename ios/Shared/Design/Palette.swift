import SwiftUI
import UIKit

/// Reins Light / Reins Dark: cool neutrals and one violet accent, the same family as Zeron (and the Android app's
/// `RColors`). Colour only; nothing here affects layout.
enum Palette {
    static func dynamic(_ light: UInt32, _ dark: UInt32, alpha: CGFloat = 1) -> Color {
        Color(uiColor: UIColor { traits in
            UIColor(hex: traits.userInterfaceStyle == .dark ? dark : light, alpha: alpha)
        })
    }

    static func dual(_ light: UIColor, _ dark: UIColor) -> Color {
        Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? dark : light })
    }

    static let background = dynamic(0xF3F3F5, 0x060606)
    static let elevated = dynamic(0xFFFFFF, 0x111113)
    static let text = dynamic(0x27272C, 0xE8E8EA)
    static let secondary = dynamic(0x62626A, 0xA9A9AE)
    static let tertiary = dynamic(0x97979F, 0x6B6B72)
    static let hairline = dynamic(0xE2E2E6, 0x1E1E22)
    static let accent = dynamic(0x5B43E8, 0x8B7CF6)
    static let accentSoft = dynamic(0x5B43E8, 0x8B7CF6, alpha: 0.12)
    /// Translucent control fill that reads on glass and plain surfaces in both modes.
    static let controlFill = dynamic(0x27272C, 0xE8E8EA, alpha: 0.075)
    static let danger = dynamic(0xDC2626, 0xF87171)
    static let success = dynamic(0x15803D, 0x34D399)
    static let warning = dynamic(0xA16207, 0xFACC15)
    static let chip = dynamic(0xE7E7EB, 0x1C1C20)
    /// The selected row in the iPad sidebar.
    static let rowActive = dual(UIColor(white: 1, alpha: 0.9), UIColor(white: 1, alpha: 0.06))

    // What an operation is: each kind has its own colour so a list reads at a glance.
    static let search = dynamic(0x2563EB, 0x60A5FA)
    static let read = dynamic(0x0F766E, 0x2DD4BF)
    static let send = dynamic(0xC2410C, 0xFB923C)
    static let grant = accent
    static let pair = dynamic(0x0E7490, 0x22D3EE)
    static let accounts = dynamic(0xBE185D, 0xF472B6)

    /// Floating chrome where glass is not used (toasts over content): a near-opaque plate with a hairline edge.
    static let plate = dual(UIColor(white: 1, alpha: 0.99), UIColor(red: 0x1C / 255, green: 0x1C / 255, blue: 0x1F / 255, alpha: 0.99))
    static let plateEdge = dual(UIColor(white: 0, alpha: 0.07), UIColor(white: 1, alpha: 0.08))
    static let scrim = dual(UIColor(white: 0, alpha: 0.32), UIColor(white: 0, alpha: 0.55))
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
