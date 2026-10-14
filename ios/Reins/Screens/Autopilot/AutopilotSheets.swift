import SwiftUI

/// A heading, a card of rows and the explanation under it: Autopilot's pages are cards on the background (as in
/// the Android app and Zeron's settings), not a system list, so the hero and the mode picker sit among them.
struct AutopilotGroup<Content: View>: View {
    var header: String?
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let header { SectionHeader(header).padding(.horizontal, 12) }
            VStack(alignment: .leading, spacing: 0) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 0.5))
                .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
            if let footer { GroupFooter(footer).padding(.horizontal, 12) }
        }
    }
}

/// A hairline between rows of a card, inset to where the text starts.
struct RowDivider: View {
    var inset: CGFloat = 16

    var body: some View {
        Rectangle().fill(Palette.hairline).frame(height: 0.5).padding(.leading, inset)
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

/// Turning a bypass on, for every AI (`who` nil) or one (its name): 15 min, 1 hour, 1 day or Custom (which opens
/// "Bypass for": 4 hours, 1 week, a number and unit of the user's own, or until turned off), held for two seconds to
/// confirm. Used by the Autopilot page, the mode pill's menu and an AI's page; no bypass starts with one tap.
struct BypassSheet: View {
    var who: String?
    var onConfirm: (BypassLength) -> Void

    private enum Pick: Hashable { case preset(UInt32), custom }

    @State private var picked: Pick = .preset(60)
    /// What Custom holds once "Bypass for" chose it.
    @State private var custom: BypassLength?
    @State private var choosing = false
    /// "Bypass for" was held to confirm: started once it has closed.
    @State private var confirmedInside: BypassLength?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    private var length: BypassLength {
        switch picked {
        case let .preset(m): .minutes(m)
        case .custom: custom ?? .minutes(AutopilotText.customMinutes(count: 3, unit: .days))
        }
    }

    private var items: [SegmentItem<Pick>] {
        let presets = AutopilotText.bypassMinutes.map { m in
            SegmentItem(id: Pick.preset(m), title: AutopilotText.lengthLabel(minutes: m), identifier: "bypass:\(m)")
        }
        let off = custom == .untilOff
        return presets + [SegmentItem(
            id: Pick.custom, title: custom?.label ?? "Custom", symbol: off ? "infinity" : nil, chevron: true,
            tint: off ? Palette.danger : nil, identifier: "bypass:custom"
        )]
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(Palette.danger.opacity(0.16))
                    .frame(width: 56, height: 56)
                    .overlay {
                        Image(systemName: "bolt.fill")
                            .font(.system(size: 26, weight: .semibold))
                            .foregroundStyle(Palette.danger)
                    }
                    .accessibilityHidden(true)
                Text(who.map { "Bypass \(untrusted($0))?" } ?? "Bypass every AI?")
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(Palette.text)
                    .multilineTextAlignment(.center)
                    .padding(.top, 14)
                    .accessibilityAddTraits(.isHeader)
                HelpLine(text: "Approved without asking. Sent means sent.", help: AutopilotText.bypassHelp, alignment: .center)
                    .padding(.top, 6)
                Segments(items: items, selected: picked) { pick in
                    if pick == .custom {
                        choosing = true
                    } else {
                        picked = pick
                    }
                }
                .padding(.top, 18)
                HStack(spacing: 10) {
                    Button("Cancel") { dismiss() }
                        .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 50))
                        .accessibilityIdentifier("cancelBypass")
                    HoldButton(title: length.holdTitle, symbol: "bolt.fill", height: 50) { confirm(length) }
                        .layoutPriority(1)
                        .accessibilityIdentifier("confirmBypass")
                }
                .padding(.top, 20)
                Text("Hold for 2 seconds")
                    .font(RFont.sans(12))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.top, 8)
            }
            .padding(.horizontal, 20)
            .padding(.top, 24)
            .padding(.bottom, 16)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .accessibilityIdentifier("bypassDialog")
        .sheet(isPresented: $choosing, onDismiss: {
            guard let chosen = confirmedInside else { return }
            confirmedInside = nil
            confirm(chosen)
        }) {
            BypassForSheet(initial: custom) { chosen in
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
                    Text("Bypass for")
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
                    .accessibilityIdentifier("closeBypassFor")
                }
                .padding(.bottom, 14)

                card(border: Palette.hairline) {
                    ForEach(AutopilotText.bypassMoreMinutes, id: \.self) { m in
                        radioRow(.preset(m), identifier: "bypassFor:\(m)") {
                            Text(AutopilotText.lengthLabel(minutes: m)).font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                        }
                        RowDivider()
                    }
                    // Its stepper and unit menu stay controls of their own for VoiceOver.
                    radioRow(.custom, identifier: "bypassFor:custom", combine: false) {
                        HStack(spacing: 8) {
                            Text("Custom").font(RFont.sans(15.5)).foregroundStyle(Palette.text)
                            Spacer(minLength: 4)
                            stepper
                            unitMenu
                        }
                    }
                }

                card(border: Palette.danger.opacity(0.3)) {
                    radioRow(.untilOff, tint: Palette.danger, identifier: "bypassFor:untilOff") {
                        HStack(spacing: 12) {
                            Image(systemName: "infinity")
                                .font(.system(size: 18, weight: .semibold))
                                .foregroundStyle(Palette.danger)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("Until I turn it off").font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.danger)
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
                            .foregroundStyle(Palette.danger)
                            .padding(.top, 1)
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Everything is approved without asking until you stop it.")
                                .font(RFont.sans(13.5, .semibold))
                                .foregroundStyle(Palette.danger)
                            Text(AutopilotText.bypassStillAsks)
                                .font(RFont.sans(13.5))
                                .foregroundStyle(Palette.text)
                        }
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Palette.danger.opacity(0.10), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.danger.opacity(0.28), lineWidth: 1))
                    .padding(.top, 10)
                    .transition(.opacity)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("untilOffWarning")
                }

                HoldButton(title: length.holdTitle, symbol: "bolt.fill", height: 50) {
                    onConfirm(length)
                    dismiss()
                }
                .accessibilityIdentifier("confirmBypassFor")
                .padding(.top, 16)
                Text("Hold for 2 seconds")
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
        .accessibilityIdentifier("bypassForDialog")
        .onAppear {
            switch initial {
            case .untilOff?:
                pick = .untilOff
            case let .minutes(m)? where AutopilotText.bypassMoreMinutes.contains(m):
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

/// Where a screen in Autopilot opens the next one: from the section's root the detail column (or a push on a
/// phone), from a pushed Autopilot (Activity's pill, Settings) one more screen on top.
extension AppModel {
    func openFromAutopilot(_ route: Route) {
        if section == .autopilot && !path(section).contains(.autopilot) { show(route) } else { push(route) }
    }
}
