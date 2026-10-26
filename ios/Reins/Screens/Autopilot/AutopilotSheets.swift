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

    var tint: Color { self == .bypass ? Palette.danger : Palette.violet }

    /// The lengths offered first, and the one picked at first.
    /// The Mode picker board's lengths: 15 min, 1 hour, 4 hours for a bypass; 4, 8 and 12 hours for Overnight.
    var presets: [UInt32] { self == .bypass ? [15, 60, 240] : AutopilotText.overnightMinutes }
    var initial: UInt32 { self == .bypass ? 60 : 480 }

    /// The longer ones its Custom offers.
    var more: [UInt32] { self == .bypass ? [1_440, 10_080] : AutopilotText.overnightMoreMinutes }

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

/// Turning a bypass on, for every AI (`who` nil) or one (its name): the Bypass sheet (the Mode picker board, 05): what
/// still asks, 15 min, 1 hour, 4 hours or Custom (which opens "Bypass for": 1 day, 1 week, a number and unit of the
/// user's own, or until turned off), then a hold. The phone's owner confirms it too (`AutopilotModel.startBypass`).
struct BypassSheet: View {
    var who: String?
    var onConfirm: (BypassLength) -> Void

    var body: some View {
        TimedModeSheet(kind: .bypass, who: who, help: "Approved without asking. Sent means sent. " + AutopilotText.bypassHelp,
                       onConfirm: onConfirm) {
            HStack(spacing: 8) {
                Text("Still asks").font(RFont.sans(13, .semibold)).foregroundStyle(Palette.ink2)
                Label { Text("New AI") } icon: { GlyphIcon(.link, size: 12) }.labelStyle(StillAsksLabel())
                Label { Text("New phone") } icon: { GlyphIcon(.phone, size: 12) }.labelStyle(StillAsksLabel())
            }
            .accessibilityElement(children: .combine)
        }
    }
}

/// A one-word chip of what a bypass still asks, on paper: its glyph and word.
private struct StillAsksLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon
            configuration.title
        }
        .font(RFont.sans(13, .medium))
        .foregroundStyle(Palette.ink)
    }
}

/// Turning Overnight on for one AI or every AI, as a bypass is: 4, 8 or 12 hours or Custom, what to do with what is
/// too risky, then a hold; the core then asks the owner. Its (?) says it decides like Auto when the model does not rate
/// risk. (Trust starts Overnight on a tap, for 8 hours; this sheet is the mode pill's and an AI's page's.)
struct OvernightSheet: View {
    var who: String?
    var ap: AutopilotModel
    var onConfirm: (BypassLength) -> Void

    var body: some View {
        TimedModeSheet(kind: .overnight, who: who, help: AutopilotText.overnightHelpText(riskRated: ap.settings?.model.risk ?? false),
                       onConfirm: onConfirm) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 6) {
                    GlyphIcon(.warning, size: 15)
                    Text("Too risky").textRole(.section)
                }
                .foregroundStyle(Palette.ink2)
                WhenRiskyPicker(selected: ap.settings?.overnightWhenRisky ?? .wait) { risky in
                    Task { await ap.setOvernightWhenRisky(risky) }
                }
            }
        }
    }
}

/// What Overnight does with what is too risky: Wait for me, or Deny (Overnight's setting, for every AI).
struct WhenRiskyPicker: View {
    var selected: OvernightRisky
    var onPick: (OvernightRisky) -> Void

    var body: some View {
        Chooser([
            Chooser.Segment(id: "whenRisky:wait", title: AutopilotText.whenRiskyWait, selected: selected == .wait) {
                if selected != .wait { onPick(.wait) }
            },
            Chooser.Segment(id: "whenRisky:deny", title: AutopilotText.whenRiskyDeny, selected: selected == .deny) {
                if selected != .deny { onPick(.deny) }
            },
        ])
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("whenRisky")
    }
}

/// A timed mode's sheet (see `BypassSheet`, `OvernightSheet`): its name with a (?) and the close button, who it is
/// for, what `extra` adds, the lengths, and the hold that starts it.
struct TimedModeSheet<Extra: View>: View {
    var kind: TimedMode
    var who: String?
    var help: String
    var onConfirm: (BypassLength) -> Void
    @ViewBuilder var extra: Extra

    private enum Pick: Hashable { case preset(UInt32), custom }

    @State private var picked: Pick?
    /// What Custom holds once "Bypass for" chose it.
    @State private var custom: BypassLength?
    @State private var choosing = false
    @State private var helpOpen = false
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

    private var name: String { kind == .bypass ? ModeKind.bypass.name : ModeKind.overnight.name }

    /// "For Claude": an AI's name, one short clean line; nil for every AI.
    private var forWhom: String? {
        guard let who else { return nil }
        let name = untrustedName(who, limit: 40)
        return name.isEmpty ? nil : name
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .center, spacing: 6) {
                    Text(name)
                        .textRole(.title)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    HelpMark(open: helpOpen) {
                        let next = !helpOpen
                        withAnimation(.smooth(duration: 0.25)) { helpOpen = next }
                        feedback.play(.expand(next))
                    }
                    Spacer(minLength: 8)
                    RoundHeaderButton(glyph: .close, label: "Close") { dismiss() }
                        .accessibilityIdentifier(kind.id("cancelBypass"))
                }
                if helpOpen {
                    Text(help)
                        .textRole(.sub)
                        .foregroundStyle(Palette.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                        .transition(.opacity)
                        .accessibilityIdentifier("helpText")
                }
                if let forWhom {
                    // The AI's name in its own run, never part of Reins' words.
                    HStack(spacing: 4) {
                        Text("For").foregroundStyle(Palette.ink2)
                        Text(verbatim: forWhom).foregroundStyle(Palette.ink).lineLimit(1)
                    }
                    .font(RFont.sans(15, .medium))
                }
                extra
                Chooser(kind.presets.map { m in
                    Chooser.Segment(id: kind.id("bypass:\(m)"), title: AutopilotText.lengthLabel(minutes: m), selected: pick == .preset(m)) {
                        picked = .preset(m)
                    }
                } + [Chooser.Segment(id: kind.id("bypass:custom"), title: custom?.label ?? "Custom", selected: pick == .custom, more: true) {
                    choosing = true
                }])
                HoldButton(title: "Hold to start", tint: kind.tint) { confirm(length) }
                    .accessibilityLabel(kind.holdTitle(length))
                    .accessibilityIdentifier(kind.id("confirmBypass"))
            }
            .padding(.horizontal, 20)
            .padding(.top, 22)
            .padding(.bottom, 16)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.paper.ignoresSafeArea())
        .presentationDetents([.height(who == nil ? (kind == .bypass ? 330 : 380) : 380), .large])
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
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    Spacer()
                    Button {
                        feedback.play(.tap)
                        dismiss()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.system(size: 13, weight: .bold))
                            .foregroundStyle(Palette.ink2)
                            .frame(width: 30, height: 30)
                            .background(Palette.paperSunken, in: Circle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Close")
                    .accessibilityIdentifier(kind.id("closeBypassFor"))
                }
                .padding(.bottom, 14)

                card(border: Palette.rule) {
                    ForEach(kind.more, id: \.self) { m in
                        radioRow(.preset(m), identifier: kind.id("bypassFor:\(m)")) {
                            Text(AutopilotText.lengthLabel(minutes: m)).font(RFont.sans(15.5)).foregroundStyle(Palette.ink)
                        }
                        RowDivider()
                    }
                    // Its stepper and unit menu stay controls of their own for VoiceOver.
                    radioRow(.custom, identifier: kind.id("bypassFor:custom"), combine: false) {
                        HStack(spacing: 8) {
                            Text("Custom").font(RFont.sans(15.5)).foregroundStyle(Palette.ink)
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
                                Text("No end time").font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
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
                                .foregroundStyle(Palette.ink)
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
                    .foregroundStyle(Palette.inkMeta)
                    .frame(maxWidth: .infinity)
                    .padding(.top, 8)
            }
            .padding(20)
            .animation(.smooth(duration: 0.2), value: pick)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.paper.ignoresSafeArea())
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
            .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(border, lineWidth: 1))
            .clipShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
    }

    private func radioRow<Label: View>(
        _ choice: Pick, tint: Color = Palette.violet, identifier: String, combine: Bool = true, @ViewBuilder label: () -> Label
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
                .foregroundStyle(Palette.ink)
                .monospacedDigit()
                .frame(minWidth: 30)
                .accessibilityIdentifier("bypassCount")
            stepButton("plus", identifier: "bypassMore", enabled: count < unit.max) { count += 1 }
        }
        .background(Palette.paper, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
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
                .foregroundStyle(enabled ? Palette.ink2 : Palette.inkMeta.opacity(0.5))
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
                Text(unit.label).font(RFont.sans(14.5, .semibold)).foregroundStyle(Palette.ink)
                Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.ink2)
            }
            .padding(.leading, 11)
            .padding(.trailing, 8)
            .frame(height: 32)
            .background(Palette.paper, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
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
            Text(title).font(RFont.sans(22, .semibold)).foregroundStyle(Palette.ink)
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
                .foregroundStyle(Palette.inkMeta)
                .padding(.top, 18)
                .padding(.bottom, 8)
            HStack(spacing: 6) {
                ForEach(AutopilotText.icons, id: \.self) { e in
                    let on = e == icon
                    Button {
                        feedback.play(.selection)
                        icon = e
                    } label: {
                        GlyphIcon(AutopilotText.profileGlyph(e), size: 18)
                            .foregroundStyle(on ? Palette.onInverse : Palette.ink)
                            .frame(width: 38, height: 38)
                            .background(on ? Palette.inverse : Palette.paperSunken, in: Circle())
                    }
                    .buttonStyle(PressDim())
                    .accessibilityLabel(AutopilotText.profileIconName(e))
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
        .background(Palette.paper.ignoresSafeArea())
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
