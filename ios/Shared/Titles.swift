import Foundation

/// Text that came from outside the app (senders, subjects, snippets, client names, commands). Bidirectional controls and
/// every other invisible format character (zero-width spaces and joiners, the byte-order mark, soft hyphens, tag
/// characters) are removed so it cannot reorder or hide what is shown around it; other control characters become
/// spaces. Never cut: what the user approves (a command, its arguments, a recipient, a path) is shown whole. Names and
/// labels that should stay short use `untrustedName`.
func untrusted(_ text: String) -> String {
    cleaned(text, showHidden: false).trimmingCharacters(in: .whitespacesAndNewlines)
}

/// `untrusted` for a line whose leading spaces matter (a line of code in a diff): nothing is trimmed, and an invisible
/// format character shows as U+FFFD, so it can be seen rather than silently change what the line reads as.
/// Bidirectional controls are still removed.
func untrustedLine(_ text: String) -> String {
    cleaned(text, showHidden: true)
}

/// A name or label from outside (an AI's or a device's name, a host): `untrusted`, on one line (line breaks and tabs
/// become spaces, runs of spaces one), and at most `limit` characters, ending in "…" when longer. Only for names and
/// titles, never for anything approved.
func untrustedName(_ text: String, limit: Int = 64) -> String {
    let flat = untrusted(text)
        .components(separatedBy: .whitespacesAndNewlines)
        .filter { !$0.isEmpty }
        .joined(separator: " ")
    guard flat.count > limit, limit > 1 else { return flat }
    return String(flat.prefix(limit - 1)).trimmingCharacters(in: .whitespaces) + "…"
}

private func cleaned(_ text: String, showHidden: Bool) -> String {
    var out = String.UnicodeScalarView()
    for scalar in text.unicodeScalars {
        switch scalar.value {
        case 0x200E, 0x200F, 0x061C, 0x202A...0x202E, 0x2066...0x2069:
            continue
        case 0x0A, 0x09:
            out.append(scalar)
        case 0x00...0x1F, 0x7F...0x9F, 0x2028, 0x2029:
            out.append(" ")
        default:
            if scalar.properties.generalCategory == .format {
                if showHidden { out.append("\u{FFFD}") }
            } else {
                out.append(scalar)
            }
        }
    }
    return String(out)
}

/// What an operation is (the Android app's `ActionKind`); each has its icon and colour.
enum ActionKind: String, CaseIterable {
    case search, list, read, write, send, grant, accounts, pair
    /// A starter grant's "headlines": search results show who, what and when; opening anything still asks.
    case headlines
    /// A file an AI uploaded through the server.
    case upload
    /// Another phone asks for the account's keys.
    case join
    case other = "request"

    static func of(_ action: String) -> ActionKind { ActionKind(rawValue: action) ?? .other }

    /// SF Symbol.
    var symbol: String {
        switch self {
        case .search: "magnifyingglass"
        case .list: "list.bullet"
        case .write: "pencil"
        case .read: "envelope.open"
        case .headlines: "list.bullet.rectangle"
        case .send: "paperplane.fill"
        case .grant: "checkmark.shield.fill"
        case .accounts: "person.2.fill"
        case .pair: "link"
        case .upload: "doc.badge.arrow.up"
        case .join: "iphone.gen3"
        case .other: "key.fill"
        }
    }
}

/// The service id of an MCP server's calls: `mcp:<server id>`.
let mcpPrefix = "mcp:"

/// The names of the MCP servers the user added and of their tools, so lists and notifications (which only carry
/// `mcp:<id>` and the tool's name) can say "Linear" and "Create issue". Kept up to date from the app state.
enum McpNames {
    struct Server: Codable {
        var name: String
        var tools: [String: String]
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var servers: [String: Server] = [:]
    private static let key = "mcp.names.v1"

    static func update(_ list: [String: Server]) {
        lock.lock()
        servers = list
        lock.unlock()
        if let data = try? JSONEncoder().encode(list) { AppGroup.defaults.set(data, forKey: key) }
    }

    /// Reads the names the app saved last (the extensions start with nothing).
    static func loadSaved() {
        guard let data = AppGroup.defaults.data(forKey: key), let list = try? JSONDecoder().decode([String: Server].self, from: data) else {
            return
        }
        lock.lock()
        servers = list
        lock.unlock()
    }

    static func server(_ id: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return servers[id]?.name
    }

    /// A tool's title, when the server and the tool are known.
    static func tool(_ serverId: String, _ tool: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return servers[serverId]?.tools[tool]
    }

    /// The server a permission names: permissions call it `mcp_<id>` with `-` as `_` (the core's `grant_service`).
    static func server(grantService: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return servers.first { mcpGrantPrefix + $0.key.replacingOccurrences(of: "-", with: "_") == grantService }?.value.name
    }
}

/// The service id an MCP server's permissions name: `mcp_<server id>`.
let mcpGrantPrefix = "mcp_"

/// "Gmail", "Google Calendar", "Linear" (an MCP server's name).
func serviceName(_ service: String) -> String {
    if service.hasPrefix(mcpPrefix) { return McpNames.server(String(service.dropFirst(mcpPrefix.count))) ?? "MCP server" }
    if service.hasPrefix(mcpGrantPrefix) { return McpNames.server(grantService: service) ?? "MCP server" }
    switch service {
    case "gmail": return "Gmail"
    case "telegram": return "Telegram"
    case "gcalendar": return "Google Calendar"
    case "gcontacts": return "Google Contacts"
    case "device_calendar": return "Phone calendar"
    case "device_contacts": return "Phone contacts"
    case "sms": return "Text messages"
    case "github": return "GitHub"
    case "gitlab": return "GitLab"
    case "codeberg": return "Codeberg"
    case "bitbucket": return "Bitbucket"
    case "desktop": return "Desktop app"
    case "files": return "Files"
    case "vault": return "Password vault"
    default: return service.prefix(1).uppercased() + service.dropFirst()
    }
}

/// "Search Gmail", "Read 3 emails", "Send email to 2": what an operation is, in a few words. `title` is the name the
/// core gives an operation of another integration ("Read Telegram messages"); without it a generic one is made up.
/// A call to an MCP server's tool reads as the tool's title (`op` is the tool's name).
func operationTitle(action: String, count: Int, service: String, title: String = "", op: String = "") -> String {
    if !title.trimmingCharacters(in: .whitespaces).isEmpty { return title }
    if service.hasPrefix(mcpPrefix) { return mcpTitle(String(service.dropFirst(mcpPrefix.count)), op) }
    let what = serviceName(service.isEmpty ? "gmail" : service)
    let mail = service.isEmpty || service == "gmail"
    switch ActionKind.of(action) {
    case .search: return "Search \(what)"
    case .list: return "List \(what)"
    case .write: return "Change \(what)"
    case .read:
        if !mail { return "Read \(what)" }
        switch count {
        case 0: return "Read email"
        case 1: return "Read 1 email"
        default: return "Read \(count) emails"
        }
    case .send:
        if !mail { return "Send with \(what)" }
        return count > 1 ? "Send email to \(count)" : "Send email"
    case .grant: return "Ask for access"
    case .headlines: return mail ? "See email headlines" : "See \(what) names"
    case .accounts: return service.isEmpty ? "See integrations" : "See \(what) accounts"
    case .pair: return "Connect"
    case .upload: return "Share a file"
    case .join: return "Add a phone"
    case .other: return "Request"
    }
}

private func mcpTitle(_ serverId: String, _ tool: String) -> String {
    if tool.trimmingCharacters(in: .whitespaces).isEmpty { return "Use a tool of \(serviceName(mcpPrefix + serverId))" }
    if let title = McpNames.tool(serverId, tool).map(untrusted), !title.isEmpty { return title }
    return "Use \(untrusted(tool))"
}

/// "Claude: Search Gmail".
func fullTitle(label: String, action: String, count: Int, service: String, title: String = "", op: String = "") -> String {
    "\(untrusted(label)): \(operationTitle(action: action, count: count, service: service, title: title, op: op))"
}

/// The headline of an activity entry; a permission reads as what happened to it.
func entryTitle(label: String, action: String, count: Int, service: String, outcome: String, title: String = "", op: String = "") -> String {
    let what: String
    if ActionKind.of(action) == .grant {
        switch outcome {
        case "granted": what = "Access granted"
        case "denied": what = "Access refused"
        default: what = "Access request"
        }
    } else {
        what = operationTitle(action: action, count: count, service: service, title: title, op: op)
    }
    return "\(untrusted(label)): \(what)"
}

/// A host as ASCII: each label with characters beyond ASCII becomes its punycode ("bücher.de" → "xn--bcher-kva.de"),
/// so a look-alike domain shows what it really is.
func asciiHost(_ host: String) -> String {
    host.split(separator: ".", omittingEmptySubsequences: false).map { label -> String in
        let scalars = Array(label.lowercased().unicodeScalars)
        guard scalars.contains(where: { !$0.isASCII }), let encoded = punycode(scalars) else { return String(label) }
        return "xn--" + encoded
    }.joined(separator: ".")
}

/// An address with its domain as ASCII: "Anna <anna@bücher.de>" → "Anna <anna@xn--bcher-kva.de>".
func asciiAddress(_ address: String) -> String {
    guard let at = address.lastIndex(of: "@") else { return address }
    let rest = address[address.index(after: at)...]
    let end = rest.firstIndex(where: { $0 == ">" || $0 == " " }) ?? rest.endIndex
    return String(address[...at]) + asciiHost(String(rest[..<end])) + String(rest[end...])
}

/// RFC 3492 punycode of one label's scalars; nil if it would overflow.
private func punycode(_ input: [Unicode.Scalar]) -> String? {
    let base = 36, tMin = 1, tMax = 26, skew = 38, damp = 700
    func digit(_ d: Int) -> Character { Character(Unicode.Scalar(UInt8(d < 26 ? d + 97 : d + 22))) }
    func adapt(_ delta: Int, _ points: Int, _ first: Bool) -> Int {
        var delta = first ? delta / damp : delta / 2
        delta += delta / points
        var k = 0
        while delta > ((base - tMin) * tMax) / 2 {
            delta /= base - tMin
            k += base
        }
        return k + (base - tMin + 1) * delta / (delta + skew)
    }
    var output = String(input.filter(\.isASCII).map(Character.init))
    let basic = output.count
    var handled = basic
    if basic > 0 { output.append("-") }
    var n = 128, delta = 0, bias = 72
    while handled < input.count {
        guard let m = input.map({ Int($0.value) }).filter({ $0 >= n }).min() else { break }
        let (step, overflow) = (m - n).multipliedReportingOverflow(by: handled + 1)
        guard !overflow else { return nil }
        delta += step
        n = m
        for scalar in input {
            let c = Int(scalar.value)
            if c < n { delta += 1 }
            if c == n {
                var q = delta
                var k = base
                while true {
                    let t = k <= bias ? tMin : k >= bias + tMax ? tMax : k - bias
                    if q < t { break }
                    output.append(digit(t + (q - t) % (base - t)))
                    q = (q - t) / (base - t)
                    k += base
                }
                output.append(digit(q))
                bias = adapt(delta, handled + 1, handled == basic)
                delta = 0
                handled += 1
            }
        }
        delta += 1
        n += 1
    }
    return output
}

/// The raw form of a name that mixes letters of different scripts ("octo/аpp" with a Cyrillic "а" →
/// "octo/\u{430}pp"), so a look-alike shows what it is; nil for a name in one script.
func mixedScriptRaw(_ text: String) -> String? {
    enum Script { case latin, greek, cyrillic, armenian, fullwidth, other }
    func script(_ s: Unicode.Scalar) -> Script? {
        guard s.properties.isAlphabetic else { return nil }
        switch s.value {
        case 0x41...0x5A, 0x61...0x7A, 0xC0...0x24F, 0x1E00...0x1EFF: return .latin
        case 0x370...0x3FF, 0x1F00...0x1FFF: return .greek
        case 0x400...0x52F, 0x1C80...0x1C8F, 0x2DE0...0x2DFF, 0xA640...0xA69F: return .cyrillic
        case 0x530...0x58F: return .armenian
        case 0xFF21...0xFF3A, 0xFF41...0xFF5A: return .fullwidth
        default: return .other
        }
    }
    let scripts = Set(text.unicodeScalars.compactMap(script))
    guard scripts.count > 1 else { return nil }
    return text.unicodeScalars.map { $0.isASCII ? String($0) : "\\u{\(String($0.value, radix: 16))}" }.joined()
}
