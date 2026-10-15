import SwiftUI

/// Signal's motion tokens (signal.tokens.json `motion-time`, Motion.png). Every animation reads its curve here and
/// asks `Motion.of(_:reduce:)`, so Reduce Motion turns slides and folds into 150 ms cross-fades in one place. A hold's
/// duration is never animation: it is measured in elapsed time (`HoldButton`), so nothing here can shorten it.
enum Motion {
    /// Hold to confirm: the fill runs this long, linear. Never shortened.
    static let holdDuration: Double = 1.2
    /// A released hold drains this fast, with no haptic.
    static let holdDrain: Double = 0.18
    /// A field slides down into place when a request opens; its ask follows `askDelay` later.
    static let fieldIn: Double = 0.32
    static let askDelay: Double = 0.06
    /// Colour and hover changes; also every cross-fade under Reduce Motion.
    static let fade: Double = 0.15
    /// An answered field folds into its kind tile at the top of the list.
    static let fold: Double = 0.36
    /// A flicked waiting card leaves.
    static let cardFly: Double = 0.22
    /// The tab bar's pill slides to the chosen tab.
    static let tabPill: Double = 0.24
    /// A sheet rises over its scrim.
    static let sheetRise: Double = 0.28
    /// A pushed page.
    static let drillIn: Double = 0.30
    /// A menu grows from its trigger, scale `menuScale` to 1; it closes in `menuClose`.
    static let menuOpen: Double = 0.18
    static let menuClose: Double = 0.12
    static let menuScale: CGFloat = 0.92
    /// Moving between waiting asks cross-fades the field colour.
    static let kindChange: Double = 0.20
    /// Stats and durations roll digit by digit.
    static let numbers: Double = 0.24
    /// A running bypass breathes its dot from 35 % to 100 % and back.
    static let breathe: Double = 1.8

    /// cubic-bezier(0.2, 0.8, 0.2, 1): fields in, sheets, drill-ins.
    static func easeOut(_ duration: Double) -> Animation { .timingCurve(0.2, 0.8, 0.2, 1, duration: duration) }

    /// The next waiting card rises on a spring (0.32, 0.86).
    static func cardSpring(_ duration: Double = cardFly) -> Animation { .timingCurve(0.32, 0.86, 0.32, 1, duration: duration) }

    /// The tab pill: a spring with a slight overshoot, cubic-bezier(0.3, 1.25, …).
    static var tabSpring: Animation { .spring(duration: tabPill, bounce: 0.18) }

    /// What a moment's animation becomes: itself, or a 150 ms cross-fade under Reduce Motion.
    static func of(_ animation: Animation, reduce: Bool) -> Animation { reduce ? .easeInOut(duration: fade) : animation }

    static func menuIn(reduce: Bool) -> Animation { of(easeOut(menuOpen), reduce: reduce) }
    static func menuOut(reduce: Bool) -> Animation { of(.easeIn(duration: menuClose), reduce: reduce) }
}

extension AnyTransition {
    /// A menu growing from `anchor` (scale 0.92 to 1 with a fade), or only the fade under Reduce Motion.
    static func signalMenu(from anchor: UnitPoint, reduce: Bool) -> AnyTransition {
        reduce ? .opacity : .scale(scale: Motion.menuScale, anchor: anchor).combined(with: .opacity)
    }

    /// A field sliding down from behind the status bar, or a cross-fade under Reduce Motion.
    static func signalField(reduce: Bool) -> AnyTransition {
        reduce ? .opacity : .move(edge: .top)
    }
}

/// Settings > Appearance > Grain on fields, kept in the App Group so widgets and Live Activities follow it. Read it with
/// `@AppStorage(Appearance.grainKey, store: AppGroup.defaults)`; grain also stays off under Reduce Transparency.
enum Appearance {
    static let grainKey = "appearance.grain"
}
