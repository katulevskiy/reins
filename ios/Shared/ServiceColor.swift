import SwiftUI

// Shared with the widget extension, whose request icons draw the same marks.

/// The brand colour of a service's mark.
func serviceColor(_ service: String) -> Color {
    switch service {
    case "gmail": Color(uiColor: UIColor(hex: 0xEA4335))
    case "telegram": Color(uiColor: UIColor(hex: 0x26A5E4))
    case "github": Palette.ink
    case "gitlab": Color(uiColor: UIColor(hex: 0xFC6D26))
    case "codeberg": Color(uiColor: UIColor(hex: 0x2185D0))
    case "bitbucket": Color(uiColor: UIColor(hex: 0x0052CC))
    case "mcp": Palette.violet
    case "gcalendar", "gcontacts": Color(uiColor: UIColor(hex: 0x4285F4))
    case "device_calendar": Color(uiColor: UIColor(hex: 0x34A853))
    case "device_contacts": Color(uiColor: UIColor(hex: 0xF57C00))
    case "sms": Color(uiColor: UIColor(hex: 0x00A884))
    case "vault": Color(uiColor: UIColor(hex: 0x175DDC))
    default: Palette.ink2
    }
}
