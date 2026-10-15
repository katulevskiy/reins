import SwiftUI

// Signal's menu (Components.png, Menus): a raised card of rows that grows from the control that opened it, scale 0.92
// to 1 over 180 ms, and folds back into it in 120 ms. It replaces system pickers and context menus: the same paper and
// rows on every platform. Press the trigger and slide down the rows to pick in one gesture; each row crossed ticks.
//
// The mechanics are AutopilotQuickMenu's, made general: the trigger publishes its frame, and a host high on the page
// (`signalMenuHost()`) draws the open menu over everything with a catcher that closes it.

/// One row of a menu.
struct SignalMenuItem: Identifiable, Equatable {
    enum Tone: Equatable {
        /// The glyph on a sunken tile, in ink.
        case plain
        /// The glyph white on a solid tile (the Auto row's violet).
        case solid(Color)
        /// The glyph in a kind's ink on its tile (Lockdown's amber).
        case kind(FieldKind)
        /// Red title and glyph on the danger wash (Bypass…).
        case danger
    }

    /// The accessibility identifier, and what `onPick` receives.
    var id: String
    var title: String
    var subtitle: String? = nil
    var glyph: Glyph? = nil
    var tone: Tone = .plain
    /// A hairline above this row.
    var separated = false
    /// The row opens something further (a chevron).
    var more = false
    var enabled = true
}

/// The open menu, kept by the host.
@Observable
final class SignalMenuController {
    struct Open {
        var trigger: String
        var items: [SignalMenuItem]
        var selected: String?
        var onPick: (String) -> Void
    }

    var open: Open?
    /// The row under a sliding finger, highlighted until it lifts.
    var hovered: String?
    /// Where each trigger and each open row is, in the host's space.
    @ObservationIgnored fileprivate var rows: [String: CGRect] = [:]

    func toggle(_ menu: Open) {
        open = open?.trigger == menu.trigger ? nil : menu
        hovered = nil
    }

    func close() {
        open = nil
        hovered = nil
    }

    func row(at point: CGPoint) -> SignalMenuItem? {
        guard let open else { return nil }
        return open.items.first { $0.enabled && rows[$0.id]?.contains(point) == true }
    }
}

private struct SignalMenuAnchors: PreferenceKey {
    static var defaultValue: [String: Anchor<CGRect>] { [:] }

    static func reduce(value: inout [String: Anchor<CGRect>], nextValue: () -> [String: Anchor<CGRect>]) {
        value.merge(nextValue()) { $1 }
    }
}

private struct SignalMenuHost: ViewModifier {
    @State private var controller = SignalMenuController()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.feedback) private var feedback

    func body(content: Content) -> some View {
        content
            .environment(controller)
            .overlayPreferenceValue(SignalMenuAnchors.self) { anchors in
                GeometryReader { geo in
                    ZStack(alignment: .topLeading) {
                        if controller.open != nil {
                            Color.clear
                                .contentShape(Rectangle())
                                .ignoresSafeArea()
                                .onTapGesture {
                                    feedback.play(.close)
                                    close()
                                }
                                .accessibilityAddTraits(.isButton)
                                .accessibilityLabel("Close")
                                .accessibilityIdentifier("menuScrim")
                        }
                        if let open = controller.open, let anchor = anchors[open.trigger] {
                            let trigger = geo[anchor]
                            let width = min(290, geo.size.width - 24)
                            let trailing = trigger.midX > geo.size.width / 2
                            let x = trailing
                                ? min(max(trigger.maxX - width, 12), geo.size.width - width - 12)
                                : min(max(trigger.minX, 12), geo.size.width - width - 12)
                            // Below the trigger, unless it sits low: then it grows upward.
                            let estimate = CGFloat(open.items.count) * 56 + 12
                            let up = trigger.maxY + 8 + estimate > geo.size.height && trigger.minY - 8 - estimate > 0
                            SignalMenuCard(controller: controller, open: open, onPick: pick)
                                .frame(width: width)
                                .fixedSize(horizontal: false, vertical: true)
                                .alignmentGuide(.top) { d in up ? d.height - trigger.minY + 8 : -(trigger.maxY + 8) }
                                .alignmentGuide(.leading) { _ in -x }
                                .transition(.signalMenu(from: UnitPoint(x: trailing ? 1 : 0, y: up ? 1 : 0), reduce: reduceMotion))
                                .accessibilityAction(.escape) { close() }
                        }
                    }
                    .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
                }
                .allowsHitTesting(controller.open != nil)
            }
            .coordinateSpace(.named(SignalMenuCard.space))
            .animation(controller.open == nil ? Motion.menuOut(reduce: reduceMotion) : Motion.menuIn(reduce: reduceMotion), value: controller.open?.trigger)
    }

    private func pick(_ item: SignalMenuItem) {
        guard let open = controller.open else { return }
        feedback.play(.selection)
        close()
        open.onPick(item.id)
    }

    private func close() { controller.close() }
}

/// The open menu in the host: the panel, reporting where its rows are so a sliding finger can find them.
private struct SignalMenuCard: View {
    static let space = "signalMenuHost"

    var controller: SignalMenuController
    var open: SignalMenuController.Open
    var onPick: (SignalMenuItem) -> Void

    var body: some View {
        SignalMenuPanel(items: open.items, selected: open.selected, hovered: controller.hovered, onPick: onPick) { id, frame in
            controller.rows[id] = frame
        }
    }
}

/// The menu itself: raised paper, a hairline ring and a soft shadow, 6 pt inside; rows 52 pt with a tile, a title and a
/// line under it; a hairline between groups.
struct SignalMenuPanel: View {
    var items: [SignalMenuItem]
    var selected: String?
    var hovered: String? = nil
    var onPick: (SignalMenuItem) -> Void = { _ in }
    /// Where each row is, in the host's space.
    var onRowFrame: ((String, CGRect) -> Void)? = nil

    var body: some View {
        VStack(spacing: 0) {
            ForEach(items) { item in
                if item.separated {
                    Rectangle().fill(Palette.rule).frame(height: 1).padding(.horizontal, 10).padding(.vertical, 4)
                }
                Button { onPick(item) } label: {
                    SignalMenuRow(item: item, selected: item.id == selected, hovered: item.id == hovered)
                }
                .buttonStyle(PressDimsStyle())
                .disabled(!item.enabled)
                .accessibilityAddTraits(item.id == selected ? .isSelected : [])
                .accessibilityIdentifier(item.id)
                .background {
                    if let onRowFrame {
                        GeometryReader { row in
                            let frame = row.frame(in: .named(SignalMenuCard.space))
                            Color.clear
                                .onAppear { onRowFrame(item.id, frame) }
                                .onChange(of: frame) { _, frame in onRowFrame(item.id, frame) }
                        }
                    }
                }
            }
        }
        .padding(6)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
        // 0 24 48 -16 rgba(10, 10, 20, .35), without the negative spread SwiftUI can't draw.
        .shadow(color: Color(red: 10 / 255, green: 10 / 255, blue: 20 / 255).opacity(0.22), radius: 18, y: 18)
        .accessibilityElement(children: .contain)
    }
}

/// A menu row: tile, title and its line, and a check (the current choice) or a chevron (opens more).
struct SignalMenuRow: View {
    var item: SignalMenuItem
    var selected = false
    var hovered = false

    var body: some View {
        HStack(spacing: 12) {
            if let glyph = item.glyph {
                GlyphIcon(glyph, size: 18)
                    .foregroundStyle(tileInk)
                    .frame(width: 32, height: 32)
                    .background(tileFill, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(item.title)
                    .font(RFont.sans(15, .medium))
                    .foregroundStyle(item.tone == .danger ? Palette.danger : Palette.ink)
                if let subtitle = item.subtitle {
                    Text(subtitle).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if selected {
                GlyphIcon(.check, size: 18).foregroundStyle(Palette.violet)
            } else if item.more {
                GlyphIcon(.chevronRight, size: 16).foregroundStyle(Palette.inkFaint)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .frame(minHeight: 52)
        .background(selected || hovered ? Palette.paperSunken : .clear, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .contentShape(Rectangle())
        .opacity(item.enabled ? 1 : 0.4)
    }

    private var tileInk: Color {
        switch item.tone {
        case .plain: Palette.ink
        case .solid: .white
        case .kind(let kind): kind.ink
        case .danger: Palette.danger
        }
    }

    private var tileFill: Color {
        switch item.tone {
        case .plain: Palette.paperSunken
        case .solid(let color): color
        case .kind(let kind): kind.tile
        case .danger: Palette.dangerWash
        }
    }
}

extension View {
    /// Draws the Signal menus opened inside this view, over it. Put it on a page or sheet root.
    func signalMenuHost() -> some View { modifier(SignalMenuHost()) }

    /// Makes this view the trigger of a Signal menu: a tap opens or closes it; pressing and sliding onto a row picks it
    /// when the finger lifts, with a select tick for each row crossed. Needs a `signalMenuHost()` above it.
    func signalMenu(
        id: String, items: [SignalMenuItem], selected: String? = nil, onPick: @escaping (String) -> Void
    ) -> some View {
        modifier(SignalMenuTrigger(id: id, items: items, selected: selected, onPick: onPick))
    }
}

private struct SignalMenuTrigger: ViewModifier {
    var id: String
    var items: [SignalMenuItem]
    var selected: String?
    var onPick: (String) -> Void

    @Environment(SignalMenuController.self) private var controller: SignalMenuController?
    @Environment(\.feedback) private var feedback
    @State private var sliding = false
    /// Whether this press found the menu already open (a tap then closes it); nil between presses.
    @State private var wasOpen: Bool?

    func body(content: Content) -> some View {
        content
            .anchorPreference(key: SignalMenuAnchors.self, value: .bounds) { [id: $0] }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .named(SignalMenuCard.space))
                    .onChanged { drag in
                        guard let controller else { return }
                        if wasOpen == nil {
                            wasOpen = controller.open?.trigger == id
                            sliding = false
                            if wasOpen == false {
                                controller.toggle(open)
                                feedback.play(.open)
                            }
                            return
                        }
                        // A slide onto the rows: highlight the row under the finger, a tick for each one crossed.
                        guard drag.translation.height.magnitude > 8 || sliding else { return }
                        sliding = true
                        let row = controller.row(at: drag.location)?.id
                        if row != controller.hovered {
                            controller.hovered = row
                            if row != nil { feedback.play(.selection) }
                        }
                    }
                    .onEnded { drag in
                        guard let controller else { return }
                        defer {
                            sliding = false
                            wasOpen = nil
                        }
                        if sliding, let row = controller.row(at: drag.location) {
                            feedback.play(.selection)
                            controller.close()
                            onPick(row.id)
                        } else if !sliding, wasOpen == true {
                            feedback.play(.close)
                            controller.close()
                        } else {
                            controller.hovered = nil
                        }
                    }
            )
            .accessibilityAddTraits(.isButton)
            .accessibilityAction {
                controller?.toggle(open)
            }
    }

    private var open: SignalMenuController.Open {
        SignalMenuController.Open(trigger: id, items: items, selected: selected, onPick: onPick)
    }
}
