import Foundation

// How a push from git is put into words (the Android app's GitPush.kt). Pure, tested in GitPushFormatTests.

enum GitPushFormat {
    /// Commits shown before "and N more".
    static let commitsShown = 5
    /// Files shown before "and N more".
    static let filesShown = 8

    /// What an update does to the history of a ref.
    enum History { case adds, rewrites, unknown }

    /// An update that could not be checked is treated as a force push, and said so apart from one that is known to be.
    static func history(_ ref: GitRefView) -> History {
        if ref.forceUnknown { return .unknown }
        return ref.force ? .rewrites : .adds
    }

    /// The chip next to a ref: "New branch", "Update", "Delete", "New tag", "Tag" (a tag that moves).
    static func chipLabel(_ ref: GitRefView) -> String {
        switch ref.change {
        case "delete": return "Delete"
        case "create":
            switch ref.kind {
            case "branch": return "New branch"
            case "tag": return "New tag"
            default: return "New"
            }
        default: return ref.kind == "tag" ? "Tag" : "Update"
        }
    }

    private static func files(_ n: UInt32) -> String { n == 1 ? "1 file" : "\(n) files" }

    /// "+120 −14 in 12 files"; a count the desktop app could not make is left out. Nil when no file changes.
    static func totalsLabel(_ ref: GitRefView) -> String? {
        if ref.filesChanged == 0 { return nil }
        let counts = [ref.additions.map { "+\($0)" }, ref.deletions.map { "−\($0)" }].compactMap { $0 }
        return counts.isEmpty ? "\(files(ref.filesChanged)) changed" : "\(counts.joined(separator: " ")) in \(files(ref.filesChanged))"
    }

    static func fileLetter(_ status: String) -> String {
        switch status {
        case "added": "A"
        case "modified": "M"
        case "deleted": "D"
        case "type_changed": "T"
        default: "?"
        }
    }

    /// "+3 −1", "binary", or nothing when the lines were not counted.
    static func fileCounts(_ file: GitFileView) -> String {
        if file.binary { return "binary" }
        return [file.additions.map { "+\($0)" }, file.deletions.map { "−\($0)" }].compactMap { $0 }.joined(separator: " ")
    }

    /// "0 B", "12.4 KB", "3.0 MB" (powers of 1024).
    static func packSize(_ bytes: UInt64) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        let units = ["KB", "MB", "GB"]
        var value = Double(bytes) / 1024
        var unit = 0
        while value >= 1024 && unit < units.count - 1 {
            value /= 1024
            unit += 1
        }
        return String(format: "%.1f %@", locale: FileText.posix, value, units[unit])
    }
}

// The desktop app's other requests, in words (the Android app's NewSections.kt).

/// "for 30 minutes", "for 1 hour", "for 1 h 30 min": how long the desktop app may keep secrets.
func leaseLabel(_ secs: UInt64) -> String {
    let minutes = max(Int64(secs / 60), 1)
    if minutes < 60 { return minutes == 1 ? "for 1 minute" : "for \(minutes) minutes" }
    let hours = minutes / 60
    let rest = minutes % 60
    if rest != 0 { return "for \(hours) h \(rest) min" }
    return hours == 1 ? "for 1 hour" : "for \(hours) hours"
}

/// The server of an SSH sign-in: its name when the desktop app knows it, else its host key.
func sshTarget(host: String?, hostKey: String?) -> String {
    if let h = host?.trimmingCharacters(in: .whitespaces), !h.isEmpty { return h }
    if let k = hostKey?.trimmingCharacters(in: .whitespaces), !k.isEmpty { return k }
    return "an unknown server"
}
