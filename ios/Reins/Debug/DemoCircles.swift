import Foundation

/// The demo core's Circles (`-demoCircles` seeds people, reviews and a notice): an invitation answers on its second
/// poll with the code and is accepted on the third; a link pasted into Join shows its terms and code on the first poll
/// and is active once accepted. All state sits behind one lock; nothing is held across a suspension point.
final class DemoCircleStore: @unchecked Sendable {
    struct Invite {
        var start: CircleInviteStart
        var draft: CircleDraft
        var polls = 0
        var confirmed = false
        var cancelled = false
    }

    private let lock = NSLock()
    private var available: Bool
    private var people: [CirclePerson] = []
    private var reviews: [CircleReviewView] = []
    private var notices: [CircleNoticeView] = []
    private var invites: [String: Invite] = [:]
    private var joins: [String: CircleInviteView] = [:]
    private var next = 1

    static let code = "482 193"
    static let helper = "alex.rivera@example.com"

    init(seeded: Bool, available: Bool = true, now: Int64 = Int64(Date().timeIntervalSince1970)) {
        self.available = available
        guard seeded else { return }
        people = Self.people(now)
        reviews = Self.reviews(now)
        notices = [
            CircleNoticeView(id: "n1", person: "dana@example.com", text: "Turned on Bypass for 1 h", outcome: nil, at: now - 1_800),
            CircleNoticeView(id: "n2", person: "dana@example.com", text: "Bypass ended", outcome: nil, at: now - 600),
        ]
    }

    private func locked<T>(_ body: () throws -> T) rethrows -> T {
        lock.lock()
        defer { lock.unlock() }
        return try body()
    }

    static func people(_ now: Int64) -> [CirclePerson] {
        [
            CirclePerson(linkId: "l1", peer: helper, role: .cosign, mine: true, scopeLine: "Money", deferred: true,
                         windowSecs: 4 * 3_600, state: .active, since: now - 86_400 * 12),
            CirclePerson(linkId: "l2", peer: "maria@studio.example", role: .delegate, mine: true, scopeLine: "Messages to Work or Outside",
                         deferred: false, windowSecs: 120, state: .paused, since: now - 86_400 * 30),
            CirclePerson(linkId: "l3", peer: "sam.lee@example.com", role: .guardian, mine: false, scopeLine: "Money · New contacts",
                         deferred: true, windowSecs: 4 * 3_600, state: .active, since: now - 86_400 * 60),
            CirclePerson(linkId: "l4", peer: "dana@example.com", role: .watcher, mine: false, scopeLine: "Bypass", deferred: true,
                         windowSecs: 0, state: .active, since: now - 86_400 * 4),
        ]
    }

    static func review(
        _ id: String, person: String, link: String, role: CircleRole, title: String, subtitle: String, agent: String, service: String,
        line: String, detail: String?, scope: String, created: Int64, deadline: Int64, state: CircleReviewState = .waiting
    ) -> CircleReviewView {
        CircleReviewView(
            id: id, linkId: link, person: person, role: role, title: title, subtitle: subtitle, headline: "\(agent) asks to \(title.lowercased()).",
            service: service, agent: agent, line: line, detail: detail, scopeLine: scope, createdAt: created, deadline: deadline, state: state
        )
    }

    static func reviews(_ now: Int64) -> [CircleReviewView] {
        [
            review("rv1", person: "sam.lee@example.com", link: "l3", role: .guardian, title: "Purchase $42", subtitle: "Lego set · amazon.com",
                   agent: "ChatGPT", service: "payments", line: "Ask · Outside · Spends money", detail: nil, scope: "Money · New contacts",
                   created: now - 900, deadline: now + 3 * 3_600),
            review("rv2", person: "sam.lee@example.com", link: "l3", role: .guardian, title: "Add a contact", subtitle: "Jordan Park",
                   agent: "ChatGPT", service: "gcontacts", line: "Ask · Outside · Reaches people", detail: nil, scope: "Money · New contacts",
                   created: now - 600, deadline: now + 3 * 3_600 + 600),
            review("rv3", person: "jo@acme.example", link: "l5", role: .delegate, title: "Send an email", subtitle: "To client@globex.example",
                   agent: "Claude", service: "gmail", line: "Ask · Work · Reaches people", detail: "Subject: Q4 numbers",
                   scope: "Messages to Work or Outside", created: now - 300, deadline: now + 100),
            review("rv0", person: "sam.lee@example.com", link: "l3", role: .guardian, title: "Purchase $15", subtitle: "Book · bookshop.org",
                   agent: "ChatGPT", service: "payments", line: "Ask · Outside · Spends money", detail: nil, scope: "Money · New contacts",
                   created: now - 86_400, deadline: now - 72_000, state: .approved),
        ]
    }

    // MARK: The core's calls

    func circlesAvailable() -> Bool { locked { available } }

    func inviteCreate(_ draft: CircleDraft) throws -> CircleInviteStart {
        try locked {
            guard available else { throw CoreError.Invalid(reason: "Circles need this phone to be your primary phone.") }
            let id = "inv\(next)"
            next += 1
            let start = CircleInviteStart(id: id, link: "reins-circle:demo.reins.dev/\(id)-k3Hq9VxT2", expiresAt: Int64(Date().timeIntervalSince1970) + 900)
            invites[id] = Invite(start: start, draft: draft)
            return start
        }
    }

    func invitePoll(_ id: String) throws -> CircleInviteProgress {
        try locked {
            guard var invite = invites[id] else { throw CoreError.NotFound }
            if invite.cancelled { return .cancelled }
            if invite.confirmed { return .active(linkId: "link-\(id)") }
            invite.polls += 1
            invites[id] = invite
            switch invite.polls {
            case ..<2: return .waiting
            case 2: return .compare(code: Self.code, helper: Self.helper)
            default: return .accepted(code: Self.code, helper: Self.helper)
            }
        }
    }

    func inviteConfirm(_ id: String) throws -> CirclePerson {
        try locked {
            guard var invite = invites[id], invite.polls >= 3, !invite.cancelled else {
                throw CoreError.Invalid(reason: "They haven't accepted yet.")
            }
            invite.confirmed = true
            invites[id] = invite
            let d = invite.draft
            let person = CirclePerson(
                linkId: "link-\(id)", peer: Self.helper, role: d.role, mine: true, scopeLine: CircleLogic.presetTitle(d.preset),
                deferred: d.deferred, windowSecs: d.deferred ? 4 * 3_600 : 120, state: .active, since: Int64(Date().timeIntervalSince1970)
            )
            people.removeAll { $0.linkId == person.linkId }
            people.append(person)
            return person
        }
    }

    func inviteCancel(_ id: String) { locked { invites[id]?.cancelled = true } }

    func joinOpen(_ link: String) throws -> CircleInviteView {
        try locked {
            let trimmed = link.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed.hasPrefix("reins-circle:"), trimmed.count > 16 else { throw CoreError.Invalid(reason: "That isn't a Circle link.") }
            let id = "join\(next)"
            next += 1
            let view = CircleInviteView(
                id: id, inviter: "sam.lee@example.com", role: .guardian, scopeLine: "", deferred: true, windowSecs: 4 * 3_600,
                fullCards: false, code: nil, stage: .opened
            )
            joins[id] = view
            return view
        }
    }

    func joinPoll(_ id: String) throws -> CircleInviteView {
        try locked {
            guard var view = joins[id] else { throw CoreError.NotFound }
            if view.stage == .opened {
                view.stage = .compare
                view.scopeLine = "Money · New contacts"
                view.code = Self.code
            } else if view.stage == .accepted {
                view.stage = .active
                people.append(CirclePerson(
                    linkId: "link-\(id)", peer: view.inviter, role: view.role, mine: false, scopeLine: view.scopeLine, deferred: view.deferred,
                    windowSecs: view.windowSecs, state: .active, since: Int64(Date().timeIntervalSince1970)
                ))
            }
            joins[id] = view
            return view
        }
    }

    func joinAccept(_ id: String) throws {
        try locked {
            guard joins[id]?.stage == .compare else { throw CoreError.Invalid(reason: "Compare the code first.") }
            joins[id]?.stage = .accepted
        }
    }

    func joinDecline(_ id: String) { locked { joins[id]?.stage = .declined } }

    func peopleList(refresh: Bool) -> [CirclePerson] { locked { people } }

    func pause(_ linkId: String, paused: Bool) throws {
        try locked {
            guard let i = people.firstIndex(where: { $0.linkId == linkId }), people[i].mine else { throw CoreError.NotFound }
            people[i].state = paused ? .paused : .active
        }
    }

    func end(_ linkId: String) throws {
        try locked {
            guard let i = people.firstIndex(where: { $0.linkId == linkId }) else { throw CoreError.NotFound }
            people[i].state = .ended
            for j in reviews.indices where reviews[j].linkId == linkId && reviews[j].state == .waiting { reviews[j].state = .withdrawn }
        }
    }

    func reviewList(refresh: Bool) -> [CircleReviewView] { locked { reviews } }

    func noticeList() -> [CircleNoticeView] { locked { notices } }

    func answer(_ id: String, approve: Bool) throws {
        try locked {
            guard let i = reviews.firstIndex(where: { $0.id == id }) else { throw CoreError.NotFound }
            guard reviews[i].state == .waiting else { throw CoreError.NotFound }
            reviews[i].state = approve ? .approved : .denied
        }
    }
}
