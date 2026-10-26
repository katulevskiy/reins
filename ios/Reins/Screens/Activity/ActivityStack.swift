import SwiftUI

// The stack (Next-Activity, 01): what waits, as cards in their field colour, the newest on top and the rest peeking
// below. Swipe a card right to take Autopilot's pick, left to deny, tap it to review. The answers go through the same
// paths as the cards' buttons (Burst.answer), with the same feedback, played at the decision.

/// Activity in words (pure, unit-tested).
enum ActivityText {
    private static func capitalized(_ s: String) -> String { s.prefix(1).uppercased() + s.dropFirst() }

    /// An entry that went through (not denied, not failed) reads in the past.
    static func done(_ entry: ActivityEntry) -> Bool { ["released", "sent", "granted"].contains(entry.outcome) }

    /// The label line of a waiting card: "Gmail · Send" (set in mono capitals), from the core's provider and action.
    static func label(_ item: PendingItem) -> String { AskLabels.pending(item, field: field(item, danger: false)).kind }

    /// The ask on a waiting card (ASK-TEXT): the shared formatter's sentence, one italic target; a burst of several
    /// asks says how many.
    static func ask(_ group: WaitGroup, line: AskLine? = nil) -> [AskText.Run] {
        if !group.isBurst {
            return (line ?? AskLabels.pending(group.first, field: field(group.first, danger: false))).runs
        }
        return runs(WantsText.of(burst: group.items))
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

/// Who asks, as a waiting card shows it beside its icon (pure, unit-tested; security review rule 4): the other side's
/// own name, one short clean line in quotes, never part of Reins' sentence. A card offers one-tap Allow (its button
/// and the swipe right) only while it can show that name: a card that cannot say who asks sends you to Review.
enum WaitCardIdentity {
    /// At most this many characters of the name.
    static let limit = 28

    /// "“Claude Code”", nil when the name is empty after cleaning. A pairing or a new phone shows its name in the
    /// card's mono line instead (quoted there).
    static func who(_ item: PendingItem) -> String? {
        guard item.kind == .request || item.kind == .blob else { return nil }
        let name = untrustedName(item.connectionLabel, limit: limit)
        return name.isEmpty ? nil : "“\(name)”"
    }

    /// One tap may approve only when who asks is on the card.
    static func allowsOneTap(_ item: PendingItem, pick: AutopilotPick?) -> Bool {
        guard case .allow = pick else { return true }
        return who(item) != nil
    }

    /// The pick a card offers: Autopilot's, unless it is Allow on a card that cannot say who asks.
    static func pick(_ item: PendingItem, _ pick: AutopilotPick?) -> AutopilotPick? {
        allowsOneTap(item, pick: pick) ? pick : nil
    }
}

/// The waiting cards as a stack (Next-Activity, 01): the newest on top, the next two as edges of their own colour
/// below it and "+N" for how many more; a tap on the edges or on +N opens the stack in place, one card under another.
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
                if groups.count > 1 {
                    Button { toggle() } label: {
                        GlyphIcon(.chevronUp, size: 16)
                            .foregroundStyle(Palette.ink2)
                            .frame(width: 44, height: 32)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityLabel("Show one")
                    .accessibilityIdentifier("stackToggle")
                }
            } else if let top = groups.first {
                let peeks = min(groups.count - 1, 2)
                card(top)
                    .background(alignment: .bottom) { edges }
                    .overlay(alignment: .bottomTrailing) {
                        if groups.count > 1 {
                            Button { toggle() } label: {
                                Text("+\(groups.count - 1)")
                                    .font(RFont.sans(12, .bold))
                                    .monospacedDigit()
                                    .foregroundStyle(Palette.onInverse)
                                    .padding(.horizontal, 7)
                                    .frame(minWidth: 22, minHeight: 22)
                                    .background(Palette.inverse, in: Capsule())
                                    .frame(minWidth: 44, minHeight: 44)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(PressDimsStyle())
                            // The badge's centre sits on the lowest edge, 18 pt in from the right.
                            .offset(x: -7, y: CGFloat(8 * peeks) + 22)
                            .accessibilityLabel("\(groups.count - 1) more waiting")
                            .accessibilityHint("Shows them all")
                            .accessibilityIdentifier("stackToggle")
                        }
                    }
                    .padding(.bottom, CGFloat(8 * peeks + 6))
            }
        }
        .padding(.horizontal, 16)
    }

    private func toggle() {
        withAnimation(Motion.of(Motion.easeOut(Motion.fold), reduce: reduceMotion)) { expanded.toggle() }
        feedback.play(.expand(expanded))
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

    /// The next cards' bottom edges, a little narrower and darker each, in their own colours; a tap opens the stack.
    private var edges: some View {
        ZStack(alignment: .bottom) {
            ForEach(Array(groups.dropFirst().prefix(2).enumerated().reversed()), id: \.element.id) { i, group in
                let field = ActivityText.field(group.first, danger: group.items.contains { suggestions[$0.id]?.floor == true })
                UnevenRoundedRectangle(bottomLeadingRadius: 22, bottomTrailingRadius: 22, style: .continuous)
                    .fill(field.fill)
                    .brightness(-0.12 * Double(i + 1))
                    .frame(height: 40)
                    .padding(.horizontal, CGFloat(i + 1) * 10)
                    .offset(y: CGFloat(i + 1) * 8)
            }
        }
        .contentShape(Rectangle())
        .onTapGesture { toggle() }
        .accessibilityHidden(true)
    }
}

/// A waiting request, connection, new phone or file as a card in its field (Next-Activity, 01): the request's icon,
/// who asks (avatar and quoted name, one line), the time left; the ask; the target in mono; then Autopilot's pick and
/// Review. Follows the thumb sideways, tilting up to 6°; past 35 % it flies off and answers: right takes Autopilot's
/// pick (Allow, or Deny), left denies a request. Anything else springs back, and a pick that is not one (Unsure,
/// Always asks) opens the review instead. One-tap Allow, by button or swipe, only while who asks shows.
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
    @State private var width: CGFloat = 360

    private var item: PendingItem { group.first }
    private var field: FieldKind { ActivityText.field(item, danger: danger) }
    private var pick: AutopilotPick? { WaitCardIdentity.pick(item, ActivityText.pick(advice, suggestion)) }

    var body: some View {
        let line = AskLabels.pending(item, field: field)
        WaitCardShell(
            kind: field, label: line.kind, ask: AskText(ActivityText.ask(group, line: line), role: .cardAsk),
            askIdentifier: "pendingTitle:\(item.id)"
        ) {
            RequestIcon(spec: line.icon, size: 34, ring: field.fill)
                .accessibilityIdentifier("cardIcon:\(item.id)")
        } timer: {
            HStack(spacing: 10) {
                // Who asks, beside the icon: the avatar and the quoted name, one capped line.
                if let who = WaitCardIdentity.who(item) {
                    HStack(spacing: 7) {
                        ConnectionIcon(connectionId: item.connectionId, label: item.connectionLabel, size: 26)
                            .environment(\.colorScheme, .light)
                        Text(verbatim: who)
                            .font(RFont.sans(14, .semibold))
                            .foregroundStyle(Palette.onField)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("From \(who)")
                    .accessibilityIdentifier("cardWho:\(item.id)")
                    .layoutPriority(1)
                }
                Spacer(minLength: 0)
                TimerPill(createdAt: item.createdAt, waitUntil: item.kind == .blob ? nil : group.items.compactMap(\.waitUntil).min(), onField: true)
                    .fixedSize()
            }
            // The head row keeps its size at accessibility text sizes, so who asks always fits beside the icon.
            .dynamicTypeSize(...DynamicTypeSize.xxLarge)
        } actions: {
            VStack(alignment: .leading, spacing: 12) {
                // Machine details under the ask (a command, a branch, an address), never in the display face.
                if !group.isBurst, let mono = line.mono ?? ActivityText.detail(item) {
                    Text(verbatim: mono)
                        .font(RFont.mono(13))
                        .foregroundStyle(Palette.onField2)
                        .lineLimit(3)
                        .dynamicTypeSize(...DynamicTypeSize.accessibility1)
                        .environment(\.layoutDirection, .leftToRight)
                        .padding(.top, -4)
                        .accessibilityIdentifier("cardDetails:\(item.id)")
                }
                if group.isBurst { peek }
                if let pick {
                    WaitCardButtons(pick: pick, onPick: { pressed { take(pick) } }, onReview: { pressed(review) })
                        .accessibilityIdentifier(pickIdentifier(pick))
                } else {
                    Button("Review") { pressed(review) }
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
        // A tap holds no scroll back (it is a tap only when the finger lifts without moving); the flick begins only
        // for clearly sideways movement (SidewaysPan), so a scroll that starts on the card scrolls.
        .onTapGesture { pressed(review) }
        .gesture(flick)
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

    private var flick: SidewaysPan {
        SidewaysPan(isEnabled: !busy, onChanged: { drag in
            guard !busy else { return }
            offset = drag.translation.width
        }, onEnded: { drag, cancelled in
            guard !busy else { return }
            // A flick the system took away springs back.
            let fraction = cancelled ? 0 : drag.translation.width / max(width, 1)
            if fraction > 0.35, let pick, pick.firm {
                flyOff(right: true) { take(pick) }
            } else if fraction < -0.35, item.kind == .request {
                flyOff(right: false) { answer(approve: false) }
            } else {
                let open = abs(fraction) > 0.35
                withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) { offset = 0 }
                if open { review() }
            }
        })
    }

    /// A tap on the card or on one of its buttons, unless it is the finger lifting at the end of a flick.
    private func pressed(_ action: () -> Void) {
        guard !SidewaysPan.swiping else { return }
        action()
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
            // Approving without opening it needs who asks on the card, as one-tap Allow does.
            if WaitCardIdentity.who(item) != nil {
                Button("Approve all", systemImage: BiometricSymbol.name) { answer(approve: true) }
            }
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
