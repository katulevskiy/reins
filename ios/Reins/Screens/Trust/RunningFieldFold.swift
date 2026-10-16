import SwiftUI

// A running bypass, Overnight or lockdown folds into a compact bar as Trust scrolls: the label line and the timer stay,
// the sentence under them fades and closes, and the button slides up to the right of the timer. Everything follows
// the scroll offset continuously (no snap); under Reduce Motion the two states cross-fade instead.
//
// The field always takes its open height in the page's top inset, so the list's insets never change while it scrolls
// (a changing inset would feed back into the offset). The colour is drawn at the current height and the list scrolls
// on under the rest: while the field folds, the list's first row stays right under its edge.

/// How far Trust has scrolled and the field's two heights. Only the field reads it, so scrolling redraws the field and
/// not the page.
@Observable @MainActor
final class TrustFold {
    /// How far the list has scrolled from its top, in points; held at `range` once the field is compact.
    private(set) var scrolled: CGFloat = 0
    /// The field's height open and compact, below the status bar (measured from its layout).
    var open: CGFloat = 0
    var compact: CGFloat = 0

    /// The scroll it takes to fold the field: as far as its edge moves.
    var range: CGFloat { max(open - compact, 0) }

    func scroll(to y: CGFloat) {
        let held = Self.held(y, range: range)
        if held != scrolled { scrolled = held }
    }

    /// 0 open … 1 compact.
    func progress(reduceMotion: Bool) -> CGFloat {
        Self.progress(scrolled: scrolled, range: range, reduceMotion: reduceMotion)
    }

    /// The offset worth keeping: none above the top (pulling to refresh keeps it open), and nothing past the fold,
    /// so scrolling on through the page does not redraw the field. Whole points, to skip sub-point updates.
    nonisolated static func held(_ y: CGFloat, range: CGFloat) -> CGFloat {
        let top = max(y, 0)
        return (range > 0 ? min(top, range) : min(top, 1_000)).rounded()
    }

    /// The fold's progress: the share of `range` scrolled; under Reduce Motion one state or the other, switching
    /// halfway (the switch cross-fades).
    nonisolated static func progress(scrolled: CGFloat, range: CGFloat, reduceMotion: Bool) -> CGFloat {
        guard range > 1 else { return 0 }
        let p = min(max(scrolled / range, 0), 1)
        return reduceMotion ? (p >= 0.5 ? 1 : 0) : p
    }

    /// The sentence under the timer fades out over the first part of the fold, before the button crosses it.
    nonisolated static func heroOpacity(_ progress: CGFloat) -> CGFloat { max(0, 1 - progress / 0.45) }

    /// The field's state as its accessibility value: "open", "folding", "compact" (VoiceOver reads it; UI tests check
    /// it on the field's own element, "runningField").
    nonisolated static func stateValue(_ progress: CGFloat) -> String {
        progress >= 1 ? "compact" : progress <= 0 ? "open" : "folding"
    }
}

/// What each child of the field is to its layout.
enum RunningFieldPart: LayoutValueKey {
    static let defaultValue = Part.hero

    enum Part { case surface, label, clock, hero, button, compactEdge }
}

/// The running field's geometry at a progress between open (0) and compact (1). Open: the label line, the timer under
/// it, the sentence, the button across the width. Compact: the label line, then the timer with the button on its
/// right (a lockdown has no timer: its button sits on the label line).
struct RunningFieldGeometry: Equatable {
    static let side: CGFloat = 22
    static let top: CGFloat = 6
    static let openBottom: CGFloat = 22
    static let compactBottom: CGFloat = 12
    static let gap: CGFloat = 12
    static let openButton: CGFloat = 54
    /// Still a full touch target when compact.
    static let compactButton: CGFloat = 44
    /// The timer compact: 58 pt serif drawn at about 30.
    static let clockScale: CGFloat = 30.0 / 58.0

    /// Natural sizes of the parts, measured by the layout.
    struct Sizes: Equatable {
        var width: CGFloat
        var labelHeight: CGFloat
        /// The label line's own width (its words and glyphs).
        var labelNaturalWidth: CGFloat
        /// Nil without a timer (a lockdown, or a mode without an end).
        var clockOpen: CGSize?
        var clockCompact: CGSize?
        var heroHeight: CGFloat
        /// The button's own width (its label and padding).
        var buttonCompactWidth: CGFloat
    }

    var label: CGRect
    /// The timer's frame at full size; it is drawn scaled by `clockScale(progress)` from its top-leading corner.
    var clock: CGRect?
    var hero: CGRect
    var button: CGRect
    /// The coloured surface: the height the field shows now.
    var surface: CGRect
    var openHeight: CGFloat
    var compactHeight: CGFloat

    static func clockScale(_ progress: CGFloat) -> CGFloat { lerp(1, clockScale, progress) }
    static func buttonHeight(_ progress: CGFloat) -> CGFloat { lerp(openButton, compactButton, progress) }

    init(_ s: Sizes, progress p: CGFloat) {
        let side = Self.side, top = Self.top, gap = Self.gap
        let inner = max(s.width - 2 * side, 0)
        let buttonWidth = min(s.buttonCompactWidth, inner)
        // Without a timer the button sits on the label line when both fit, else right-aligned under it (the label is
        // never cut).
        let beside = s.clockCompact == nil && s.labelNaturalWidth + gap + buttonWidth <= inner
        let labelWidth = inner - (beside ? p * (buttonWidth + gap) : 0)

        // Open.
        let clockOpenY = top + s.labelHeight + 18
        let heroOpenY = s.clockOpen.map { clockOpenY + $0.height + 4 } ?? top + s.labelHeight + 22
        let buttonOpenY = heroOpenY + s.heroHeight + 18
        openHeight = buttonOpenY + Self.openButton + Self.openBottom

        // Compact.
        let rowTop: CGFloat, rowHeight: CGFloat, labelCompactY: CGFloat
        var clockCompactY: CGFloat = 0
        if !beside {
            let drawn = (s.clockCompact?.height ?? 0) * Self.clockScale
            rowTop = top + s.labelHeight + 6
            rowHeight = max(drawn, Self.compactButton)
            labelCompactY = top
            clockCompactY = rowTop + (rowHeight - drawn) / 2
        } else {
            rowTop = top
            rowHeight = max(s.labelHeight, Self.compactButton)
            labelCompactY = top + (rowHeight - s.labelHeight) / 2
        }
        let buttonCompactY = rowTop + (rowHeight - Self.compactButton) / 2
        compactHeight = rowTop + rowHeight + Self.compactBottom

        label = CGRect(x: side, y: lerp(top, labelCompactY, p), width: labelWidth, height: s.labelHeight)
        if let open = s.clockOpen, let compact = s.clockCompact {
            let size = CGSize(width: lerp(open.width, compact.width, p), height: lerp(open.height, compact.height, p))
            clock = CGRect(origin: CGPoint(x: side, y: lerp(clockOpenY, clockCompactY, p)), size: size)
        } else {
            clock = nil
        }
        hero = CGRect(x: side, y: lerp(heroOpenY, rowTop, p), width: inner, height: s.heroHeight)
        button = CGRect(
            x: lerp(side, s.width - side - buttonWidth, p),
            y: lerp(buttonOpenY, buttonCompactY, p),
            width: lerp(inner, buttonWidth, p),
            height: Self.buttonHeight(p)
        )
        surface = CGRect(x: 0, y: 0, width: s.width, height: lerp(openHeight, compactHeight, p))
    }
}

private func lerp(_ a: CGFloat, _ b: CGFloat, _ t: CGFloat) -> CGFloat { a + (b - a) * t }

/// Places the running field's parts (tagged with `RunningFieldPart`) by `RunningFieldGeometry`. It always asks for the
/// open height; the surface shows the current one.
struct RunningFieldLayout: Layout {
    var progress: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let g = geometry(width: proposal.width ?? 390, subviews)
        return CGSize(width: proposal.width ?? 390, height: g.openHeight)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let g = geometry(width: bounds.width, subviews)
        for view in subviews {
            let frame: CGRect = switch view[RunningFieldPart.self] {
            case .surface: g.surface
            case .label: g.label
            case .clock: g.clock ?? .zero
            case .hero: g.hero
            case .button: g.button
            case .compactEdge: CGRect(x: 0, y: g.compactHeight, width: 0, height: 0)
            }
            view.place(
                at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY),
                anchor: .topLeading,
                proposal: ProposedViewSize(frame.size)
            )
        }
    }

    private func geometry(width: CGFloat, _ subviews: Subviews) -> RunningFieldGeometry {
        let inner = max(width - 2 * RunningFieldGeometry.side, 0)
        func part(_ p: RunningFieldPart.Part) -> LayoutSubview? { subviews.first { $0[RunningFieldPart.self] == p } }
        let buttonWidth = part(.button)?.sizeThatFits(.unspecified).width ?? 0
        let labelHeight = part(.label)?.sizeThatFits(ProposedViewSize(width: inner, height: nil)).height ?? 0
        let labelNaturalWidth = part(.label)?.sizeThatFits(.unspecified).width ?? 0
        var clockOpen: CGSize?, clockCompact: CGSize?
        if let clock = part(.clock) {
            clockOpen = clock.sizeThatFits(ProposedViewSize(width: inner, height: nil))
            // Compact, the timer shares its row with the button; drawn at `clockScale`, it may take this much.
            let room = (inner - min(buttonWidth, inner) - RunningFieldGeometry.gap) / RunningFieldGeometry.clockScale
            clockCompact = clock.sizeThatFits(ProposedViewSize(width: min(inner, max(room, 0)), height: nil))
        }
        let heroHeight = part(.hero)?.sizeThatFits(ProposedViewSize(width: inner, height: nil)).height ?? 0
        return RunningFieldGeometry(
            .init(width: width, labelHeight: labelHeight, labelNaturalWidth: labelNaturalWidth, clockOpen: clockOpen, clockCompact: clockCompact, heroHeight: heroHeight, buttonCompactWidth: buttonWidth),
            progress: progress
        )
    }
}
