import SwiftUI

/// Everything beyond "approve or deny": how long, how wide, all mail. Collapsed by default.
struct MoreOptions: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                vm.moreOpen.toggle()
                feedback.play(.expand(vm.moreOpen))
            } label: {
                HStack {
                    Text("More options").font(RFont.sans(15.5, .medium))
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.system(size: 14, weight: .semibold))
                        .rotationEffect(.degrees(vm.moreOpen ? 90 : 0))
                }
                .foregroundStyle(Palette.accent)
                .padding(.horizontal, 20)
                .padding(.vertical, 14)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("moreToggle")
            .accessibilityValue(vm.moreOpen ? "Open" : "Closed")

            if vm.moreOpen {
                VStack(alignment: .leading, spacing: 0) {
                    switch view.kind {
                    case .grant: ShortenGrant(vm: vm, view: view)
                    case .accounts: AccountsPeriod(vm: vm)
                    case .fetch, .write: ConnectorOptions(vm: vm, view: view)
                    case .search, .read, .send: StandardOptions(vm: vm, view: view)
                    }
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.top, 8)
    }
}

/// A small heading inside "More options".
private struct OptionTitle: View {
    var text: String
    var top: CGFloat = 14

    var body: some View {
        Text(text)
            .font(RFont.sans(13, .semibold))
            .foregroundStyle(Palette.secondary)
            .padding(.leading, 20)
            .padding(.top, top)
            .padding(.bottom, 8)
    }
}

private struct Note: View {
    var text: String

    var body: some View {
        Text(text).font(RFont.sans(13.5)).foregroundStyle(Palette.tertiary).padding(.horizontal, 20).padding(.top, 8)
    }
}

/// "Remember this for": once, a while, until revoked, or a number of uses (typed in).
private struct LifetimeChips: View {
    @Bindable var vm: ApprovalModel

    var body: some View {
        OptionTitle(text: "Remember this for")
        FlowRow {
            ForEach(LifetimeKind.remember) { kind in
                SelectChip(title: kind.label, selected: vm.draft.lifetime == kind) { vm.draft.lifetime = kind }
                    .accessibilityIdentifier("lifetime:\(kind.rawValue)")
            }
        }
        .padding(.horizontal, 16)
        if vm.draft.lifetime == .uses {
            UsesField(vm: vm)
        }
        if vm.draft.lifetime == .once {
            Note(text: "Only this request. Nothing is remembered.")
        }
    }
}

private struct UsesField: View {
    @Bindable var vm: ApprovalModel

    var body: some View {
        TextField("Number of uses (1–\(ApprovalRules.maxUses))", text: Binding(
            get: { String(vm.draft.uses) },
            set: { text in
                if let n = Int(String(text.filter(\.isNumber).prefix(4))) { vm.draft.uses = n }
            }
        ))
        .keyboardType(.numberPad)
        .font(RFont.mono(16))
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .accessibilityLabel("Number of uses")
        .accessibilityIdentifier("uses")
    }
}

/// "Allow all mail for a while" / "Allow all of Telegram for a while": a few time boxes, one tap each.
private struct EverythingCard: View {
    @Bindable var vm: ApprovalModel
    var title: String
    var text: String
    /// What gets ticked when it is chosen.
    var ticks: Set<String>

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "clock").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.accent)
                    Text(title).font(RFont.sans(16, .semibold)).foregroundStyle(Palette.text)
                }
                Text(text)
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .padding(.top, 6)
                    .padding(.bottom, 12)
                FlowRow {
                    ForEach(ApprovalRules.allMailLifetimes) { kind in
                        SelectChip(title: kind.label, selected: vm.draft.allMail == kind) {
                            if vm.draft.allMail == kind {
                                vm.draft.allMail = nil
                            } else {
                                vm.draft.allMail = kind
                                vm.draft.selected = ticks
                            }
                        }
                        .accessibilityIdentifier("allMail:\(kind.rawValue)")
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 6)
    }
}

/// Gmail searches, reads and sends.
private struct StandardOptions: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView

    var body: some View {
        let send = view.kind == .send
        if !send {
            EverythingCard(
                vm: vm,
                title: "Allow all mail for a while",
                text: "Release these and let this AI search and read any of your mail without asking again, then ask again when time is up. Sending is never included.",
                ticks: Set(view.messages.map(\.id))
            )
        }
        if vm.draft.allMail == nil {
            LifetimeChips(vm: vm)
            if vm.draft.lifetime != .once { ScopeBuilder(vm: vm, view: view) }
        }
    }
}

/// What a standing Gmail grant covers: the ticked emails or similar mail (senders, domains), or the recipients of a send.
private struct ScopeBuilder: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        OptionTitle(text: "What should it cover?", top: 18)
        GroupCard {
            if view.kind == .send {
                ForEach(Array(recipientAddresses(view).enumerated()), id: \.element) { i, address in
                    if i > 0 { Hairline() }
                    let wide = vm.draft.domainRecipients.contains(address)
                    Toggle(isOn: Binding(
                        get: { wide },
                        set: { on in
                            if on { vm.draft.domainRecipients.insert(address) } else { vm.draft.domainRecipients.remove(address) }
                            feedback.play(.toggle(on))
                        }
                    )) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(address).font(RFont.mono(14)).foregroundStyle(Palette.text).environment(\.layoutDirection, .leftToRight)
                            Text(wide ? "Anyone at @\(domainOf(address))" : "This address only")
                                .font(RFont.sans(12.5))
                                .foregroundStyle(Palette.secondary)
                        }
                    }
                    .tint(Palette.accent)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("domain:\(address)")
                }
            } else {
                CheckRow(checked: !vm.draft.similar, onChange: { _ in vm.draft.similar = false }) {
                    Text("Only the emails I ticked").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                }
                .padding(16)
                .accessibilityIdentifier("onlySelected")
                Hairline()
                CheckRow(checked: vm.draft.similar, onChange: { _ in vm.draft.similar = true }) {
                    Text("Also allow similar mail").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                }
                .padding(16)
                .accessibilityIdentifier("similar")
                if vm.draft.similar { similarRows }
            }
        }
        TextField("Subject contains (optional)", text: Binding(
            get: { vm.draft.subject },
            set: { vm.draft.subject = String($0.prefix(200)) }
        ))
        .font(RFont.sans(16))
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityIdentifier("subject")
    }

    @ViewBuilder private var similarRows: some View {
        let senders = senderAddresses(view, selected: vm.draft.selected)
        if senders.isEmpty {
            Text("Tick an email to offer its sender.")
                .font(RFont.sans(13))
                .foregroundStyle(Palette.tertiary)
                .padding(.leading, 16)
                .padding(.bottom, 12)
        }
        ForEach(senders, id: \.self) { address in
            Hairline()
            CheckRow(checked: vm.draft.senderAddresses.contains(address), onChange: { on in
                if on { vm.draft.senderAddresses.insert(address) } else { vm.draft.senderAddresses.remove(address) }
            }) {
                Text(address).font(RFont.mono(14)).foregroundStyle(Palette.text).environment(\.layoutDirection, .leftToRight)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
        }
        let domains = senders.map(domainOf).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
        ForEach(domains, id: \.self) { domain in
            Hairline()
            CheckRow(checked: vm.draft.senderDomains.contains(domain), onChange: { on in
                if on { vm.draft.senderDomains.insert(domain) } else { vm.draft.senderDomains.remove(domain) }
            }) {
                Text("Anyone at @\(domain)").font(RFont.sans(15)).foregroundStyle(Palette.text)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .accessibilityIdentifier("senderDomain:\(domain)")
        }
    }
}

/// How long to remember an approval in another integration and what it covers; a password is never remembered.
private struct ConnectorOptions: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView

    var body: some View {
        let write = view.kind == .write
        if view.noStanding {
            Text(write
                ? "This is asked for every time. A change like this is never remembered."
                : "This is asked for every time. Passwords, codes and one-time values are never remembered.")
                .font(RFont.sans(14))
                .foregroundStyle(Palette.secondary)
                .padding(.horizontal, 20)
                .padding(.vertical, 8)
                .accessibilityIdentifier("noStanding")
        } else {
            let service = serviceName(view.service)
            if !write {
                EverythingCard(
                    vm: vm,
                    title: "Allow all of \(service) for a while",
                    text: "This AI can list, read and search \(service) without asking again, then asks again when time is up. Changing things is never included.",
                    ticks: Set(view.messages.filter { !$0.sensitive }.map(\.id))
                )
            }
            if vm.draft.allMail == nil {
                LifetimeChips(vm: vm)
                if vm.draft.lifetime != .once {
                    if write && !view.classes.isEmpty { classPicker }
                    OptionTitle(text: "What should it cover?", top: 18)
                    let narrow = view.resources.filter { !$0.wider }
                    let wide = view.resources.filter(\.wider)
                    ResourceGroup(vm: vm, resources: narrow)
                    if !wide.isEmpty {
                        Text("Or a wider permission")
                            .font(RFont.sans(12.5, .medium))
                            .foregroundStyle(Palette.tertiary)
                            .padding(.leading, 20)
                            .padding(.top, 14)
                            .padding(.bottom, 8)
                            .accessibilityIdentifier("widerCaption")
                        ResourceGroup(vm: vm, resources: wide)
                    }
                }
            }
        }
    }

    @ViewBuilder private var classPicker: some View {
        OptionTitle(text: "Allow these kinds of change", top: 18)
        FlowRow {
            ForEach(view.classes, id: \.id) { kind in
                let on = vm.draft.classes.contains(kind.id)
                SelectChip(title: untrusted(kind.label), selected: on) {
                    vm.draft.classes = toggleClass(vm.draft.classes, kind.id, on: !on)
                }
                .accessibilityIdentifier("class:\(kind.id)")
            }
        }
        .padding(.horizontal, 16)
        Text("At least one stays ticked. Only what you tick is allowed without asking.")
            .font(RFont.sans(12.5))
            .foregroundStyle(Palette.tertiary)
            .padding(.leading, 20)
            .padding(.top, 6)
    }
}

/// The things a permission can cover, each with a tick; ticking one that contains or is inside another swaps them.
private struct ResourceGroup: View {
    @Bindable var vm: ApprovalModel
    var resources: [ResourceView]

    var body: some View {
        if !resources.isEmpty {
            GroupCard {
                ForEach(Array(resources.enumerated()), id: \.element.id) { i, resource in
                    if i > 0 { Hairline() }
                    CheckRow(checked: vm.draft.resources.contains(resource.id), onChange: { on in
                        vm.draft.resources = toggleResource(vm.draft.resources, resource.id, on: on)
                    }) {
                        Text(untrusted(resource.label)).font(RFont.sans(15.5)).foregroundStyle(Palette.text).lineLimit(2)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("resource:\(resource.id)")
                }
            }
        }
    }
}

/// Showing accounts: once, or for one of a few periods.
private struct AccountsPeriod: View {
    @Bindable var vm: ApprovalModel

    var body: some View {
        OptionTitle(text: "Let it see them for", top: 6)
        FlowRow {
            ForEach(ApprovalRules.accountsLifetimes) { kind in
                SelectChip(title: kind == .once ? "Just this once" : kind.label, selected: vm.draft.lifetime == kind) {
                    vm.draft.lifetime = kind
                }
                .accessibilityIdentifier("lifetime:\(kind.rawValue)")
            }
        }
        .padding(.horizontal, 16)
    }
}

/// A permission request can only be made shorter: a slider with a detent per step, up to what was asked.
private struct ShortenGrant: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        if let asked = view.grant.map({ Int64($0.durationSecs) }) {
            let steps = shortenSteps(asked: asked)
            let current = min(vm.draft.grantSeconds ?? asked, asked)
            let index = max(steps.lastIndex { $0 <= current } ?? 0, 0)
            VStack(alignment: .leading, spacing: 2) {
                Text("Allow for").font(RFont.sans(13, .semibold)).foregroundStyle(Palette.secondary)
                Text(TimeText.duration(steps[index]))
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(Palette.text)
                    .contentTransition(.numericText())
                    .accessibilityIdentifier("grantDuration")
                if steps.count > 1 {
                    Slider(
                        value: Binding(
                            get: { Double(index) },
                            set: { value in
                                let step = min(max(Int(value.rounded()), 0), steps.count - 1)
                                guard step != index else { return }
                                withAnimation(.snappy) { vm.draft.grantSeconds = steps[step] }
                                feedback.play(.detent, step: step)
                            }
                        ),
                        in: 0...Double(steps.count - 1),
                        step: 1
                    )
                    .tint(Palette.accent)
                    .accessibilityLabel("Allow for")
                    .accessibilityValue(TimeText.duration(steps[index]))
                    .accessibilityIdentifier("grantSlider")
                    Text("The request asked for \(TimeText.duration(asked)). You can only make it shorter.")
                        .font(RFont.sans(12.5))
                        .foregroundStyle(Palette.tertiary)
                }
            }
            .padding(.horizontal, 20)
            .padding(.vertical, 6)
        }
    }
}
