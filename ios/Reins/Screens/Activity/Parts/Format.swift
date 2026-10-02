import Foundation

// Times and sizes in words (the Android app's ui/common/Format.kt and Files.kt).

enum TimeText {
    /// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", else the date.
    static func relative(_ epochSeconds: Int64, now: Int64 = Timers.nowSeconds()) -> String {
        let d = now - epochSeconds
        switch d {
        case ..<45: return "just now"
        case ..<3_600: return "\(max(1, d / 60)) min ago"
        case ..<86_400: return "\(d / 3_600) h ago"
        case ..<(2 * 86_400): return "yesterday"
        case ..<(7 * 86_400): return "\(d / 86_400) days ago"
        default: return date(epochSeconds).formatted(date: .abbreviated, time: .omitted)
        }
    }

    /// "Nov 14, 2023 at 2:14 PM": the locale's medium date and short time.
    static func dateTime(_ epochSeconds: Int64) -> String {
        date(epochSeconds).formatted(date: .abbreviated, time: .shortened)
    }

    /// "November 14, 2023 at 2:14 PM": date and time, spelled out.
    static func full(_ epochSeconds: Int64) -> String {
        date(epochSeconds).formatted(date: .long, time: .shortened)
    }

    /// "expires in 2 h", "expired", "no time limit".
    static func expiry(_ expiresAt: Int64?, now: Int64 = Timers.nowSeconds()) -> String {
        guard let expiresAt else { return "no time limit" }
        let left = expiresAt - now
        switch left {
        case ...0: return "expired"
        case ..<3_600: return "expires in \(max(1, left / 60)) min"
        case ..<86_400: return "expires in \(left / 3_600) h"
        default: return "expires in \(left / 86_400) days"
        }
    }

    /// "10 min", "6 h", "24 hours", "7 days": a permission's length.
    static func duration(_ secs: Int64) -> String {
        switch secs {
        case ..<3_600: return "\(secs / 60) min"
        case ..<86_400: return "\(secs / 3_600) h"
        case 86_400: return "24 hours"
        default: return "\(secs / 86_400) days"
        }
    }

    private static func date(_ epochSeconds: Int64) -> Date { Date(timeIntervalSince1970: TimeInterval(epochSeconds)) }
}

enum FileText {
    /// "812 bytes", "18.0 KB", "1.4 MB": the way the core words sizes.
    static func size(_ bytes: UInt64) -> String {
        let b = Double(bytes)
        switch bytes {
        case ..<1024: return "\(bytes) bytes"
        case ..<(1024 * 1024): return String(format: "%.1f KB", locale: posix, b / 1024)
        case ..<(1024 * 1024 * 1024): return String(format: "%.1f MB", locale: posix, b / (1024 * 1024))
        default: return String(format: "%.1f GB", locale: posix, b / (1024 * 1024 * 1024))
        }
    }

    /// The start and the end of a SHA-256, enough to compare by eye: "9f86d081884c7d65…0a08".
    static func shortSha(_ hex: String) -> String {
        hex.count <= 24 ? hex : hex.prefix(16) + "…" + hex.suffix(4)
    }

    /// A file whose preview is the start of its text (as opposed to a description of a binary file).
    static func isText(_ contentType: String) -> Bool {
        let type = (contentType.split(separator: ";", maxSplits: 1).first.map(String.init) ?? "")
            .trimmingCharacters(in: .whitespaces).lowercased()
        return type.hasPrefix("text/") || type == "application/json" || type.hasSuffix("+json") || type == "application/xml"
            || type.hasSuffix("+xml") || type == "application/x-ndjson"
    }

    static let posix = Locale(identifier: "en_US_POSIX")
}

/// "mcp.linear.app", "localhost:8080": a server by its host, never with its path or query.
func mcpHostLabel(_ url: String) -> String {
    guard let comps = URLComponents(string: url.trimmingCharacters(in: .whitespaces)), let host = comps.host, !host.isEmpty else {
        return url
    }
    return comps.port.map { "\(host):\($0)" } ?? host
}
