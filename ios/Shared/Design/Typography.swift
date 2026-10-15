import SwiftUI

/// Signal's three voices (design/signal/screens/png/Type.png): Instrument Serif for what you decide on (asks, titles,
/// big numbers, the mood line), a sans for everything you read (Instrument Sans, or Geist from Appearance), and
/// Geist Mono for labels in capitals and machine values. No weight 700: `.bold` draws semibold, and the serif is never
/// bold. Sizes follow Dynamic Type relative to the text style closest to them.
enum RFont {
    enum Weight {
        case regular, medium, semibold
        /// Signal has no 700; kept so older call sites draw semibold.
        case bold

        var sansName: String { RFont.sansFamily.name(self) }

        var monoName: String {
            switch self {
            case .regular: "GeistMono-Regular"
            case .medium: "GeistMono-Medium"
            case .semibold, .bold: "GeistMono-SemiBold"
            }
        }
    }

    /// The sans you read in (Settings > Appearance): Instrument Sans, or Geist.
    enum SansFamily: String, CaseIterable {
        case instrument, geist

        var title: String { self == .instrument ? "Instrument" : "Geist" }

        func name(_ weight: Weight) -> String {
            switch (self, weight) {
            case (.instrument, .regular): "InstrumentSans-Regular"
            case (.instrument, .medium): "InstrumentSans-Medium"
            case (.instrument, .semibold), (.instrument, .bold): "InstrumentSans-SemiBold"
            case (.geist, .regular): "Geist-Regular"
            case (.geist, .medium): "Geist-Medium"
            case (.geist, .semibold), (.geist, .bold): "Geist-SemiBold"
            }
        }
    }

    static let sansFamilyKey = "appearance.sans"

    /// Read from the App Group, so widgets speak in the same sans as the app. Views pick a change up when they next
    /// draw; Appearance redraws the app when it changes.
    static var sansFamily: SansFamily {
        get { AppGroup.defaults.string(forKey: sansFamilyKey).flatMap(SansFamily.init(rawValue:)) ?? .instrument }
        set { AppGroup.defaults.set(newValue.rawValue, forKey: sansFamilyKey) }
    }

    static func sans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, size: size, relativeTo: style(for: size))
    }

    static func mono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, size: size, relativeTo: style(for: size))
    }

    /// Instrument Serif, 400 only; `italic` for the target of an ask and the one italic word of a sheet title.
    static func serif(_ size: CGFloat, italic: Bool = false) -> Font {
        .custom(serifName(italic), size: size, relativeTo: style(for: size))
    }

    /// The same face at a fixed size (widgets, Live Activities, fixed-height chrome).
    static func fixedSans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, fixedSize: size)
    }

    static func fixedMono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, fixedSize: size)
    }

    static func fixedSerif(_ size: CGFloat, italic: Bool = false) -> Font {
        .custom(serifName(italic), fixedSize: size)
    }

    static func serifName(_ italic: Bool) -> String { italic ? "InstrumentSerif-Italic" : "InstrumentSerif-Regular" }

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

/// The type scale (signal.tokens.json `type`), as named roles: `Text("Activity").textRole(.title)`.
enum TextRole: CaseIterable {
    /// Serif 64/62: asks on desktop and iPad.
    case askXL
    /// Serif 45/46: the ask on a field; the target in italic.
    case ask
    /// Serif 25/26: the ask on a waiting card.
    case askSmall
    /// Serif 44/44: screen titles.
    case title
    /// Serif 40: numbers that are the point (142 requests, 94 %, 5h left).
    case stat
    /// Serif 32/34: sheet titles, with one italic word.
    case sheetTitle
    /// Serif italic 24/30 in ink-2: one quiet line under a title.
    case mood
    /// Sans 16.5 semibold: decision buttons.
    case button
    /// Sans 14.5 semibold: small buttons.
    case buttonSmall
    /// Sans 15.5 medium: row titles.
    case row
    /// Sans 15/22: explanations in sheets and pages.
    case body
    /// Sans 13/18: row subtitles, hints.
    case meta
    /// Sans 11.5 semibold: tags (Auto, Ready, Code).
    case tag
    /// Mono 11.5 medium, capitals tracked 0.16 em: the label line on a field, section labels.
    case label
    /// Mono 13.5: branches, accounts, addresses, SHAs, the timer.
    case value

    enum Family { case serif, sans, mono }

    var family: Family {
        switch self {
        case .askXL, .ask, .askSmall, .title, .stat, .sheetTitle, .mood: .serif
        case .button, .buttonSmall, .row, .body, .meta, .tag: .sans
        case .label, .value: .mono
        }
    }

    var size: CGFloat {
        switch self {
        case .askXL: 64
        case .ask: 45
        case .askSmall: 25
        case .title: 44
        case .stat: 40
        case .sheetTitle: 32
        case .mood: 24
        case .button: 16.5
        case .buttonSmall: 14.5
        case .row: 15.5
        case .body: 15
        case .meta: 13
        case .tag, .label: 11.5
        case .value: 13.5
        }
    }

    var weight: RFont.Weight {
        switch self {
        case .button, .buttonSmall, .tag: .semibold
        case .row, .label: .medium
        default: .regular
        }
    }

    var italic: Bool { self == .mood }

    /// Line height in points, where the scale sets one.
    var lineHeight: CGFloat? {
        switch self {
        case .askXL: 62
        case .ask: 46
        case .askSmall: 26
        case .title: 44
        case .stat: 40
        case .sheetTitle: 34
        case .mood: 30
        case .body: 22
        case .meta: 18
        case .label: 14
        default: nil
        }
    }

    /// Letter spacing in em.
    var trackingEm: CGFloat {
        switch self {
        case .askXL: -0.015
        case .ask: -0.012
        case .askSmall, .title, .stat, .sheetTitle: -0.01
        case .label: 0.16
        default: 0
        }
    }

    var uppercased: Bool { self == .label }

    func font(italic: Bool? = nil) -> Font {
        switch family {
        case .serif: RFont.serif(size, italic: italic ?? self.italic)
        case .sans: RFont.sans(size, weight)
        case .mono: RFont.mono(size, weight)
        }
    }
}

extension View {
    /// Sets a role's face, size, line height and tracking. A `.label` also wants its text in capitals: use
    /// `Text(role: .label, …)` or `.textCase(.uppercase)`.
    func textRole(_ role: TextRole) -> some View {
        font(role.font())
            .tracking(role.trackingEm * role.size)
            .lineHeight(role.lineHeight.map { .multiple(factor: $0 / role.size) })
            .textCase(role.uppercased ? .uppercase : nil)
    }
}

/// An ask: one sentence in the serif, its target in italic ("Push 8 commits to *octo/app*"). It reflows to three lines
/// before it shrinks, and never shrinks below 32 pt (20 pt on a waiting card).
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
        let floor: CGFloat = role == .askSmall ? 20 : 32
        runs.reduce(Text(verbatim: "")) { sentence, run in
            Text("\(sentence)\(Text(verbatim: run.text).font(RFont.fixedSerif(size, italic: run.target)))")
        }
        .tracking(role.trackingEm * size)
        .lineHeight(role.lineHeight.map { .multiple(factor: $0 / role.size) })
        .lineLimit(lineLimit)
        .minimumScaleFactor(min(1, floor / size))
        .fixedSize(horizontal: false, vertical: true)
    }
}
