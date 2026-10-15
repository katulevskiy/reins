import SwiftUI

/// A heading, a card of rows and the explanation under it: Autopilot's pages are cards on the background (as in
/// the Android app and Zeron's settings), not a system list, so the hero and the mode picker sit among them. `help`
/// puts the explanation behind a (?) beside the heading instead.
struct AutopilotGroup<Content: View>: View {
    var header: String?
    var help: String?
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let header, let help {
                HelpHeader(title: header, help: help).padding(.horizontal, 12)
            } else if let header {
                SectionHeader(header).padding(.horizontal, 12)
            }
            // Hairlines, never cards: a rule above and below, and between the rows (`RowDivider`).
            VStack(alignment: .leading, spacing: 0) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
                .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
            if let footer { GroupFooter(footer).padding(.horizontal, 12) }
        }
    }
}

/// A hairline between rows of a card, inset to where the text starts.
struct RowDivider: View {
    var inset: CGFloat = 16

    var body: some View {
        Rectangle().fill(Palette.rule).frame(height: 1).padding(.leading, inset)
    }
}

/// A tappable row of a card: a symbol on a tile, a title and subtitle, maybe a chevron.
struct AutopilotRow: View {
    var title: String
    var subtitle: String? = nil
    var symbol: String
    var tint: Color = Palette.accent
    var destructive = false
    var chevron = false
    var identifier: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 14) {
                IconTile(symbol: symbol, tint: destructive ? Palette.danger : tint, size: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(RFont.sans(16, .medium)).foregroundStyle(destructive ? Palette.danger : Palette.text)
                    if let subtitle {
                        Text(subtitle).font(RFont.sans(13)).foregroundStyle(Palette.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 6)
                if chevron {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Palette.tertiary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier(identifier)
    }
}

/// A row's pressed state: the control fill behind it.
struct RowPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.background(configuration.isPressed ? Palette.controlFill : .clear)
    }
}

/// How long a bypass runs: a number of minutes, or until the user turns it off.
enum BypassLength: Hashable {
    case minutes(UInt32)
    case untilOff

    /// What the core takes: the minutes, nil until turned off.
    var minutes: UInt32? {
        switch self {
        case let .minutes(m): m
        case .untilOff: nil
        }
    }

    var label: String { AutopilotText.lengthLabel(minutes: minutes) }

    /// "Hold to bypass", or "Hold to bypass until off".
    var holdTitle: String { self == .untilOff ? "Hold to bypass until off" : "Hold to bypass" }
}

/// The two modes that run for a while and start only after a hold and the owner's check: Bypass and Overnight. Their
/// sheets are one sheet, in each one's symbol, colour, lengths and words.
enum TimedMode: Hashable {
    case bypass, overnight

    var symbol: String { self == .bypass ? "bolt.fill" : AutopilotText.overnightSymbol }

    var tint: Color { self == .bypass ? Palette.danger : Palette.overnight }

    /// The lengths offered first, and the one picked at first.
    var presets: [UInt32] { self == .bypass ? AutopilotText.bypassMinutes : AutopilotText.overnightMinutes }
    var initial: UInt32 { self == .bypass ? 60 : 480 }

    /// The longer ones its Custom offers.
    var more: [UInt32] { self == .bypass ? AutopilotText.bypassMoreMinutes : AutopilotText.overnightMoreMinutes }

    /// "Bypass every AI?", "Overnight for Claude?".
    func title(who: String?) -> String { let t = titleRuns(who: who); return t.lead + t.italic }

    /// The question in the serif: its lead, and what it is for in italic ("Bypass *every AI?*"). An AI's name is one
    /// short clean line.
    func titleRuns(who: String?) -> (lead: String, italic: String) {
        let whom = who.map { untrustedName($0, limit: 40) }.flatMap { $0.isEmpty ? nil : $0 } ?? "every AI"
        switch self {
        case .bypass: return ("Bypass ", "\(whom)?")
        case .overnight: return ("\(AutopilotText.overnightName) for ", "\(whom)?")
        }
    }

    /// Its solid tile's colour: the field it runs as (red for a bypass, violet for Overnight).
    var field: FieldKind { self == .bypass ? .delete : .grant }

    /// The title of its Custom sheet.
    var forTitle: String { self == .bypass ? "Bypass for" : "\(AutopilotText.overnightName) for" }

    func holdTitle(_ length: BypassLength) -> String {
        switch self {
        case .bypass: length.holdTitle
        case .overnight: length == .untilOff ? "Hold to start until off" : "Hold to start"
        }
    }

    /// The warning under "Until I turn it off", in two lines.
    var untilOffWarning: (title: String, detail: String) {
        switch self {
        case .bypass: ("Everything is approved without asking until you stop it.", AutopilotText.bypassStillAsks)
        case .overnight: ("\(AutopilotText.overnightName) runs until you stop it.", "What is too risky is still held.")
        }
    }

    /// Bypass's accessibility identifier, or the same with Overnight in its place ("confirmBypass", "confirmOvernight").
    func id(_ bypassId: String) -> String {
        self == .bypass ? bypassId : bypassId.replacingOccurrences(of: "bypass", with: "overnight").replacingOccurrences(of: "Bypass", with: "Overnight")
    }
}

/// Turning a bypass on, for every AI (`who` nil) or one (its name): 15 min, 1 hour, 1 day or Custom (which opens
/// "Bypass for": 4 hours, 1 week, a number and unit of the user's own, or until turned off), held to
/// confirm. Used by the Autopilot page, the mode pill's menu and an AI's page; no bypass starts with one tap.
struct BypassSheet: View {
    var who: String?
    var onConfirm: (BypassLength) -> Void

    var body: some View {
        TimedModeSheet(kind: .bypass, who: who, line: "Approved without asking. Sent means sent.", help: AutopilotText.bypassHelp,
                       onConfirm: onConfirm) { EmptyView() }
    }
}

/// Turning Overnight on, as a bypass is: 4, 8 or 12 hours or Custom, what to do with what is too risky, held for two
/// seconds, then the owner's check. Its (?) says it decides like Auto when the model does not rate risk.
struct OvernightSheet: View {
    var who: String?
    var ap: AutopilotModel
    var onConfirm: (BypassLength) -> Void

    var body: some View {
        TimedModeSheet(
            kind: .overnight, who: who, line: AutopilotText.overnightSummary,
            help: AutopilotText.overnightHelpText(riskRated: ap.settings?.model.risk ?? false), onConfirm: onConfirm
        ) {
            WhenRiskyPicker(selected: ap.settings?.overnightWhenRisky ?? .wait) { risky in
                Task { await ap.setOvernightWhenRisky(risky) }
            }
            .padding(.top, 18)
        }
    }
}

/// "If something is too risky:" Wait for me, or Deny it (Overnight's setting, for every AI).
struct WhenRiskyPicker: View {
    var selected: OvernightRisky
    var onPick: (OvernightRisky) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(AutopilotText.whenRiskyTitle)
                .textRole(.meta)
                .foregroundStyle(Palette.ink2)
                .padding(.horizontal, 4)
            Chooser([
                Chooser.Segment(id: "whenRisky:wait", title: AutopilotText.whenRiskyWait, selected: selected == .wait) {
                    if selected != .wait { onPick(.wait) }
                },
                Chooser.Segment(id: "whenRisky:deny", title: AutopilotText.whenRiskyDeny, selected: selected == .deny) {
                    if selected != .deny { onPick(.deny) }
                },
            ])
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("whenRisky")
    }
}

/// A timed mode's sheet (see `BypassSheet`, `OvernightSheet`): its symbol and question, a line with its (?), the
/// lengths, what `extra` adds, Cancel and the hold that starts it.
struct TimedModeSheet<Extra: View>: View {
    var kind: TimedMode
    var who: String?
    var line: String
    var help: String
    var onConfirm: (BypassLength) -> Void
    @ViewBuilder var extra: Extra

    private enum Pick: Hashable { case preset(UInt32), custom }

    @State private var picked: Pick?
    /// What Custom holds once "Bypass for" chose it.
    @State private var custom: BypassLength?
    @State private var choosing = false
    /// "Bypass for" was held to confirm: started once it has closed.
    @State private var confirmedInside: BypassLength?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    private var pick: Pick { picked ?? .preset(kind.initial) }

    private var length: BypassLength {
        switch pick {
        case let .preset(m): .minutes(m)
        case .custom: custom ?? .minutes(AutopilotText.customMinutes(count: 3, unit: .days))
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                // The heaviest decision in the app: the mode's solid tile, the question in the serif.
                Image(systemName: kind.symbol)
                    .font(.system(size: 24, weight: .semibold))
                    .foregroundStyle(Palette.onField)
                    .frame(width: 52, height: 52)
                    .fieldBackground(kind.field, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .accessibilityHidden(true)
                let title = kind.titleRuns(who: who)
                AskText([AskText.Run(text: title.lead), AskText.Run(text: title.italic, target: true)], role: .sheetTitle)
                    .foregroundStyle(Palette.ink)
                    .padding(.top, 16)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(title.lead + title.italic)
                    .accessibilityAddTraits(.isHeader)
                HelpLine(text: line, help: help, alignment: .leading)
                    .padding(.top, 8)
                Chooser(kind.presets.map { m in
                    Chooser.Segment(id: kind.id("bypass:\(m)"), title: AutopilotText.lengthLabel(minutes: m), selected: pick == .preset(m)) {
                        picked = .preset(m)
                    }
                } + [Chooser.Segment(id: kind.id("bypass:custom"), title: custom?.label ?? "Custom", selected: pick == .custom, more: true) {
                    choosing = true
                }])
                .padding(.top, 20)
                extra
                HoldButton(title: kind.holdTitle(length), tint: kind.tint, symbol: kind.symbol) { confirm(length) }
                    .accessibilityIdentifier(kind.id("confirmBypass"))
                    .padding(.top, 22)
                Button("Cancel") { dismiss() }
                    .font(RFont.sans(15, .medium))
                    .foregroundStyle(Palette.ink2)
                    .frame(maxWidth: .infinity, minHeight: 44)
                    .accessibilityIdentifier(kind.id("cancelBypass"))
                    .padding(.top, 4)
            }
            .padding(.horizontal, 24)
            .padding(.top, 28)
            .padding(.bottom, 16)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .accessibilityIdentifier(kind.id("bypassDialog"))
        .sheet(isPresented: $choosing, onDismiss: {
            guard let chosen = confirmedInside else { return }
            confirmedInside = nil
            confirm(chosen)
        }) {
            BypassForSheet(kind: kind, initial: custom) { chosen in
                custom = chosen
                picked = .custom
            } onConfirm: { chosen in
                confirmedInside = chosen
            }
            .environment(\.feedback, feedback)
        }
    }

    private func confirm(_ chosen: BypassLength) {
        dismiss()
        onConfirm(chosen)
    }
}

/// "Bypass for": the longer lengths, one of the user's own (a number and a unit), and, apart in red, until turned
/// off, which warns. Every change shows on the first sheet's Custom segment; held here, the bypass starts at once.
struct BypassForSheet: View {
    var kind: TimedMode = .bypass
    var initial: BypassLength?
    var onPick: (BypassLength) -> Void
    var onConfirm: (BypassLength) -> Void

    private enum Pick: Hashable { case preset(UInt32), custom, untilOff }

    @State private var pick: Pick = .custom
    @State private var count = 3
    @State private var unit: AutopilotText.BypassUnit = .days
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    private var length: BypassLength {
        switch pick {
        case let .preset(m): .minutes(m)
        case .custom: .minutes(AutopilotText.customMinutes(count: count, unit: unit))
        case .untilOff: .untilOff
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text(kind.forTitle)
                        .font(RFont.sans(18, .semibold))
                        .foregroundStyle(Palette.text)
                        .accessibilityAddTraits(.isHeader)
                    Spacer()
                    Button {
                        feedback.play(.tap)
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundStyle(Palette.secondary)
                            .frame(width: 30, height: 30)
                            .background(Palette.controlFill, in: Circle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Close")
                    .accessibilityIdentifier(kind.id("closeBypassFor"))
                }
                .padding(.bottom, 14)

                card(border: Palette.hairline) {
                    ForEach(kind.more, id: \.self) { m in
                        radioRow(.preset(m), identifier: kind.id("bypassFor:\(m)")) {
                            Text(AutopilotText.lengthLabel(minutes: m)).font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                        }
                        RowDivider()
                    }
                    // Its stepper and unit menu stay controls of their own for VoiceOver.
                    radioRow(.custom, identifier: kind.id("bypassFor:custom"), combine: false) {
                        HStack(spacing: 8) {
                            Text("Custom").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                            Spacer(minLength: 4)
                            stepper
                            unitMenu
                        }
                    }
                }

                card(border: kind.tint.opacity(0.3)) {
                    radioRow(.untilOff, tint: kind.tint, identifier: kind.id("bypassFor:untilOff")) {
                        HStack(spacing: 12) {
                            Image(systemName: "infinity")
                                .font(.system(size: 18, weight: .semibold))
                                .foregroundStyle(kind.tint)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Until I turn it off").font(RFont.sans(15.5, .semibold)).foregroundStyle(kind.tint)
                                Text("No end time").font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                            }
                        }
                    }
                }
                .padding(.top, 12)

                if pick == .untilOff {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.system(size: 15, weight: .semibold))
                            .foregroundStyle(kind.tint)
                            .padding(.top, 1)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(kind.untilOffWarning.title)
                                .font(RFont.sans(13.5, .semibold))
                                .foregroundStyle(kind.tint)
                            Text(kind.untilOffWarning.detail)
                                .font(RFont.sans(13.5))
                                .foregroundStyle(Palette.text)
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(kind.tint.opacity(0.10), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(kind.tint.opacity(0.28), lineWidth: 1))
                    .padding(.top, 10)
                    .transition(.opacity)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("untilOffWarning")
                }

                HoldButton(title: kind.holdTitle(length), symbol: kind.symbol, height: 50) {
                    onConfirm(length)
                    dismiss()
                }
                .accessibilityIdentifier(kind.id("confirmBypassFor"))
                .padding(.top, 16)
                Text(HoldButton.hint)
                    .font(RFont.sans(12))
                    .foregroundStyle(Palette.tertiary)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 8)
            }
            .padding(20)
            .animation(.smooth(duration: 0.2), value: pick)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .accessibilityIdentifier(kind.id("bypassForDialog"))
        .onAppear {
            switch initial {
            case .untilOff?:
                pick = .untilOff
            case let .minutes(m)? where kind.more.contains(m):
                pick = .preset(m)
            case let .minutes(m)?:
                let stepper = AutopilotText.countAndUnit(minutes: m)
                count = stepper.count
                unit = stepper.unit
                pick = .custom
            case nil:
                pick = .custom
            }
            onPick(length)
        }
        .onChange(of: length) { onPick(length) }
        .onChange(of: unit) {
            count = min(count, unit.max)
            pick = .custom
        }
    }

    private func card<Content: View>(border: Color, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 0) { content() }
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(border, lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
    }

    private func radioRow<Label: View>(
        _ choice: Pick, tint: Color = Palette.accent, identifier: String, combine: Bool = true, @ViewBuilder label: () -> Label
    ) -> some View {
        let on = pick == choice
        return HStack(spacing: 10) {
            label()
            Spacer(minLength: 4)
            RadioDot(selected: on, tint: tint)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 13)
        .contentShape(Rectangle())
        .onTapGesture {
            guard pick != choice else { return }
            feedback.play(.selection)
            pick = choice
        }
        .accessibilityElement(children: combine ? .combine : .contain)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
        .accessibilityAction { pick = choice }
        .accessibilityIdentifier(identifier)
    }

    /// − 3 +: the custom number, 1 to 99 (weeks: 52).
    private var stepper: some View {
        HStack(spacing: 0) {
            stepButton("minus", identifier: "bypassLess", enabled: count > 1) { count -= 1 }
            Text("\(count)")
                .font(RFont.mono(15, .medium))
                .foregroundStyle(Palette.text)
                .monospacedDigit()
                .frame(minWidth: 30)
                .accessibilityIdentifier("bypassCount")
            stepButton("plus", identifier: "bypassMore", enabled: count < unit.max) { count += 1 }
        }
        .background(Palette.background, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
        .accessibilityElement(children: .ignore)
        .accessibilityIdentifier("bypassStepper")
        .accessibilityLabel("Custom length")
        .accessibilityValue("\(count) \(unit.label)")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: count = min(count + 1, unit.max)
            case .decrement: count = max(count - 1, 1)
            @unknown default: break
            }
            pick = .custom
        }
    }

    private func stepButton(_ symbol: String, identifier: String, enabled: Bool, action: @escaping () -> Void) -> some View {
        Button {
            feedback.play(.selection)
            action()
            pick = .custom
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(enabled ? Palette.secondary : Palette.tertiary.opacity(0.5))
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityIdentifier(identifier)
    }

    /// days ▾: minutes, hours, days or weeks.
    private var unitMenu: some View {
        Menu {
            Picker("Unit", selection: $unit) {
                ForEach(AutopilotText.BypassUnit.allCases) { u in
                    Text(u.label).tag(u)
                }
            }
        } label: {
            HStack(spacing: 3) {
                Text(unit.label).font(RFont.sans(14.5, .semibold)).foregroundStyle(Palette.text)
                Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.secondary)
            }
            .padding(.leading, 11)
            .padding(.trailing, 8)
            .frame(height: 32)
            .background(Palette.background, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
        }
        .accessibilityLabel("Unit")
        .accessibilityValue(unit.label)
        .accessibilityIdentifier("bypassUnit")
    }
}

/// Naming a profile and picking its icon (new and rename).
struct ProfileEditorSheet: View {
    var title: String
    var initialName: String
    var initialIcon: String?
    var confirmLabel: String
    var onConfirm: (String, String?) -> Void
    @State private var name = ""
    @State private var icon: String?
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title).font(RFont.sans(22, .semibold)).foregroundStyle(Palette.text)
            InputWell {
                TextField("Name, e.g. Side project", text: $name)
                    .font(RFont.sans(16))
                    .focused($focused)
                    .submitLabel(.done)
                    .onSubmit(save)
                    .onChange(of: name) { if name.count > 40 { name = String(name.prefix(40)) } }
                    .accessibilityIdentifier("profileName")
            }
            .padding(.top, 14)
            Text("Icon")
                .font(RFont.sans(13, .medium))
                .foregroundStyle(Palette.tertiary)
                .padding(.top, 18)
                .padding(.bottom, 8)
            HStack(spacing: 6) {
                ForEach(AutopilotText.icons, id: \.self) { e in
                    let on = e == icon
                    Button {
                        feedback.play(.selection)
                        icon = e
                    } label: {
                        Text(e)
                            .font(.system(size: 19))
                            .frame(width: 38, height: 38)
                            .background(on ? Palette.accentSoft : .clear, in: Circle())
                            .overlay(Circle().strokeBorder(on ? Palette.accent : .clear, lineWidth: 1.5))
                    }
                    .buttonStyle(PressDim())
                    .accessibilityAddTraits(on ? .isSelected : [])
                    .accessibilityIdentifier("icon:\(e)")
                }
            }
            Spacer(minLength: 20)
            HStack(spacing: 10) {
                ActionButton(title: "Cancel", kind: .secondary) { dismiss() }
                ActionButton(title: confirmLabel, kind: .accent, enabled: !trimmed.isEmpty, action: save)
                    .accessibilityIdentifier("saveProfile")
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.height(340), .medium])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .onAppear {
            name = initialName
            icon = initialIcon
            focused = true
        }
        .accessibilityIdentifier("profileDialog")
    }

    private var trimmed: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func save() {
        guard !trimmed.isEmpty else { return }
        dismiss()
        onConfirm(trimmed, icon)
    }
}

/// Where Autopilot's part of Trust opens the next one: from Trust's root the detail column (or a push on a phone),
/// from anywhere deeper one more screen on top.
extension AppModel {
    func openFromAutopilot(_ route: Route) {
        if section == .trust && path(section).isEmpty { show(route) } else { push(route) }
    }
}
