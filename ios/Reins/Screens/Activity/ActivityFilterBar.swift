import SwiftUI

// Activity's search controls: the round filter button that shows the chips, the chips (Outcome, Who, Kind, AI, When),
// each a Signal menu growing from it, and the search field.

/// The round filter button: shows or hides the row of filter chips; a dot when a filter is on.
struct ActivityFilterToggle: View {
    @Binding var shown: Bool
    var active: Bool
    @Environment(\.feedback) private var feedback

    var body: some View {
        // Inverse while its chips show (they also show whenever a filter is on).
        let shown = self.shown || active
        Button {
            self.shown.toggle()
            feedback.play(.expand(self.shown))
        } label: {
            GlyphIcon(.filter, size: 20)
                .foregroundStyle(shown ? Palette.onInverse : Palette.ink)
                .frame(width: 40, height: 40)
                .background(shown ? Palette.inverse : Palette.paperRaised, in: Circle())
                .overlay {
                    if !shown { Circle().strokeBorder(Palette.rule, lineWidth: 1) }
                }
                .overlay(alignment: .topTrailing) {
                    if active {
                        Circle().fill(Palette.violetFill).frame(width: 9, height: 9).offset(x: -2, y: 2)
                    }
                }
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(active ? "Filters, on" : "Filters")
        .accessibilityValue(shown ? "Shown" : "Hidden")
        .accessibilityIdentifier("filterToggle")
    }
}

/// The row of filter chips under the search field, each a Signal menu, and a clear-all button while any is on. A chip
/// that is on is ink with its value; Outcome, Who, Kind and AI pick several, When one.
struct ActivityFilterBar: View {
    @Binding var filters: ActivityFilters
    var facets: ActivityFacets?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                who
                outcome
                if !kindChoices.services.isEmpty || !kindChoices.actions.isEmpty { kind }
                if !aiChoices.isEmpty { ai }
                when
                if !filters.isEmpty { clear }
            }
            .padding(.horizontal, 20)
        }
        .scrollClipDisabled()
    }

    private func change(_ body: (inout ActivityFilters) -> Void) {
        var next = filters
        body(&next)
        withAnimation(Motion.of(.smooth(duration: 0.2), reduce: reduceMotion)) { filters = next }
    }

    private var outcome: some View {
        let items = [SignalMenuItem(id: "outcome:any", title: "Any outcome", glyph: .activity)]
            + ActivityFilters.outcomeOrder.enumerated().map { i, o in
                SignalMenuItem(id: ActivitySearchText.id(o), title: ActivitySearchText.outcome(o), glyph: Self.glyph(o), separated: i == 0)
            }
        return FilterMenuChip(
            title: ActivitySearchText.chip("Outcome", filters.outcomes.map(ActivitySearchText.outcome)), active: !filters.outcomes.isEmpty,
            menu: "outcomeMenu", items: items,
            chosen: filters.outcomes.isEmpty ? ["outcome:any"] : Set(filters.outcomes.map(ActivitySearchText.id)), multiple: true,
            label: "Outcome", identifier: "filter:outcome"
        ) { id in
            change { f in
                if let o = ActivityFilters.outcomeOrder.first(where: { ActivitySearchText.id($0) == id }) { f.toggle(o) } else { f.outcomes = [] }
            }
        }
    }

    private var who: some View {
        let items = [SignalMenuItem(id: "who:any", title: "Anyone", glyph: .activity)]
            + ActivityFilters.deciderOrder.enumerated().map { i, d in
                SignalMenuItem(id: ActivitySearchText.id(d), title: ActivitySearchText.decider(d), glyph: Self.glyph(d), separated: i == 0)
            }
        return FilterMenuChip(
            title: ActivitySearchText.chip("Who", filters.deciders.map(ActivitySearchText.decider)), active: !filters.deciders.isEmpty,
            menu: "whoMenu", items: items,
            chosen: filters.deciders.isEmpty ? ["who:any"] : Set(filters.deciders.map(ActivitySearchText.id)), multiple: true,
            label: "Who decided", identifier: "filter:who", lead: filters.deciders.count == 1 ? Self.glyph(filters.deciders[0]) : nil
        ) { id in
            change { f in
                if let d = ActivityFilters.deciderOrder.first(where: { ActivitySearchText.id($0) == id }) { f.toggle(d) } else { f.deciders = [] }
            }
        }
    }

    /// What the history holds, and what is chosen even if it no longer does.
    private var kindChoices: (services: [String], actions: [String]) {
        (Self.union(facets?.services ?? [], filters.services), Self.union(facets?.actions ?? [], filters.actions))
    }

    private var kind: some View {
        let (services, actions) = kindChoices
        let items = [SignalMenuItem(id: "kind:any", title: "Any kind", glyph: .activity)]
            + services.enumerated().map { i, s in
                SignalMenuItem(id: "kind:s:\(s)", title: serviceName(s), glyph: ActivitySearchText.glyph(service: s), separated: i == 0)
            }
            + actions.enumerated().map { i, a in
                SignalMenuItem(id: "kind:a:\(a)", title: ActivitySearchText.kind(a), glyph: ActivitySearchText.glyph(action: a), separated: i == 0)
            }
        let names = filters.services.map(serviceName) + filters.actions.map(ActivitySearchText.kind)
        let chosen = Set(filters.services.map { "kind:s:\($0)" } + filters.actions.map { "kind:a:\($0)" })
        return FilterMenuChip(
            title: ActivitySearchText.chip("Kind", names), active: !names.isEmpty, menu: "kindMenu", items: items,
            chosen: chosen.isEmpty ? ["kind:any"] : chosen, multiple: true, label: "Kind", identifier: "filter:kind"
        ) { id in
            change { f in
                if id.hasPrefix("kind:s:") {
                    f.toggle(service: String(id.dropFirst("kind:s:".count)))
                } else if id.hasPrefix("kind:a:") {
                    f.toggle(action: String(id.dropFirst("kind:a:".count)))
                } else {
                    f.services = []
                    f.actions = []
                }
            }
        }
    }

    /// The AIs in the history, and any chosen that no longer is.
    private var aiChoices: [ActivityConnection] {
        let known = facets?.connections ?? []
        return known + filters.connections.filter { id in !known.contains { $0.id == id } }.map { ActivityConnection(id: $0, label: "") }
    }

    private var ai: some View {
        let all = aiChoices
        let items = [SignalMenuItem(id: "ai:", title: "Any AI", glyph: .activity)]
            + all.enumerated().map { i, c in SignalMenuItem(id: "ai:\(c.id)", title: Self.name(c), glyph: .person, separated: i == 0) }
        let names = filters.connections.map { id in all.first { $0.id == id }.map(Self.name) ?? "Unnamed AI" }
        return FilterMenuChip(
            title: ActivitySearchText.chip("Any AI", names), active: !names.isEmpty, menu: "aiMenu", items: items,
            chosen: filters.connections.isEmpty ? ["ai:"] : Set(filters.connections.map { "ai:\($0)" }), multiple: true,
            label: "AI", identifier: "filter:ai"
        ) { id in
            change { f in
                let connection = Self.connection(id)
                if connection.isEmpty { f.connections = [] } else { f.toggle(connection: connection) }
            }
        }
    }

    private var when: some View {
        FilterMenuChip(
            title: filters.when == .any ? "When" : filters.when.title, active: filters.when != .any, menu: "whenMenu",
            items: ActivityWhen.allCases.map { w in
                SignalMenuItem(id: "when:\(w.rawValue)", title: w.title, glyph: w == .any ? .activity : .calendar, separated: w == .today)
            },
            chosen: ["when:\(filters.when.rawValue)"], multiple: false, label: "When", identifier: "filter:when"
        ) { id in
            change { $0.when = ActivityWhen(rawValue: String(id.dropFirst("when:".count))) ?? .any }
        }
    }

    private var clear: some View {
        Button {
            change { $0 = ActivityFilters() }
            feedback.play(.tap)
        } label: {
            GlyphIcon(.close, size: 14)
                .foregroundStyle(Palette.ink)
                .frame(width: 34, height: 34)
                .background(Palette.paperRaised, in: Circle())
                .overlay(Circle().strokeBorder(Palette.rule, lineWidth: 1))
                .contentShape(Circle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel("Clear filters")
        .accessibilityIdentifier("filter:clear")
        .transition(.opacity)
    }

    static func glyph(_ o: ActivityOutcome) -> Glyph {
        switch o {
        case .approved: .check
        case .denied: .block
        case .failed: .warning
        }
    }

    static func glyph(_ d: ActivityDecider) -> Glyph {
        switch d {
        case .you: .person
        case .autopilot: .autopilot
        case .bypass: .autopilotOff
        case .lockdown: .lock
        }
    }

    /// An AI's name, untrusted, one line.
    static func name(_ c: ActivityConnection) -> String {
        let name = untrustedName(c.label, limit: 40)
        return name.isEmpty ? "Unnamed AI" : name
    }

    /// The connection an AI menu row stands for ("" for any).
    static func connection(_ menuId: String) -> String { String(menuId.dropFirst("ai:".count)) }

    private static func union(_ a: [String], _ b: [String]) -> [String] { a + b.filter { !a.contains($0) } }
}

/// A filter chip with a menu ("Outcome", "Denied +1", with a chevron): raised with a hairline ring, ink with its value
/// while on.
struct FilterMenuChip: View {
    var title: String
    var active: Bool
    var menu: String
    var items: [SignalMenuItem]
    var chosen: Set<String>
    /// Picks several: the menu stays open and each row ticks.
    var multiple: Bool
    /// What the chip filters, for VoiceOver.
    var label: String
    var identifier: String
    /// The glyph of the value it is set to (Autopilot's bolt).
    var lead: Glyph? = nil
    var onPick: (String) -> Void

    var body: some View {
        // The kit's chip, 40 pt: set, it is ink-inverse with its value (and its glyph) and a cross; unset, a chevron.
        HStack(spacing: 6) {
            if active, let lead { GlyphIcon(lead, size: 14) }
            Text(title).lineLimit(1)
            GlyphIcon(active ? .close : .chevronDown, size: 13)
        }
        .font(RFont.sans(14, .semibold))
        .foregroundStyle(active ? Palette.onInverse : Palette.ink)
        .padding(.horizontal, 13)
        .frame(height: 40)
        .background(active ? Palette.inverse : Palette.paperRaised, in: Capsule())
        .overlay {
            if !active { Capsule().strokeBorder(Palette.rule, lineWidth: 1) }
        }
        .signalMenu(
            id: menu, items: items, selected: multiple ? nil : chosen.first, chosen: multiple ? chosen : [], staysOpen: multiple,
            identifier: menu, onPick: onPick
        )
        .accessibilityLabel(active ? "\(label): \(title)" : label)
        .accessibilityAddTraits(active ? .isSelected : [])
        .accessibilityIdentifier(identifier)
    }
}

/// The search field under the title: sunken paper, the magnifier, "Search", and a clear button once something is typed.
struct ActivitySearchField: View {
    @Binding var text: String
    /// Takes the keyboard as it appears (the header's search bar).
    var focusOnAppear = false
    @FocusState private var focused: Bool
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 10) {
            GlyphIcon(.search, size: 18)
                .foregroundStyle(Palette.inkMeta)
                .accessibilityHidden(true)
            TextField(text: $text, prompt: Text("Search").foregroundStyle(Palette.inkMeta)) { Text("Search") }
                .font(RFont.sans(16))
                .foregroundStyle(Palette.ink)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.search)
                .focused($focused)
                .onSubmit { focused = false }
                .accessibilityIdentifier("activitySearch")
            if !text.isEmpty {
                Button {
                    text = ""
                    feedback.play(.tap)
                } label: {
                    GlyphIcon(.circleClose, size: 18)
                        .foregroundStyle(Palette.inkMeta)
                        .frame(width: 32, height: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel("Clear search")
                .accessibilityIdentifier("clearSearch")
            }
        }
        .padding(.leading, 14)
        .padding(.trailing, 6)
        .frame(height: 44)
        .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        .onTapGesture { focused = true }
        .onAppear { if focusOnAppear { focused = true } }
    }
}
