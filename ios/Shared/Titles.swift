import Foundation

/// Text that came from outside the app (senders, subjects, snippets, client names). Bidirectional controls are
/// removed so it cannot reorder what is shown around it, and other control characters become spaces.
func untrusted(_ text: String) -> String {
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
            out.append(scalar)
        }
    }
    return String(out).trimmingCharacters(in: .whitespacesAndNewlines)
}

/// What an operation is (the Android app's `ActionKind`); each has its icon and colour.
enum ActionKind: String, CaseIterable {
    case search, list, read, write, send, grant, accounts, pair
    /// A file an AI uploaded through the server.
    case upload
    case other = "request"

    static func of(_ action: String) -> ActionKind { ActionKind(rawValue: action) ?? .other }

    /// SF Symbol.
    var symbol: String {
        switch self {
        case .search: "magnifyingglass"
        case .list: "list.bullet"
        case .write: "pencil"
        case .read: "envelope.open"
        case .send: "paperplane.fill"
        case .grant: "checkmark.shield.fill"
        case .accounts: "person.2.fill"
        case .pair: "link"
        case .upload: "doc.badge.arrow.up"
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
    case .accounts: return service.isEmpty ? "See integrations" : "See \(what) accounts"
    case .pair: return "Connect"
    case .upload: return "Share a file"
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
