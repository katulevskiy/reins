import SwiftUI

// The approval screen's sheets for how long and how wide (the Android app's ApprovalSheets.kt; Next-Approve 04):
// "Remember" behind the dock's Custom ▾, durations as rows and what a remembered approval covers under "Covers", and
// a permission's length.

/// "Remember": a week, 30 days, a custom time, a number of uses and, in red, until revoked, each a row with its radio;
/// under "Covers", this one or anything like it (and, behind More, exactly what), then one button. The choice shows in
/// the dock's last segment.
struct RememberSheet: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback
    @State private var pending: LifetimeKind?
    @State private var more = false

    var body: some View {
        let forever = pending == .untilRevoked && vm.draft.allMail == nil
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 12) {
                    Text("Remember").textRole(.cardAsk).foregroundStyle(Palette.ink).accessibilityAddTraits(.isHeader)
                    Spacer()
                    RoundHeaderButton(glyph: .close, label: "Close") { dismiss() }
                        .accessibilityIdentifier("closeOverlay")
                }
                .padding(.horizontal, 20)
                .padding(.top, 22)
                .padding(.bottom, 4)
                FactGroup {
                    ForEach(ApprovalWords.sheetLifetimes(view)) { kind in
                        LifetimeOption(kind: kind, selected: pending == kind && vm.draft.allMail == nil, vm: vm) { pending = kind }
                    }
                    if ApprovalWords.offersUntilRevoked(view) {
                        OptionRow(selected: forever, id: "lifetime:untilRevoked", tint: Palette.danger) {
                            pending = .untilRevoked
                        } label: {
                            HStack(spacing: 12) {
                                GlyphIcon(.infinity, size: 18).foregroundStyle(Palette.danger)
                                Text("Until I revoke").font(RFont.sans(16, .medium)).foregroundStyle(Palette.danger)
                            }
                        }
                    }
                }
                if forever {
                    HStack(alignment: .top, spacing: 10) {
                        GlyphIcon(.warning, size: 16).foregroundStyle(Palette.danger).padding(.top, 1)
                        Text("Anything like this runs without asking, forever.").font(RFont.sans(13.5, .semibold)).foregroundStyle(Palette.danger)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .background(Palette.danger.opacity(0.10), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .padding(.horizontal, 16)
                    .transition(.opacity.combined(with: .move(edge: .top)))
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("foreverWarning")
                }
                if view.kind != .accounts {
                    FactHeader(text: "Covers", glyph: .key) {
                        Button {
                            withAnimation(.smooth) { more.toggle() }
                            feedback.play(.expand(more))
                        } label: {
                            HStack(spacing: 3) {
                                Text("More")
                                GlyphIcon(.chevronDown, size: 13).rotationEffect(.degrees(more ? 180 : 0))
                            }
                            .font(RFont.sans(13, .semibold))
                            .foregroundStyle(Palette.violet)
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("moreToggle")
                        .accessibilityValue(more ? "Open" : "Closed")
                    }
                    .padding(.top, 8)
                    .padding(.vertical, -11)
                    CoversChooser(vm: vm, view: view).padding(.horizontal, 16)
                    if more { ScopeOptions(vm: vm, view: view).transition(.opacity) }
                }
            }
            .padding(.bottom, 12)
            .animation(.smooth(duration: 0.25), value: forever)
        }
        .safeAreaInset(edge: .bottom, spacing: 0) { confirm(forever: forever).padding(.horizontal, 16).padding(.top, 10).padding(.bottom, 8).background(Palette.paper) }
        .background(Palette.paper.ignoresSafeArea())
        .presentationBackground(Palette.paper)
        .presentationDetents([.fraction(0.72), .large])
        // What the dock had, else the first row: one tap on the button remembers it.
        .onAppear { if pending == nil { pending = vm.draft.lifetime != .once ? vm.draft.lifetime : ApprovalWords.sheetLifetimes(view).first } }
        .accessibilityContainer("rememberSheet")
    }

    @ViewBuilder private func confirm(forever: Bool) -> some View {
        if forever {
            VStack(spacing: 8) {
                HoldButton(title: "Remember until I revoke", symbol: nil) { apply() }
                    .accessibilityIdentifier("rememberConfirm")
                Text(HoldButton.hint).font(RFont.sans(12)).foregroundStyle(Palette.inkMeta)
            }
        } else {
            let label: String = if vm.draft.allMail != nil {
                "Done"
            } else if let pending {
                ApprovalWords.rememberButton(view, { var d = vm.draft; d.lifetime = pending; return d }())
            } else {
                "Choose how long"
            }
            let enabled = vm.draft.allMail != nil || pending != nil
            Button { apply() } label: { Text(label) }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .disabled(!enabled)
                .accessibilityIdentifier("rememberConfirm")
        }
    }

    private func apply() {
        if let pending {
            vm.draft.lifetime = pending
            if vm.draft.allMail != nil && pending != .once { vm.draft.allMail = nil }
        }
        dismiss()
    }
}

/// Covers: this one, or anything like it ("This push" · "Pushes like it"), as the kit's chooser. Anything like it is
/// wider than the screen, so the dock then says so.
private struct CoversChooser: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        if let quick = view.quick, quick.allow != nil {
            let like = vm.draft.likeThis && vm.draft.allMail == nil
            VStack(alignment: .leading, spacing: 6) {
                Chooser([
                    Chooser.Segment(id: "scope:picked", title: words.0, selected: !like) {
                        vm.draft.likeThis = false
                        vm.draft.allMail = nil
                        feedback.play(.selection)
                    },
                    Chooser.Segment(id: "scope:like", title: words.1, selected: like) {
                        vm.draft.likeThis = true
                        vm.draft.allMail = nil
                        feedback.play(.selection)
                    },
                ])
                if like {
                    Text("\(untrusted(quick.allowWhat)), by \(untrustedName(view.connectionLabel))")
                        .font(RFont.sans(12.5, .medium)).foregroundStyle(Palette.warning)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, 8)
                        .accessibilityIdentifier("allowWhat")
                }
            }
        }
    }

    /// "This push" · "Pushes like it", by kind.
    private var words: (String, String) {
        if view.git != nil { return ("This push", "Pushes like it") }
        switch view.kind {
        case .send: return ("This email", "Emails like it")
        case .search, .read, .fetch: return ("These", "Like these")
        default: return ("Only this", "Like this")
        }
    }
}

/// A radio row (the kit's: a glyph, a title, what it needs, the radio at the right).
private struct OptionRow<Label: View>: View {
    var selected: Bool
    var id: String
    var tint: Color = Palette.violetFill
    var onSelect: () -> Void
    @ViewBuilder var label: Label
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 10) {
            label.frame(maxWidth: .infinity, alignment: .leading)
            RadioDot(selected: selected, tint: tint)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
        .frame(minHeight: 52)
        .contentShape(Rectangle())
        .onTapGesture {
            onSelect()
            feedback.play(.selection)
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction { onSelect() }
        .accessibilityIdentifier(id)
    }
}

private struct LifetimeOption: View {
    var kind: LifetimeKind
    var selected: Bool
    @Bindable var vm: ApprovalModel
    var onSelect: () -> Void

    var body: some View {
        OptionRow(selected: selected, id: "lifetime:\(kind.rawValue)", onSelect: onSelect) {
            HStack(spacing: 6) {
                Group {
                    // The set has no clock: the system's, at the glyphs' weight.
                    if let glyph { GlyphIcon(glyph, size: 18) } else { Image(systemName: "clock").font(.system(size: 16, weight: .regular)) }
                }
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 18)
                .padding(.trailing, 6)
                Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
                switch kind {
                case .custom:
                    CountStepper(value: $vm.draft.customAmount, in: 1...vm.draft.customUnit.max, id: "custom") { onSelect() }
                    Menu {
                        ForEach(LifetimeUnit.allCases) { unit in
                            Button(unit.rawValue) {
                                vm.draft.customUnit = unit
                                vm.draft.customAmount = min(vm.draft.customAmount, unit.max)
                                onSelect()
                            }
                        }
                    } label: {
                        HStack(spacing: 4) {
                            Text(vm.draft.customUnit.rawValue).font(RFont.sans(13.5, .semibold))
                            GlyphIcon(.chevronDown, size: 12)
                        }
                        .foregroundStyle(Palette.ink)
                        .padding(.horizontal, 10)
                        .frame(height: 34)
                        .background(Palette.paperRaised, in: Capsule())
                        .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
                    }
                    .padding(.leading, 6)
                    .accessibilityIdentifier("custom:unit")
                case .uses:
                    CountStepper(value: $vm.draft.uses, in: 1...ApprovalRules.maxUses, id: "uses") { onSelect() }
                default:
                    EmptyView()
                }
            }
        }
    }

    private var glyph: Glyph? {
        switch kind {
        case .custom: .timer
        case .uses: .token
        default: nil
        }
    }

    private var title: String {
        switch kind {
        case .week: "1 week"
        case .month: "30 days"
        case .custom: "Custom"
        case .uses: "Uses"
        default: kind.label
        }
    }
}

/// − n + on the sunken capsule (the kit's stepper).
private struct CountStepper: View {
    @Binding var value: Int
    var range: ClosedRange<Int>
    var id: String
    var onChange: () -> Void
    @Environment(\.feedback) private var feedback

    init(value: Binding<Int>, in range: ClosedRange<Int>, id: String, onChange: @escaping () -> Void) {
        _value = value
        self.range = range
        self.id = id
        self.onChange = onChange
    }

    var body: some View {
        HStack(spacing: 4) {
            step(.minus, "Fewer", value > range.lowerBound) { value -= 1 }
                .accessibilityIdentifier("\(id):minus")
            Text("\(value)").font(RFont.mono(14)).foregroundStyle(Palette.ink).frame(minWidth: 14).monospacedDigit()
                .accessibilityIdentifier("\(id):value")
            step(.plus, "More", value < range.upperBound) { value += 1 }
                .accessibilityIdentifier("\(id):plus")
        }
        .padding(.horizontal, 4)
        .frame(height: 34)
        .background(Palette.paperSunken, in: Capsule())
        .accessibilityElement(children: .contain)
    }

    private func step(_ glyph: Glyph, _ label: String, _ enabled: Bool, _ change: @escaping () -> Void) -> some View {
        Button {
            change()
            onChange()
            feedback.play(.detent, step: value)
        } label: {
            GlyphIcon(glyph, size: 14)
                .foregroundStyle(enabled ? Palette.ink : Palette.inkFaint)
                .frame(width: 26, height: 34)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

/// What a remembered approval covers: anything like this (the default), or exactly what is ticked here.
private struct ScopeOptions: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.feedback) private var feedback

    var body: some View {
        if let release = view.envRelease {
            SecretsScopeOptions(vm: vm, release: release)
        } else {
            options
        }
    }

    private var options: some View {
        let write = view.kind == .write
        let send = view.kind == .send
        let gmail = [ApprovalKind.search, .read, .send].contains(view.kind)
        return VStack(alignment: .leading, spacing: 10) {
            if !write && !send { everything }
            if vm.draft.allMail == nil {
                if gmail {
                    gmailScope
                } else {
                    if write && !view.classes.isEmpty {
                        FactHeader("Kinds of change")
                        FlowRow {
                            ForEach(view.classes, id: \.id) { kind in
                                let on = vm.draft.classes.contains(kind.id)
                                SelectChip(title: untrusted(kind.label), selected: on) {
                                    vm.draft.likeThis = false
                                    vm.draft.classes = toggleClass(vm.draft.classes, kind.id, on: !on)
                                }
                                .accessibilityIdentifier("class:\(kind.id)")
                            }
                        }
                        .padding(.horizontal, 16)
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("classes")
                    }
                    resources(view.resources.filter { !$0.wider })
                    let wide = view.resources.filter(\.wider)
                    if !wide.isEmpty {
                        FactHeader("Or wider").accessibilityIdentifier("widerCaption")
                        resources(wide)
                    }
                }
            }
        }
    }

    /// All mail (or all of an integration) for a while: never open-ended, never for changing things.
    private var everything: some View {
        VStack(alignment: .leading, spacing: 8) {
            let gmail = view.service.isEmpty || view.service == "gmail"
            FactHeader(gmail ? "All mail for" : "All of \(serviceName(view.service)) for")
            FlowRow {
                ForEach(ApprovalRules.allMailLifetimes) { kind in
                    SelectChip(title: ApprovalWords.period(kind.seconds ?? 3_600), selected: vm.draft.allMail == kind) {
                        if vm.draft.allMail == kind {
                            vm.draft.allMail = nil
                        } else {
                            vm.draft.allMail = kind
                            vm.draft.selected = Set(view.messages.filter { !$0.sensitive }.map(\.id))
                        }
                    }
                    .accessibilityIdentifier("allMail:\(kind.rawValue)")
                }
            }
            .padding(.horizontal, 16)
        }
    }

    @ViewBuilder private func resources(_ list: [ResourceView]) -> some View {
        if !list.isEmpty {
            FactGroup {
                ForEach(list, id: \.id) { r in
                    CheckRow(checked: vm.draft.resources.contains(r.id), onChange: { on in
                        vm.draft.likeThis = false
                        vm.draft.resources = toggleResource(vm.draft.resources, r.id, on: on)
                    }) {
                        Text(untrusted(r.label)).font(RFont.sans(15.5)).foregroundStyle(Palette.ink)
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("resource:\(r.id)")
                }
            }
        }
    }

    /// Gmail: only the ticked emails, or similar mail (senders, domains, subject); a send: each recipient, or its domain.
    @ViewBuilder private var gmailScope: some View {
        if view.kind == .send {
            FactGroup {
                ForEach(recipientAddresses(view), id: \.self) { address in
                    let wide = vm.draft.domainRecipients.contains(address)
                    Toggle(isOn: Binding(
                        get: { wide },
                        set: { on in
                            vm.draft.likeThis = false
                            if on { vm.draft.domainRecipients.insert(address) } else { vm.draft.domainRecipients.remove(address) }
                            feedback.play(.toggle(on))
                        }
                    )) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(untrusted(address)).font(RFont.mono(14)).foregroundStyle(Palette.ink).environment(\.layoutDirection, .leftToRight)
                            Text(wide ? "Anyone at @\(domainOf(address))" : "This address only").font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                        }
                    }
                    .tint(Palette.violet)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 12)
                    .accessibilityIdentifier("domain:\(address)")
                }
            }
        } else {
            let picked = !vm.draft.likeThis || view.quick?.allow == nil
            FactGroup {
                CheckRow(checked: picked && !vm.draft.similar, onChange: { _ in
                    vm.draft.likeThis = false
                    vm.draft.similar = false
                }) { Text("Only the emails I ticked").font(RFont.sans(15.5)).foregroundStyle(Palette.ink) }
                    .padding(16)
                    .accessibilityIdentifier("onlySelected")
                CheckRow(checked: picked && vm.draft.similar, onChange: { _ in
                    vm.draft.likeThis = false
                    vm.draft.similar = true
                }) { Text("Also similar mail").font(RFont.sans(15.5)).foregroundStyle(Palette.ink) }
                    .padding(16)
                    .accessibilityIdentifier("similar")
                if picked && vm.draft.similar {
                    let senders = senderAddresses(view, selected: vm.draft.selected)
                    ForEach(senders, id: \.self) { address in
                        CheckRow(checked: vm.draft.senderAddresses.contains(address), onChange: { on in
                            if on { vm.draft.senderAddresses.insert(address) } else { vm.draft.senderAddresses.remove(address) }
                        }) {
                            Text(untrusted(address)).font(RFont.mono(14)).foregroundStyle(Palette.ink).environment(\.layoutDirection, .leftToRight)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .accessibilityIdentifier("sender:\(address)")
                    }
                    let domains = senders.map(domainOf).reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
                    ForEach(domains, id: \.self) { domain in
                        CheckRow(checked: vm.draft.senderDomains.contains(domain), onChange: { on in
                            if on { vm.draft.senderDomains.insert(domain) } else { vm.draft.senderDomains.remove(domain) }
                        }) {
                            Text("Anyone at @\(domain)").font(RFont.sans(15)).foregroundStyle(Palette.ink)
                        }
                        .padding(.horizontal, 16)
                        .padding(.vertical, 12)
                        .accessibilityIdentifier("senderDomain:\(domain)")
                    }
                }
            }
        }
        TextField("Subject contains (optional)", text: Binding(
            get: { vm.draft.subject },
            set: {
                vm.draft.likeThis = false
                vm.draft.subject = String($0.prefix(200))
            }
        ))
        .font(RFont.sans(16))
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .padding(.horizontal, 16)
        .accessibilityIdentifier("subject")
    }
}

/// What a remembered release of an environment covers: one of the commands the core offers (narrowest first, the exact
/// command to start with), and the variables asked for or every variable of the environment.
private struct SecretsScopeOptions: View {
    @Bindable var vm: ApprovalModel
    var release: EnvReleaseView

    var body: some View {
        let picked = secretsCommand(release, vm.draft)?.id
        VStack(alignment: .leading, spacing: 10) {
            FactHeader("Command")
            FactGroup {
                ForEach(release.commands, id: \.id) { option in
                    OptionRow(selected: picked == option.id, id: "secretsCommand:\(option.id)", tint: option.prefix ? Palette.warning : Palette.violet) {
                        vm.draft.secretsCommand = option.id
                    } label: {
                        Text(untrusted(option.label))
                            .font(RFont.mono(14, .medium))
                            .foregroundStyle(option.prefix ? Palette.warning : Palette.ink)
                            .environment(\.layoutDirection, .leftToRight)
                    }
                }
            }
            .accessibilityContainer("secretsCommands")
            if !release.allVariables {
                FactGroup {
                    CheckRow(checked: vm.draft.secretsAllVariables, onChange: { on in vm.draft.secretsAllVariables = on }) {
                        Text("Every variable").font(RFont.sans(15.5)).foregroundStyle(Palette.ink)
                    }
                    .padding(16)
                    .accessibilityIdentifier("secretsAllVariables")
                }
            }
        }
    }
}

/// A permission the AI asked for can only be made shorter: a slider with a detent per step, up to what was asked.
struct GrantSheet: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        if let asked = view.grant.map({ Int64($0.durationSecs) }) {
            let steps = shortenSteps(asked: asked)
            let current = min(vm.draft.grantSeconds ?? asked, asked)
            let index = max(steps.lastIndex { $0 <= current } ?? 0, 0)
            VStack(alignment: .leading, spacing: 8) {
                Text("Allow for").textRole(.section).foregroundStyle(Palette.ink2)
                Text(ApprovalWords.period(UInt64(steps[index])))
                    .textRole(.cardAsk)
                    .foregroundStyle(Palette.ink)
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
                    .tint(Palette.violet)
                    .accessibilityLabel("Allow for")
                    .accessibilityValue(ApprovalWords.period(UInt64(steps[index])))
                    .accessibilityIdentifier("grantSlider")
                    Text("Asked for \(ApprovalWords.period(UInt64(asked))). Only shorter.").font(RFont.sans(13)).foregroundStyle(Palette.inkMeta)
                }
                Button("Done") { dismiss() }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .padding(.top, 12)
                .accessibilityIdentifier("grantDone")
            }
            .padding(22)
            .presentationDetents([.height(320)])
            .accessibilityContainer("grantSheet")
        }
    }
}
