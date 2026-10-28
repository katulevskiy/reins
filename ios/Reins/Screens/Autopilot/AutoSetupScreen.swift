import SwiftUI

/// Auto's setup (01 §9.1), full screen over whatever picked Auto: three steps under one header (back, the step's
/// dots, close), each a standard page in the Next look; Next at the bottom, and on the last step the hold that turns
/// Auto on. Your world: what Auto found, by service, each with its zone in a 4-way chooser (a proposal shows the zone
/// proposed). What Auto does: the matrix, zones by levels, with your names in it; a tap cycles a cell. Turn on: a hold,
/// then the owner's check. Nets come later (P3).
struct AutoSetupScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var setup = AutoSetupModel()

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    Text(AutoSetupText.title(setup.step))
                        .textRole(.title)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                        .padding(.horizontal, 20)
                        .padding(.top, 8)
                        .accessibilityIdentifier("autoSetupTitle")
                    if let error = setup.error {
                        IntegrationBanner(text: error, kind: .error)
                            .padding(.horizontal, 16)
                            .padding(.top, 12)
                            .accessibilityIdentifier("autoSetupError")
                    }
                    step
                }
                .padding(.bottom, 24)
            }
            .scrollBounceBehavior(.basedOnSize)
            footer
        }
        .pageBackground()
        .animation(Motion.of(.smooth(duration: 0.25), reduce: reduceMotion), value: setup.step)
        .task {
            setup.bind(model)
            await setup.load()
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("autoSetup")
        .alert("Let Auto do more", isPresented: Binding(get: { setup.loosen != nil }, set: { if !$0 { setup.cancelLoosen() } })) {
            Button("Cancel", role: .cancel) { setup.cancelLoosen() }
            Button("Continue") { Task { await setup.confirmLoosen() } }
                .accessibilityIdentifier("loosenConfirm")
        } message: {
            Text(verbatim: PolicyPreview.shown(setup.loosen?.line ?? ""))
        }
    }

    // MARK: Parts

    /// Back (or nothing on the first step), the step's dots, close.
    private var header: some View {
        HStack(spacing: 12) {
            if setup.step != .world {
                RoundHeaderButton(glyph: .chevronLeft, label: "Back") { setup.back() }
                    .accessibilityIdentifier("autoSetupBack")
            } else {
                Color.clear.frame(width: 40, height: 40)
            }
            Spacer(minLength: 0)
            HStack(spacing: 6) {
                ForEach(AutoSetupModel.Step.allCases, id: \.self) { s in
                    Capsule()
                        .fill(s == setup.step ? Palette.violet : Palette.rule)
                        .frame(width: s == setup.step ? 20 : 6, height: 6)
                }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Step \(setup.step.rawValue + 1) of \(AutoSetupModel.Step.allCases.count)")
            Spacer(minLength: 0)
            RoundHeaderButton(glyph: .close, label: "Close") {
                feedback.play(.close)
                setup.close()
            }
            .accessibilityIdentifier("autoSetupClose")
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 4)
    }

    @ViewBuilder private var step: some View {
        if setup.setup == nil {
            ProgressView().frame(maxWidth: .infinity).padding(.vertical, 60)
        } else {
            switch setup.step {
            case .world: WorldStep(setup: setup)
            case .matrix: MatrixStep(setup: setup)
            case .turnOn: TurnOnStep(setup: setup)
            }
        }
    }

    @ViewBuilder private var footer: some View {
        Group {
            if setup.step == .turnOn {
                HoldButton(title: "Hold to turn on", tint: Palette.violet, symbol: "bolt.fill", enabled: setup.setup != nil,
                           busy: setup.busy) {
                    Task { await setup.finish() }
                }
                .accessibilityLabel("Turn on Auto")
                .accessibilityHint(HoldButton.hint)
                .accessibilityIdentifier("autoSetupTurnOn")
            } else {
                Button("Next") { setup.next() }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(setup.setup == nil || setup.busy)
                    .accessibilityIdentifier("autoSetupNext")
            }
        }
        .padding(.horizontal, 20)
        .padding(.top, 10)
        .padding(.bottom, 12)
        .background(Palette.paper)
    }
}

// MARK: 1. Your world

private struct WorldStep: View {
    var setup: AutoSetupModel

    var body: some View {
        let sections = setup.sections
        if sections.isEmpty {
            VStack(spacing: 10) {
                Image(systemName: "tray").font(.system(size: 28, weight: .regular)).foregroundStyle(Palette.inkMeta)
                Text("Nothing to sort yet").textRole(.row).foregroundStyle(Palette.ink2)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 48)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("worldEmpty")
        } else {
            ForEach(sections) { section in
                HStack(spacing: 6) {
                    Image(systemName: AutoSetupText.serviceSymbol(section.service)).font(.system(size: 13, weight: .semibold))
                    Text(section.title).font(RFont.sans(13, .semibold)).accessibilityAddTraits(.isHeader)
                    Spacer(minLength: 8)
                }
                .foregroundStyle(Palette.ink2)
                .padding(.horizontal, 24)
                .padding(.top, 22)
                .padding(.bottom, 8)
                RaisedGroup(inset: 14) {
                    ForEach(section.items, id: \.key) { item in
                        WorldRow(item: item, busy: setup.busy) { zone in
                            Task { await setup.setZone(item, zone) }
                        }
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityIdentifier("world:\(section.service)")
            }
        }
    }
}

/// One thing: what it is, its name, how often it came up (quiet), and its zone. A proposal's zone is the one proposed,
/// marked by a small violet dot until it is confirmed (here, or by turning Auto on).
private struct WorldRow: View {
    var item: WorldItem
    var busy: Bool
    var onPick: (ZoneChoice) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: AutoSetupText.kindSymbol(item.kind))
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(Palette.ink2)
                    .frame(width: 20)
                    .accessibilityHidden(true)
                Text(verbatim: AutoSetupText.label(item))
                    .textRole(.row)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if !item.confirmed {
                    Circle().fill(Palette.violet).frame(width: 6, height: 6)
                        .accessibilityLabel("Suggested")
                }
                Spacer(minLength: 8)
                if item.count > 0 {
                    Text("\(item.count)")
                        .font(RFont.mono(13))
                        .foregroundStyle(Palette.inkMeta)
                        .accessibilityLabel("\(item.count) requests")
                }
            }
            Chooser(AutoSetupText.zones.map { zone in
                Chooser.Segment(
                    id: "zone:\(item.key):\(AutoSetupText.zoneId(zone))", title: AutoSetupText.zoneName(zone),
                    selected: item.zone == zone, enabled: !busy
                ) { onPick(zone) }
            })
            // A proposal's own zone can be tapped to confirm it as it is.
            .overlay {
                if !item.confirmed {
                    Capsule().strokeBorder(Palette.violet.opacity(0.35), lineWidth: 1).allowsHitTesting(false)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("worldItem:\(item.key)")
    }
}

// MARK: 2. What Auto does

private struct MatrixStep: View {
    var setup: AutoSetupModel
    @State private var helpOpen = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Spacer()
                Button {
                    helpOpen = true
                    feedback.play(.tap)
                } label: {
                    Image(systemName: "questionmark.circle")
                        .font(.system(size: 18, weight: .medium))
                        .foregroundStyle(Palette.ink2)
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel("About the matrix")
                .accessibilityIdentifier("matrixHelp")
                .popover(isPresented: $helpOpen) {
                    Text(AutoSetupText.matrixHelp)
                        .font(RFont.sans(14.5))
                        .foregroundStyle(Palette.ink)
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 280)
                        .padding(16)
                        .presentationCompactAdaptation(.popover)
                }
            }
            .padding(.horizontal, 12)
            ScrollView(.horizontal) {
                MatrixGrid(setup: setup)
                    .padding(.horizontal, 16)
            }
            .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
            .scrollIndicators(.hidden)
        }
    }
}

/// Zones down, levels across: the zone's name with up to three of your names under it, then a cell per level. "Can't
/// be undone" is locked to Ask.
private struct MatrixGrid: View {
    var setup: AutoSetupModel

    var body: some View {
        let rows = setup.setup?.matrix ?? []
        Grid(alignment: .leading, horizontalSpacing: 6, verticalSpacing: 6) {
            GridRow {
                Color.clear.frame(width: 1, height: 1).gridCellUnsizedAxes([.horizontal, .vertical])
                ForEach(Array(AutoSetupText.levels.enumerated()), id: \.offset) { _, level in
                    Text(level)
                        .font(RFont.sans(11.5, .semibold))
                        .foregroundStyle(Palette.ink2)
                        .multilineTextAlignment(.center)
                        // One word never breaks inside itself ("Recoverab-le"): it shrinks to fit its column instead;
                        // a phrase wraps onto two lines.
                        .lineLimit(level.contains(" ") ? 2 : 1)
                        .minimumScaleFactor(0.7)
                        .frame(width: 66)
                        .accessibilityAddTraits(.isHeader)
                }
            }
            ForEach(rows, id: \.zone) { row in
                GridRow {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(AutoSetupText.zoneName(row.zone)).font(RFont.sans(14.5, .semibold)).foregroundStyle(Palette.ink)
                        let names = AutoSetupText.names(row)
                        if !names.isEmpty {
                            Text(verbatim: names).font(RFont.sans(11.5)).foregroundStyle(Palette.inkMeta).lineLimit(2)
                        }
                    }
                    .frame(width: 84, alignment: .leading)
                    .accessibilityElement(children: .combine)
                    ForEach(Array(row.cells.enumerated()), id: \.offset) { level, cell in
                        MatrixCell(cell: cell, locked: !AutoSetupText.editable(level: level), busy: setup.busy) {
                            Task { await setup.tap(row.zone, level: level) }
                        }
                        .accessibilityLabel("\(AutoSetupText.zoneName(row.zone)), \(AutoSetupText.levels[min(level, 3)])")
                        .accessibilityValue(AutoSetupText.cellName(cell))
                        .accessibilityIdentifier("cell:\(AutoSetupText.zoneId(row.zone)):\(level)")
                    }
                }
            }
        }
        .padding(.vertical, 4)
    }
}

/// A cell: Auto is violet and filled, Ask first a violet wash, Ask sunken; locked shows the lock.
private struct MatrixCell: View {
    var cell: CellChoice
    var locked: Bool
    var busy: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                if locked { Image(systemName: "lock.fill").font(.system(size: 10, weight: .semibold)) }
                Text(AutoSetupText.cellName(cell)).lineLimit(1).minimumScaleFactor(0.8)
            }
            .font(RFont.sans(13, .semibold))
            .foregroundStyle(foreground)
            .frame(width: 66, height: 46)
            .background(background, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
        }
        .buttonStyle(PressDimsStyle())
        .disabled(locked || busy)
        .accessibilityHint(locked ? "Always asks" : "Changes it")
    }

    private var foreground: Color {
        switch cell {
        case .auto: .white
        case .askFirst: Palette.violet
        case .ask: Palette.ink2
        }
    }

    private var background: Color {
        switch cell {
        case .auto: Palette.violetFill
        case .askFirst: Palette.violet.opacity(0.14)
        case .ask: Palette.paperSunken
        }
    }
}

// MARK: 3. Turn on

/// Auto's bolt and how your world is sorted, in one quiet line per zone that has anything.
private struct TurnOnStep: View {
    var setup: AutoSetupModel

    var body: some View {
        let world = setup.setup?.world ?? []
        VStack(spacing: 18) {
            ZStack {
                RoundedRectangle(cornerRadius: 22, style: .continuous).fill(Palette.violetFill)
                Grain().clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
                Image(systemName: "bolt.fill").font(.system(size: 30, weight: .semibold)).foregroundStyle(.white)
            }
            .frame(width: 76, height: 76)
            .accessibilityHidden(true)
            RaisedGroup(inset: 14) {
                ForEach(AutoSetupText.zones, id: \.self) { zone in
                    let n = world.filter { $0.zone == zone }.count
                    if n > 0 {
                        HStack {
                            Text(AutoSetupText.zoneName(zone)).textRole(.row).foregroundStyle(Palette.ink)
                            Spacer()
                            Text("\(n)").font(RFont.mono(14)).foregroundStyle(Palette.inkMeta)
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 48)
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 28)
        .accessibilityIdentifier("autoSetupSummary")
    }
}
