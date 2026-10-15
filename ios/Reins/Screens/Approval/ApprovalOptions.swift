import SwiftUI

// The approval screen's sheets for how long and how wide (the Android app's ApprovalSheets.kt): "Remember for" behind
// the dock's Custom ▾, with what a remembered approval covers under "What it covers", and a permission's length.

/// "Remember for": a week, 30 days, a custom time, a number of uses, and, set apart in red, until revoked. Under "What it
/// covers", what a remembered approval covers. The choice shows in the dock's last segment.
struct RememberSheet: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback
    @State private var pending: LifetimeKind?
    @State private var more = false

    var body: some View {
        let forever = pending == .untilRevoked && vm.draft.allMail == nil
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    FactGroup {
                        ForEach(ApprovalWords.sheetLifetimes(view)) { kind in
                            LifetimeOption(kind: kind, selected: pending == kind && vm.draft.allMail == nil, vm: vm) { pending = kind }
                        }
                    }
                    if ApprovalWords.offersUntilRevoked(view) {
                        FactGroup(border: Palette.danger.opacity(0.3)) {
                            OptionRow(selected: forever, id: "lifetime:untilRevoked", tint: Palette.danger) {
                                pending = .untilRevoked
                            } label: {
                                HStack(spacing: 12) {
                                    Image(systemName: "infinity").font(.system(size: 17, weight: .semibold)).foregroundStyle(Palette.danger)
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text("Until I revoke").font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.danger)
                                        Text("Never expires").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                                    }
                                }
                            }
                        }
                    }
                    if forever {
                        HStack(alignment: .top, spacing: 10) {
                            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Palette.danger)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Anything like this runs without asking, forever.").font(RFont.sans(13.5, .semibold)).foregroundStyle(Palette.danger)
                                Text("Only for things you'd never want to stop.").font(RFont.sans(13.5)).foregroundStyle(Palette.dynamic(0xDC2626, 0xFFBBBB))
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                        .background(Palette.danger.opacity(0.10), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.danger.opacity(0.28), lineWidth: 1))
                        .padding(.horizontal, 16)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("foreverWarning")
                    }
                    if view.kind != .accounts {
                        Button {
                            withAnimation(.smooth) { more.toggle() }
                            feedback.play(.expand(more))
                        } label: {
                            HStack {
                                Text("What it covers").font(RFont.sans(15, .medium))
                                Spacer()
                                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).rotationEffect(.degrees(more ? 90 : 0))
                            }
                            .foregroundStyle(Palette.accent)
                            .padding(.horizontal, 22)
                            .padding(.vertical, 12)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("moreToggle")
                        .accessibilityValue(more ? "Open" : "Closed")
                        if more { ScopeOptions(vm: vm, view: view).transition(.opacity) }
                    }
                }
                .padding(.vertical, 6)
                .animation(.smooth(duration: 0.25), value: forever)
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { confirm(forever: forever).padding(16).background(Palette.dynamic(0xFFFFFF, 0x151517)) }
            .background(Palette.dynamic(0xFFFFFF, 0x151517).ignoresSafeArea())
            .navigationTitle("Remember for")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button { dismiss() } label: { Image(systemName: "xmark") }
                        .accessibilityLabel("Close")
                        .accessibilityIdentifier("closeOverlay")
                }
            }
        }
        .onAppear { if pending == nil, vm.draft.lifetime != .once { pending = vm.draft.lifetime } }
        .accessibilityContainer("rememberSheet")
    }

    @ViewBuilder private func confirm(forever: Bool) -> some View {
        if forever {
            VStack(spacing: 8) {
                HoldButton(title: "Remember until I revoke", symbol: nil) { apply() }
                    .accessibilityIdentifier("rememberConfirm")
                Text("Hold for 2 seconds").font(RFont.sans(12)).foregroundStyle(Palette.tertiary)
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
            Button { apply() } label: {
                Text(label)
                    .font(RFont.sans(17, .semibold))
                    .foregroundStyle(enabled ? Palette.background : Palette.tertiary)
                    .frame(maxWidth: .infinity, minHeight: 54)
                    .background(enabled ? Palette.text : Palette.controlFill, in: Capsule())
            }
            .buttonStyle(.plain)
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

/// A radio row.
private struct OptionRow<Label: View>: View {
    var selected: Bool
    var id: String
    var tint: Color = Palette.accent
    var onSelect: () -> Void
    @ViewBuilder var label: Label
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 10) {
            label.frame(maxWidth: .infinity, alignment: .leading)
            ZStack {
                Circle().strokeBorder(selected ? tint : Palette.dynamic(0x97979F, 0x4A4A52), lineWidth: 2)
                if selected { Circle().fill(tint).frame(width: 11, height: 11) }
            }
            .frame(width: 22, height: 22)
        }
        .padding(.leading, 18)
        .padding(.trailing, 16)
        .padding(.vertical, 10)
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
                Text(title).font(RFont.sans(15.5)).foregroundStyle(Palette.text).frame(maxWidth: .infinity, alignment: .leading)
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
                        HStack(spacing: 3) {
                            Text(vm.draft.customUnit.rawValue).font(RFont.sans(14.5, .semibold))
                            Image(systemName: "chevron.down").font(.system(size: 10, weight: .bold)).foregroundStyle(Palette.secondary)
                        }
                        .foregroundStyle(Palette.text)
                        .padding(.leading, 11)
                        .padding(.trailing, 8)
                        .padding(.vertical, 7)
                        .background(Palette.background, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
                    }
                    .accessibilityIdentifier("custom:unit")
                case .uses:
                    CountStepper(value: $vm.draft.uses, in: 1...ApprovalRules.maxUses, id: "uses") { onSelect() }
                default:
                    EmptyView()
                }
            }
        }
    }

    private var title: String {
        switch kind {
        case .week: "1 week"
        case .month: "30 days"
        case .custom: "Custom"
        case .uses: "Number of uses"
        default: kind.label
        }
    }
}

/// − n + on a sunken plate.
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
        HStack(spacing: 0) {
            step("minus", "Fewer", value > range.lowerBound) { value -= 1 }
                .accessibilityIdentifier("\(id):minus")
            Text("\(value)").font(RFont.mono(15, .medium)).foregroundStyle(Palette.text).frame(minWidth: 34).monospacedDigit()
                .accessibilityIdentifier("\(id):value")
            step("plus", "More", value < range.upperBound) { value += 1 }
                .accessibilityIdentifier("\(id):plus")
        }
        .background(Palette.background, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
        .accessibilityElement(children: .contain)
    }

    private func step(_ symbol: String, _ label: String, _ enabled: Bool, _ change: @escaping () -> Void) -> some View {
        Button {
            change()
            onChange()
            feedback.play(.detent, step: value)
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(enabled ? Palette.secondary : Palette.tertiary.opacity(0.5))
                .frame(width: 32, height: 34)
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
            if let quick = view.quick, quick.allow != nil {
                FactGroup {
                    OptionRow(selected: !vm.draft.likeThis && vm.draft.allMail == nil, id: "scope:picked") {
                        vm.draft.likeThis = false
                        vm.draft.allMail = nil
                    } label: {
                        Text("Only what's shown").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                    }
                    // Wider than the screen: it says exactly what, and the dock says it is on.
                    OptionRow(selected: vm.draft.likeThis && vm.draft.allMail == nil, id: "scope:like", tint: Palette.danger) {
                        vm.draft.likeThis = true
                        vm.draft.allMail = nil
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Anything like this").font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.warning)
                            Text("\(untrusted(quick.allowWhat)), by \(untrusted(view.connectionLabel))")
                                .font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(2)
                                .accessibilityIdentifier("allowWhat")
                        }
                    }
                }
            }
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
                        Text(untrusted(r.label)).font(RFont.sans(15.5)).foregroundStyle(Palette.text).lineLimit(2)
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
                            Text(address).font(RFont.mono(14)).foregroundStyle(Palette.text).environment(\.layoutDirection, .leftToRight)
                            Text(wide ? "Anyone at @\(domainOf(address))" : "This address only").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                        }
                    }
                    .tint(Palette.accent)
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
                }) { Text("Only the emails I ticked").font(RFont.sans(15.5)).foregroundStyle(Palette.text) }
                    .padding(16)
                    .accessibilityIdentifier("onlySelected")
                CheckRow(checked: picked && vm.draft.similar, onChange: { _ in
                    vm.draft.likeThis = false
                    vm.draft.similar = true
                }) { Text("Also similar mail").font(RFont.sans(15.5)).foregroundStyle(Palette.text) }
                    .padding(16)
                    .accessibilityIdentifier("similar")
                if picked && vm.draft.similar {
                    let senders = senderAddresses(view, selected: vm.draft.selected)
                    ForEach(senders, id: \.self) { address in
                        CheckRow(checked: vm.draft.senderAddresses.contains(address), onChange: { on in
                            if on { vm.draft.senderAddresses.insert(address) } else { vm.draft.senderAddresses.remove(address) }
                        }) {
                            Text(address).font(RFont.mono(14)).foregroundStyle(Palette.text).environment(\.layoutDirection, .leftToRight)
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
                            Text("Anyone at @\(domain)").font(RFont.sans(15)).foregroundStyle(Palette.text)
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
        .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
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
                    OptionRow(selected: picked == option.id, id: "secretsCommand:\(option.id)", tint: option.prefix ? Palette.warning : Palette.accent) {
                        vm.draft.secretsCommand = option.id
                    } label: {
                        Text(untrusted(option.label))
                            .font(RFont.mono(14, .medium))
                            .foregroundStyle(option.prefix ? Palette.warning : Palette.text)
                            .lineLimit(3)
                            .environment(\.layoutDirection, .leftToRight)
                    }
                }
            }
            .accessibilityContainer("secretsCommands")
            if !release.allVariables {
                FactGroup {
                    CheckRow(checked: vm.draft.secretsAllVariables, onChange: { on in vm.draft.secretsAllVariables = on }) {
                        Text("Every variable").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
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
                Text("Allow for").font(RFont.sans(18, .semibold)).foregroundStyle(Palette.text)
                Text(ApprovalWords.period(UInt64(steps[index])))
                    .font(RFont.sans(26, .semibold))
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
                    .accessibilityValue(ApprovalWords.period(UInt64(steps[index])))
                    .accessibilityIdentifier("grantSlider")
                    Text("Asked for \(ApprovalWords.period(UInt64(asked))). Only shorter.").font(RFont.sans(13)).foregroundStyle(Palette.tertiary)
                }
                Button { dismiss() } label: {
                    Text("Done").font(RFont.sans(17, .semibold)).foregroundStyle(Palette.background)
                        .frame(maxWidth: .infinity, minHeight: 54)
                        .background(Palette.text, in: Capsule())
                }
                .buttonStyle(.plain)
                .padding(.top, 12)
                .accessibilityIdentifier("grantDone")
            }
            .padding(22)
            .presentationDetents([.height(320)])
            .accessibilityContainer("grantSheet")
        }
    }
}
