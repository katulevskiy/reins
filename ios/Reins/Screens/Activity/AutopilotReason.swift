import SwiftUI

// Autopilot's reason line (PLAN.md): "like 11 pushes you approved", built on the phone from the decisions you made
// yourself, and for pushes refined with the repository and the kind of branch ("like 11 pushes to feature branches of
// acme/web you approved"), counted from the decided entries' stored approvals, which keep the push's refs.

/// The reason line in words (pure, unit-tested).
enum ReasonText {
    enum BranchKind: Equatable {
        /// The repository's default branch (main, master, or what the push names as default).
        case main
        /// Any other branch.
        case feature
    }

    /// What one decision of a kind is called, and many: "push", "pushes".
    static func noun(action: String, op: String, service: String) -> (one: String, many: String) {
        let verb = (op.isEmpty ? action : op).lowercased()
        if verb.contains("push") { return ("push", "pushes") }
        switch ActionKind.of(action) {
        case .search, .list: return ("search", "searches")
        case .read, .headlines: return service.isEmpty || service == "gmail" ? ("read", "reads") : ("look", "looks")
        case .send: return service.isEmpty || service == "gmail" ? ("email", "emails") : ("message", "messages")
        case .write: return ("change", "changes")
        case .grant: return ("permission", "permissions")
        case .accounts: return ("account change", "account changes")
        case .pair, .join: return ("connection", "connections")
        case .upload: return ("file", "files")
        case .other: return ("request", "requests")
        }
    }

    /// The kind of branch a push goes to.
    static func branchKind(_ branch: String, defaultBranch: String?) -> BranchKind {
        if let defaultBranch, !defaultBranch.isEmpty { return branch == defaultBranch ? .main : .feature }
        return ["main", "master", "trunk"].contains(branch) ? .main : .feature
    }

    /// "Like 11 pushes to feature branches of acme/web you approved." Nil with nothing alike.
    static func line(count: Int, noun: (one: String, many: String), repo: String? = nil, branch: BranchKind? = nil) -> String? {
        guard count > 0 else { return nil }
        let what = count == 1 ? "1 \(noun.one)" : "\(count) \(noun.many)"
        var place = ""
        if let repo, !repo.isEmpty {
            switch branch {
            case .main?: place = " to the default branch of \(repo)"
            case .feature?: place = " to feature branches of \(repo)"
            case nil: place = " to \(repo)"
            }
        }
        return "Like \(what)\(place) you approved."
    }
}

/// Counts the decisions like an entry that you approved yourself, reading the stored approvals of pushes for their
/// repository and branch. The approvals are read once and kept.
@MainActor
@Observable
final class ReasonMemory {
    /// A push's repository and branch kind, by entry id; nil when the entry kept no push.
    @ObservationIgnored private var pushes: [Int64: (repo: String, branch: ReasonText.BranchKind)?] = [:]
    private(set) var lines: [Int64: String] = [:]

    /// At most this many older decisions are read for a push.
    static let reads = 60

    /// Works out `entry`'s line, if it has none yet.
    func explain(_ entry: ActivityEntry, model: AppModel) async {
        guard lines[entry.id] == nil else { return }
        let alike = model.activity.filter {
            $0.id != entry.id && $0.decidedBy.isEmpty && $0.outcome != "denied" && $0.outcome != "error"
                && $0.service == entry.service && $0.action == entry.action && $0.op == entry.op
        }
        let noun = ReasonText.noun(action: entry.action, op: entry.op, service: entry.service)
        guard noun.one == "push", let own = await push(entry.id, core: model.core) else {
            if let line = ReasonText.line(count: alike.count, noun: noun) { lines[entry.id] = line }
            return
        }
        var count = 0
        for other in alike.prefix(Self.reads) {
            if let p = await push(other.id, core: model.core), p.repo == own.repo, p.branch == own.branch { count += 1 }
        }
        // Without one alike in that repository and kind of branch, the plain count still says something.
        lines[entry.id] = ReasonText.line(count: count, noun: noun, repo: own.repo, branch: own.branch)
            ?? ReasonText.line(count: alike.count, noun: noun)
    }

    private func push(_ id: Int64, core: any ReinsCoreProtocol) async -> (repo: String, branch: ReasonText.BranchKind)? {
        if let known = pushes[id] { return known }
        let view = try? await core.activityApproval(entryId: id)
        let found = view?.git.flatMap { git -> (String, ReasonText.BranchKind)? in
            guard let ref = git.refs.first else { return nil }
            return (git.repo, ReasonText.branchKind(ref.shortName, defaultBranch: git.defaultBranch))
        }
        pushes[id] = found
        return found
    }
}
