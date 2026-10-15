import SwiftUI

// The stack (iOS-Activity.png, 01): what waits, as cards in their field colour, the first on top and the rest peeking
// below. Swipe a card right to take Autopilot's pick, left to deny, tap it to review. The answers go through the same
// paths as the cards' buttons (Burst.answer), with the same feedback, played at the decision.

/// Activity in words (pure, unit-tested).
enum ActivityText {
    private static let words = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]

    private static func number(_ n: Int) -> String { n < words.count ? words[n] : "\(n)" }

    private static func capitalized(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    /// The mood line under the title: "Two things wait for you.", else "All quiet. Three AIs are connected."
    static func mood(waiting: Int, connections: Int) -> String {
        switch waiting {
        case 0:
            switch connections {
            case 0: return "All quiet. Nothing is connected yet."
            case 1: return "All quiet. One AI is connected."
            default: return "All quiet. \(capitalized(number(connections))) AIs are connected."
            }
        case 1: return "One thing waits for you."
        default: return "\(capitalized(number(waiting))) things wait for you."
        }
    }

    /// The irregular and common verbs a request starts with, in the past: what was done reads as done ("Pushed 3
    /// commits"); what was denied keeps its ask ("Delete branch old/experiment").
    private static let pastTense: [String: String] = [
        "push": "Pushed", "read": "Read", "send": "Sent", "search": "Searched", "delete": "Deleted", "create": "Created",
        "use": "Used", "open": "Opened", "list": "Listed", "share": "Shared", "write": "Wrote", "run": "Ran", "make": "Made",
        "add": "Added", "update": "Updated", "comment": "Commented", "merge": "Merged", "close": "Closed", "connect": "Connected",
        "see": "Saw", "get": "Got", "give": "Gave", "hand": "Handed", "move": "Moved", "change": "Changed", "edit": "Edited",
        "remove": "Removed", "reply": "Replied", "forward": "Forwarded", "archive": "Archived", "label": "Labelled",
        "fetch": "Fetched", "upload": "Uploaded", "download": "Downloaded", "sign": "Signed", "release": "Released",
    ]

    /// `lead` in the past when the entry was done: its first word, if it is a verb this knows.
    static func past(_ lead: String) -> String {
        let space = lead.firstIndex(of: " ") ?? lead.endIndex
        let first = String(lead[..<space])
        guard let done = pastTense[first.lowercased()] else { return lead }
        return done + lead[space...]
    }

    /// An entry that went through (not denied, not failed) reads in the past.
    static func done(_ entry: ActivityEntry) -> Bool { entry.outcome != "denied" && entry.outcome != "error" }

    /// The entries a filter and an AI keep (`connection` "" for any AI): a filter of what is loaded, nothing asked of the core.
    static func filtered(_ entries: [ActivityEntry], filter: ActivityFilter, connection: String) -> [ActivityEntry] {
        guard filter != .all || !connection.isEmpty else { return entries }
        return entries.filter { (filter == .all || filter.keeps($0)) && (connection.isEmpty || $0.connectionId == connection) }
    }

    /// The AIs in the history, newest first, once each: (connection id, its name).
    static func ais(_ entries: [ActivityEntry]) -> [(id: String, name: String)] {
        var seen: Set<String> = []
        var out: [(id: String, name: String)] = []
        for e in entries where !e.connectionId.isEmpty && seen.insert(e.connectionId).inserted {
            out.append((e.connectionId, untrusted(e.connectionLabel)))
        }
        return out
    }

    /// The river's count for a day: "12 asked · 9 by Autopilot" ("" for none).
    static func dayCount(_ entries: [ActivityEntry]) -> String {
        guard !entries.isEmpty else { return "" }
        let auto = entries.filter { !$0.decidedBy.isEmpty }.count
        return auto == 0 ? "\(entries.count) asked" : "\(entries.count) asked · \(auto) by Autopilot"
    }

    /// The label line of a waiting card: "Gmail · Send" (set in mono capitals).
    static func label(_ item: PendingItem) -> String {
        switch item.kind {
        case .pairing: return "Computer · Pair"
        case .join: return "Account · New phone"
        case .blob: return "Files · Share"
        case .request: break
        }
        let service = item.label.map(\.provider) ?? item.service
        let action = item.label.map(\.action) ?? item.action
        let name = service.isEmpty ? "Request" : serviceName(service)
        let verb = action.isEmpty ? "" : " · " + action.replacingOccurrences(of: "_", with: " ")
        return name + verb
    }

    /// The ask on a waiting card, its target in italic: the core's label ("Send an email to" *Anna Petrova*), or the
    /// sentence Activity says ("Search Gmail for" *“invoice”*).
    static func ask(_ group: WaitGroup) -> [AskText.Run] {
        if !group.isBurst, let label = group.first.label {
            let lead = untrusted(label.lead)
            let subject = untrusted(label.subject)
            let tail = untrustedLine(label.tail)
            var runs: [AskText.Run] = []
            if !lead.isEmpty { runs.append(.init(text: subject.isEmpty ? lead : lead + " ")) }
            if !subject.isEmpty { runs.append(.init(text: subject, target: true)) }
            if !tail.isEmpty { runs.append(.init(text: tail)) }
            return runs
        }
        let pieces = group.isBurst ? WantsText.of(burst: group.items) : WantsText.of(group.first)
        return runs(pieces)
    }

    /// "Wants to **read 2 emails**" as an ask: the AI's wish turned into what it would do, "Read 2 emails".
    static func runs(_ pieces: [WantsText.Piece]) -> [AskText.Run] {
        var pieces = pieces
        if let first = pieces.first, first.text == "Wants to ", pieces.count > 1 {
            pieces.removeFirst()
            pieces[0].text = capitalized(pieces[0].text)
            // The verb reads plainly; what it is done to is the target.
            pieces[0].strong = false
        }
        return pieces.map { AskText.Run(text: $0.text, target: $0.strong) }
    }

    /// The line under the ask, when the label does not already say it ("“Draft for Friday”").
    static func detail(_ item: PendingItem) -> String? {
        guard item.label != nil, item.kind == .request else { return nil }
        let subtitle = untrusted(item.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !subtitle.isEmpty, !RequestLabelLook.plain(item.label!).contains(subtitle) else { return nil }
        return subtitle
    }

    /// The field a waiting card is painted in: its kind; a new phone changes the account, a file goes out.
    static func field(_ item: PendingItem, danger: Bool) -> FieldKind {
        switch item.kind {
        case .pairing: return .pair
        case .join: return .accounts
        case .blob: return .push
        case .request: return FieldKind.of(ActionKind.of(item.action), danger: danger) ?? .push
        }
    }

    /// Autopilot's pick as a card's first button.
    static func pick(_ advice: WaitAdvice, _ suggestion: SuggestionView?) -> AutopilotPick? {
        switch advice {
        case .allow: .allow(Int(((suggestion?.pApprove ?? 1) * 100).rounded()))
        case .deny: .deny(Int(((suggestion?.pDeny ?? 1) * 100).rounded()))
        case .unsure: .unsure
        case .alwaysAsks: .alwaysAsks
        case .review: nil
        }
    }
}

/// The waiting cards as a stack: the first on top, the next two peeking below as edges of their own colour; "Show all"
/// lays them out one under another.
struct WaitStack: View {
    var groups: [WaitGroup]
    var suggestions: [String: SuggestionView]
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(spacing: 10) {
            if expanded {
                ForEach(groups) { group in card(group) }
            } else if let top = groups.first {
                card(top)
                    .background(alignment: .bottom) { peeks }
                    .padding(.bottom, CGFloat(min(groups.count - 1, 2)) * 8)
            }
            if groups.count > 1 {
                Button {
                    withAnimation(Motion.of(Motion.easeOut(Motion.fold), reduce: reduceMotion)) { expanded.toggle() }
                } label: {
                    Text(expanded ? "Show one" : "\(groups.count - 1) more waiting · Show all")
                        .font(RFont.sans(13.5, .medium))
                        .foregroundStyle(Palette.ink2)
                        .frame(minHeight: 30)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .feedbackTap(.tap, feedback)
                .accessibilityIdentifier("stackToggle")
            }
        }
        .padding(.horizontal, 16)
    }

    private func card(_ group: WaitGroup) -> some View {
        FieldWaitCard(
            group: group,
            advice: WaitAdvice.of(group.items, suggestions),
            suggestion: suggestions[group.first.id],
            danger: group.items.contains { suggestions[$0.id]?.floor == true }
        )
        .id("p:\(group.id)")
        .transition(reduceMotion ? .opacity : .asymmetric(insertion: .move(edge: .top).combined(with: .opacity), removal: .opacity))
    }

    /// The next cards' bottom edges, a little narrower each, in their own colours.
    private var peeks: some View {
        ZStack(alignment: .bottom) {
            ForEach(Array(groups.dropFirst().prefix(2).enumerated().reversed()), id: \.element.id) { i, group in
                let field = ActivityText.field(group.first, danger: group.items.contains { suggestions[$0.id]?.floor == true })
                UnevenRoundedRectangle(bottomLeadingRadius: 22, bottomTrailingRadius: 22, style: .continuous)
                    .fill(field.fill)
                    .brightness(-0.08 * Double(i + 1))
                    .frame(height: 40)
                    .padding(.horizontal, CGFloat(i + 1) * 10)
                    .offset(y: CGFloat(i + 1) * 8)
            }
        }
        .accessibilityHidden(true)
    }
}

/// A waiting request, connection, new phone or file as a card in its field: the AI's mark, the label line and the time
/// left; the ask; then Autopilot's pick and Review. Follows the thumb sideways, tilting up to 6°; past 35 % it flies
/// off and answers: right takes Autopilot's pick (Allow, or Deny), left denies a request. Anything else springs back,
/// and a pick that is not one (Unsure, Always asks) opens the review instead.
struct FieldWaitCard: View {
    var group: WaitGroup
    var advice: WaitAdvice
    var suggestion: SuggestionView?
    var danger: Bool
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var busy = false
    @State private var offset: CGFloat = 0
    /// Whether the drag going on is a sideways one (decided at its start, so scrolling still scrolls).
    @State private var sideways: Bool?
    @State private var width: CGFloat = 360

    private var item: PendingItem { group.first }
    private var field: FieldKind { ActivityText.field(item, danger: danger) }
    private var pick: AutopilotPick? { ActivityText.pick(advice, suggestion) }

    var body: some View {
        WaitCardShell(
            kind: field, label: ActivityText.label(item), ask: AskText(ActivityText.ask(group), role: .askSmall),
            askIdentifier: "pendingTitle:\(item.id)"
        ) {
            ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: 24)
                .environment(\.colorScheme, .light)
        } timer: {
            TimerPill(createdAt: item.createdAt, waitUntil: item.kind == .blob ? nil : group.items.compactMap(\.waitUntil).min(), onField: true)
        } actions: {
            VStack(alignment: .leading, spacing: 12) {
                if let detail = ActivityText.detail(item) {
                    Text(detail).font(RFont.sans(13.5)).foregroundStyle(Palette.onField2).lineLimit(2).padding(.top, -4)
                }
                if group.isBurst { peek }
                if let pick {
                    WaitCardButtons(pick: pick, onPick: { take(pick) }, onReview: review)
                        .accessibilityIdentifier(pickIdentifier(pick))
                } else {
                    Button("Review", action: review)
                        .buttonStyle(SignalButtonStyle(.onField, size: .small))
                        .accessibilityIdentifier("review:\(item.id)")
                }
            }
        }
        .overlay(alignment: .center) { flickHint }
        .background(GeometryReader { geo in Color.clear.onAppear { width = geo.size.width } })
        .offset(x: offset)
        .rotationEffect(.degrees(reduceMotion ? 0 : Double(offset / max(width, 1)) * 6 * 1.6), anchor: .bottom)
        .opacity(reduceMotion ? 1 - min(abs(offset) / max(width, 1), 0.6) : 1)
        .contentShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
        .onTapGesture(perform: review)
        .simultaneousGesture(flick)
        .disabled(busy)
        .contextMenu { menu }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier(group.isBurst ? "burst" : "pending:\(item.id)")
        .accessibilityActions {
            if let pick, pick.firm { Button(pick.title) { take(pick) } }
            if item.kind == .request { Button("Deny") { answer(approve: false) } }
        }
    }

    /// While the card is dragged: what letting go does, in words on the card.
    @ViewBuilder private var flickHint: some View {
        let fraction = offset / max(width, 1)
        if abs(fraction) > 0.12 {
            let right = fraction > 0
            Text(right ? (pick?.firm == true ? pick!.title : "Review") : (item.kind == .request ? "Deny" : "Review"))
                .font(TextRole.button.font())
                .foregroundStyle(Palette.onFieldInk)
                .padding(.horizontal, 18)
                .frame(height: 44)
                .background(.white, in: Capsule())
                .opacity(min((abs(fraction) - 0.12) * 4, 1))
                .allowsHitTesting(false)
        }
    }

    private var flick: some Gesture {
        DragGesture(minimumDistance: 14)
            .onChanged { drag in
                if sideways == nil { sideways = abs(drag.translation.width) > abs(drag.translation.height) }
                guard sideways == true, !busy else { return }
                offset = drag.translation.width
            }
            .onEnded { drag in
                defer { sideways = nil }
                guard sideways == true, !busy else { return }
                let fraction = drag.translation.width / max(width, 1)
                if fraction > 0.35, let pick, pick.firm {
                    flyOff(right: true) { take(pick) }
                } else if fraction < -0.35, item.kind == .request {
                    flyOff(right: false) { answer(approve: false) }
                } else {
                    let open = abs(fraction) > 0.35
                    withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) { offset = 0 }
                    if open { review() }
                }
            }
    }

    /// The card leaves (220 ms; a fade under Reduce Motion), then the answer goes out. If it does not (Face ID
    /// cancelled), the card comes back.
    private func flyOff(right: Bool, then answer: @escaping () -> Void) {
        withAnimation(Motion.of(.easeIn(duration: Motion.cardFly), reduce: reduceMotion)) {
            offset = (right ? 1 : -1) * width * 1.4
        }
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(Motion.cardFly))
            answer()
        }
    }

    private func pickIdentifier(_ pick: AutopilotPick) -> String {
        switch pick {
        case .allow: group.isBurst ? "approveAll" : "allow:\(item.id)"
        case .deny: group.isBurst ? "denyAll" : "deny:\(item.id)"
        case .unsure: "unsure:\(item.id)"
        case .alwaysAsks: "alwaysAsks:\(item.id)"
        }
    }

    /// A burst lists what is in it, one line each.
    private var peek: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(group.items.prefix(3), id: \.id) { i in
                Text(line(i)).font(RFont.sans(13)).foregroundStyle(Palette.onField2).lineLimit(1)
            }
            if group.items.count > 3 {
                Text("and \(group.items.count - 3) more").font(RFont.sans(13)).foregroundStyle(Palette.onField2)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private func line(_ i: PendingItem) -> String {
        let subtitle = untrusted(i.subtitle).trimmingCharacters(in: .whitespacesAndNewlines)
        return subtitle.isEmpty ? operationTitle(action: i.action, count: Int(i.count), service: i.service, title: i.opTitle, op: i.op) : subtitle
    }

    @ViewBuilder private var menu: some View {
        Button("Review", systemImage: "checkmark.shield", action: review)
        if group.isBurst {
            Button("Approve all", systemImage: BiometricSymbol.name) { answer(approve: true) }
            Button("Deny all", systemImage: "nosign") { answer(approve: false) }
        }
        Button("Copy", systemImage: "doc.on.doc") {
            UIPasteboard.general.string = ActivityText.ask(group).map(\.text).joined()
            feedback.play(.copied)
        }
    }

    private func review() {
        model.openSheet(item.sheetTarget)
        feedback.defaultTap()
    }

    /// Autopilot's pick: Allow approves (the owner's check first), Deny denies; anything else is a closer look.
    private func take(_ pick: AutopilotPick) {
        switch pick {
        case .allow: answer(approve: true)
        case .deny: answer(approve: false)
        case .unsure, .alwaysAsks: review()
        }
    }

    private func answer(approve: Bool) {
        busy = true
        let ids = group.items.map(\.id)
        Task {
            await Burst.answer(model, ids: ids, label: item.connectionLabel, approve: approve)
            busy = false
            // Still waiting (the check was cancelled): the card comes back.
            withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) { offset = 0 }
        }
    }
}
