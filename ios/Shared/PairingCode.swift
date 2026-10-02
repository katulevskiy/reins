import Foundation

/// The code a computer shows to pair with this phone ("BCDF-GHJK"): eight letters from an alphabet without vowels or
/// letters easily mistaken for digits, which the phone redeems on its own server (`pairingByCode`). It reaches the phone as a QR code, a link
/// (`https://app.reins2fa.com/pair?code=BCDF-GHJK`, a self-hosted server's own `/pair`, or `reins://pair?code=...`) or
/// typed by hand. Pure Swift: the widgets compile it too and never link the core.
///
/// Links and scans are untrusted: only the code is taken from them, never the host, which is why any host will do.
enum PairingCode {
    static let alphabet: Set<Character> = Set("BCDFGHJKLMNPQRSTVWXZ")
    static let length = 8

    /// The code in its shown form ("BCDF-GHJK"), or nil if `raw` is not one. Case, spaces and dashes do not matter.
    static func normalize(_ raw: String) -> String? {
        let letters = raw.uppercased().filter { !$0.isWhitespace && $0 != "-" }
        guard letters.count == length, letters.allSatisfy(alphabet.contains) else { return nil }
        return String(letters.prefix(4)) + "-" + String(letters.suffix(4))
    }

    /// The code in a scanned QR code, an opened link or typed text, or nil when there is none.
    static func parse(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count <= 2048 else { return nil }
        if let code = normalize(trimmed) { return code }
        guard let c = URLComponents(string: trimmed), let scheme = c.scheme?.lowercased() else { return nil }
        switch scheme {
        // A local or self-hosted server can be plain http; the host is ignored either way.
        case "https", "http":
            guard c.host?.isEmpty == false, c.path == "/pair" || c.path.hasSuffix("/pair") || c.path.hasSuffix("/pair/") else { return nil }
        case DeepLink.scheme:
            guard c.host == "pair" else { return nil }
        default:
            return nil
        }
        return c.queryItems?.first { $0.name == "code" }?.value.flatMap(normalize)
    }
}
