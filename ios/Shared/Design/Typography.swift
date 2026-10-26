import SwiftUI

/// Reins Next's type (design/next/iphone, the Type board): one face, Sora, for everything you read and decide on, and
/// JetBrains Mono for machine values (branches, hashes, addresses, codes, diffs). Sora has no Cyrillic, so each Sora
/// file carries Geologica's at the same weight (ios/scripts/make-fonts.py); Latin stays Sora. Display text (asks,
/// titles, big numbers) is Sora 500 with its target in 600, tracked tighter as it grows. No 700: `.bold` draws
/// semibold. Sizes follow Dynamic Type relative to the text style closest to them.
enum RFont {
    enum Weight {
        case regular, medium, semibold
        /// Next has no 700; kept so older call sites draw semibold.
        case bold

        var sansName: String {
            switch self {
            case .regular: "Sora-Regular"
            case .medium: "Sora-Medium"
            case .semibold, .bold: "Sora-SemiBold"
            }
        }

        /// JetBrains Mono ships 400 and 500; heavier asks draw 500.
        var monoName: String { self == .regular ? "JetBrainsMono-Regular" : "JetBrainsMono-Medium" }
    }

    static func sans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, size: size, relativeTo: style(for: size))
    }

    static func mono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, size: size, relativeTo: style(for: size))
    }

    /// Display text: Sora 500, or 600 for the target of an ask ("Push 8 commits to **octo/app**"). Pair it with
    /// `displayTracking(size)`, or use a `TextRole`, which does.
    static func display(_ size: CGFloat, target: Bool = false) -> Font {
        .custom(displayName(target), size: size, relativeTo: style(for: size))
    }

    /// The same faces at a fixed size (widgets, Live Activities, fixed-height chrome).
    static func fixedSans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, fixedSize: size)
    }

    static func fixedMono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, fixedSize: size)
    }

    static func fixedDisplay(_ size: CGFloat, target: Bool = false) -> Font {
        .custom(displayName(target), fixedSize: size)
    }

    static func displayName(_ target: Bool) -> String { target ? Weight.semibold.sansName : Weight.medium.sansName }

    /// Display tracking in points: -0.03 em from 30 pt up (titles, asks), easing to none at 12 pt (the boards' rule).
    static func displayTracking(_ size: CGFloat) -> CGFloat {
        size * displayTrackingEm(size)
    }

    static func displayTrackingEm(_ size: CGFloat) -> CGFloat {
        -0.03 * min(1, max(0, (size / 0.74 - 16) / 24))
    }

    static func style(for size: CGFloat) -> Font.TextStyle {
        switch size {
        case ..<12: .caption2
        case ..<13: .caption
        case ..<15: .footnote
        case ..<16: .subheadline
        case ..<18: .body
        case ..<21: .title3
        case ..<25: .title2
        case ..<31: .title
        default: .largeTitle
        }
    }
}

/// The type scale (the Type board), as named roles: `Text("Activity").textRole(.title)`.
enum TextRole: CaseIterable {
    /// Display 33: the ask on a field; its target in 600.
    case ask
    /// Display 22: the ask on a waiting card, a mode's name on its card, a sheet's title.
    case cardAsk
    /// Display 30: a tab's title (Activity, Trust, Connections, Vault, Settings).
    case title
    /// Display 24: a pushed page's title (everything under Settings, service details, forms).
    case quietTitle
    /// Display 41: an onboarding page's title.
    case hero
    /// Display 71, tabular: a running mode's clock (47:12).
    case clock
    /// Sans 16.5 semibold: decision buttons.
    case button
    /// Sans 15 semibold: small buttons (on cards, Change, Stop).
    case buttonSmall
    /// Sans 16 medium: row titles.
    case row
    /// Sans 16 regular, 1.35: a row's sentence (who did what), its target in 600; explanations.
    case body
    /// Sans 13 regular: the line under a row title (a mode's effect, a kind).
    case sub
    /// Sans 12.5 medium, +0.01 em: times, counts, small meta.
    case meta
    /// Sans 13 semibold, +0.015 em, never capitals: a section's header over its group.
    case section
    /// Sans 11.5 semibold: outcome tags (Auto, Bypass, Grant).
    case tag
    /// Mono 14: branches, accounts, addresses, SHAs, the timer.
    case value
    /// Mono 13: a value's detail line.
    case detail
    /// Mono 11.5 on 19: code and diffs.
    case code
    /// Mono 24 medium, +0.08 em: codes to read out or compare (4821 9930).
    case codes

    enum Family { case display, sans, mono }

    var family: Family {
        switch self {
        case .ask, .cardAsk, .title, .quietTitle, .hero, .clock: .display
        case .button, .buttonSmall, .row, .body, .sub, .meta, .section, .tag: .sans
        case .value, .detail, .code, .codes: .mono
        }
    }

    var size: CGFloat {
        switch self {
        case .ask: 33
        case .cardAsk: 22
        case .title: 30
        case .quietTitle: 24
        case .hero: 41
        case .clock: 71
        case .button: 16.5
        case .buttonSmall: 15
        case .row, .body: 16
        case .sub: 13
        case .meta: 12.5
        case .section: 13
        case .tag, .code: 11.5
        case .value: 14
        case .detail: 13
        case .codes: 24
        }
    }

    var weight: RFont.Weight {
        switch self {
        case .button, .buttonSmall, .section, .tag: .semibold
        case .row, .meta, .codes: .medium
        default: .regular
        }
    }

    /// Line height in points, where the scale sets one.
    var lineHeight: CGFloat? {
        switch self {
        case .ask, .cardAsk, .title, .quietTitle, .hero: (size * 1.04).rounded()
        case .body: 21.5
        case .sub: 17
        case .code: 19
        default: nil
        }
    }

    /// Letter spacing in em.
    var trackingEm: CGFloat {
        switch self {
        case .ask, .title, .hero, .clock: -0.03
        case .cardAsk, .quietTitle: -0.02
        case .section: 0.015
        case .meta: 0.01
        case .codes: 0.08
        default: 0
        }
    }

    func font(target: Bool = false) -> Font {
        switch family {
        case .display: RFont.display(size, target: target)
        case .sans: RFont.sans(size, target ? .semibold : weight)
        case .mono: RFont.mono(size, weight)
        }
    }
}

extension View {
    /// Sets a role's face, size, line height and tracking.
    func textRole(_ role: TextRole) -> some View {
        font(role.font())
            .tracking(role.trackingEm * role.size)
            .lineHeight(role.lineHeight.map { .multiple(factor: $0 / role.size) })
            .tabular(role == .clock)
    }

    /// Tabular figures where digits tick (timers, clocks), so they don't jitter.
    @ViewBuilder func tabular(_ on: Bool = true) -> some View {
        if on { monospacedDigit() } else { self }
    }
}

/// An ask: one sentence in the display face, its target in 600 ("Push 8 commits to **octo/app**"). It reflows to
/// three lines before it shrinks, and never shrinks below 24 pt (16 pt on a waiting card).
struct AskText: View {
    struct Run: Equatable {
        var text: String
        var target = false
    }

    var runs: [Run]
    var role: TextRole = .ask
    /// Three lines, or nil for an ask that must never be cut (what is approved reads in full).
    var lineLimit: Int? = 3

    init(_ runs: [Run], role: TextRole = .ask, lineLimit: Int? = 3) {
        self.runs = runs
        self.role = role
        self.lineLimit = lineLimit
    }

    /// "Push 8 commits to" + "octo/app".
    init(_ lead: String, target: String, after: String = "", role: TextRole = .ask) {
        self.init([Run(text: lead + " "), Run(text: target, target: true), Run(text: after)].filter { !$0.text.isEmpty }, role: role)
    }

    @ScaledMetric(relativeTo: .largeTitle) private var scale: CGFloat = 1

    var body: some View {
        let size = role.size * scale
        let floor: CGFloat = role == .ask ? 24 : 16
        runs.reduce(Text(verbatim: "")) { sentence, run in
            Text("\(sentence)\(Text(verbatim: run.text).font(RFont.fixedDisplay(size, target: run.target)))")
        }
        .tracking(role.trackingEm * size)
        .lineHeight(role.lineHeight.map { .multiple(factor: $0 / role.size) })
        .lineLimit(lineLimit)
        .minimumScaleFactor(min(1, floor / size))
        .fixedSize(horizontal: false, vertical: true)
    }
}
