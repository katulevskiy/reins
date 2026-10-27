#if REINS_APP || REINS_WIDGETS
import AppIntents
import SwiftUI
import WidgetKit

// The views of every widget and Live Activity (the Outside board). They live in Shared so the app's debug gallery
// (`-widgetGallery`) draws exactly what the widget extension draws. Fixed type sizes (widgets do not follow Dynamic Type
// the way screens do), Next's paper, ink and fields, and in the accented and vibrant rendering modes no colour that
// carries meaning.
//
// Who asks stays visible wherever something can be approved: the Live Activity and the expanded Dynamic Island show
// the request's icon and the AI's avatar and name (the who line) above the ask. The compact and minimal island, the
// widgets and the Lock Screen accessories have no room for it and offer no approval.

enum GlanceStyle {
    /// The widget plate: paper. A field takes its place only while something waits.
    static let background = Palette.paper
    /// Behind a Live Activity with nothing to decide (a download): paper.
    static let liveBackground = Palette.paper

    static func tint(_ mode: Glance.Mode) -> Color {
        switch mode {
        case .manual: Palette.ink
        case .auto: Palette.violet
        case .overnight: Palette.nightText
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func kind(_ mode: Glance.Mode) -> ModeKind {
        switch mode {
        case .manual: .manual
        case .auto: .auto
        case .overnight: .overnight
        case .bypass: .bypass
        case .lockdown: .lockdown
        }
    }
}

// MARK: Pieces

/// A glyph at a fixed size (widgets do not scale with Dynamic Type).
struct FixedGlyph: View {
    var glyph: Glyph
    var size: CGFloat

    init(_ glyph: Glyph, size: CGFloat) {
        self.glyph = glyph
        self.size = size
    }

    var body: some View {
        Image(glyph: glyph).resizable().scaledToFit().frame(width: size, height: size).accessibilityHidden(true)
    }
}

/// A mode's glyph at a fixed size; Autopilot's bolt is filled.
struct FixedModeGlyph: View {
    var mode: ModeKind
    var size: CGFloat

    var body: some View {
        if mode == .auto { Bolt(size: size).accessibilityHidden(true) } else { FixedGlyph(mode.glyph, size: size) }
    }
}

/// The connection behind an item, as the app draws it: its provider's or its platform's logo on the app's round plate,
/// else its blobatar (in the tinted and vibrant renderings, its initial in a disc; a link glyph for pairings, which
/// have no AI yet).
struct GlanceAvatar: View {
    var label: String
    var kind: Snapshot.Item.Kind = .request
    var icon: String? = nil
    var blobatar: BlobatarLook? = nil
    var size: CGFloat = 28
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        let look = AvatarLook.of(label: label, icon: icon, blobatar: blobatar)
        if kind != .pairing, case let .provider(provider) = look {
            logo(provider.asset, original: !provider.mono, tint: nil)
        } else if kind != .pairing, case let .platform(platform) = look {
            logo(platform.asset, original: false, tint: platform.brand.map { Color(uiColor: UIColor(hex: $0)) })
        } else if kind != .pairing, renderingMode == .fullColor, case let .blob(seed, shape, colour) = look, !Glance.initial(label).isEmpty {
            blob(seed, shape: shape, colour: colour)
        } else {
            initial
        }
    }

    /// A logo; `original` keeps its own colours, `tint` replaces the text colour (both only in full colour).
    private func logo(_ asset: String, original: Bool, tint: Color?) -> some View {
        let full = renderingMode == .fullColor
        return Image(asset)
            .renderingMode(original && full ? .original : .template)
            .resizable()
            .scaledToFit()
            .foregroundStyle(full ? tint ?? Palette.ink : Color.primary)
            .frame(width: size * 0.56, height: size * 0.56)
            .frame(width: size, height: size)
            .background(full ? Palette.dynamic(0xFFFFFF, 0x1D1D21) : Color.primary.opacity(0.16), in: Circle())
            .overlay(Circle().strokeBorder(full ? Palette.rule : .clear, lineWidth: 1))
            .widgetAccentable()
            .accessibilityHidden(true)
    }

    private func blob(_ seed: String, shape: Int?, colour: Int?) -> some View {
        let marks = BlobatarDrawing.marks(for: seed, shape: shape, colour: colour)
        return Canvas { context, canvas in
            let scale = min(canvas.width, canvas.height) / 100
            context.scaleBy(x: scale, y: scale)
            for mark in marks { context.fill(mark.path, with: .color(mark.color)) }
        }
        .frame(width: size * 0.92, height: size * 0.92)
        .frame(width: size, height: size)
        .background(Palette.dynamic(0xFFFFFF, 0x1D1D21), in: Circle())
        .overlay(Circle().strokeBorder(Palette.rule, lineWidth: 1))
        .clipShape(Circle())
        .accessibilityHidden(true)
    }

    private var initial: some View {
        let full = renderingMode == .fullColor
        return Circle()
            .fill(full ? Palette.violetWash : Color.primary.opacity(0.16))
            .overlay {
                Group {
                    if kind == .pairing || kind == .join || Glance.initial(label).isEmpty {
                        FixedGlyph(kind == .pairing ? .link : kind == .join ? .phone : .key, size: size * 0.5)
                    } else {
                        Text(Glance.initial(label)).font(RFont.fixedSans(size * 0.44, .semibold))
                    }
                }
                .foregroundStyle(full ? Palette.violet : Color.primary)
                .widgetAccentable()
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// A request's icon (ASK-TEXT): the service's mark on a white plate, the action as a round badge in the field's
/// colour at its corner, ringed in what is behind it. A pairing or a new phone has a glyph for a mark.
struct GlanceRequestIcon: View {
    var service: String
    var glyph: Glyph
    var field: FieldKind?
    var size: CGFloat = 26
    var ring: Color = Palette.paper
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        let full = renderingMode == .fullColor
        let badge = (size * 0.5).rounded()
        let plate = RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
        ZStack {
            plate.fill(full ? Color.white : Color.primary.opacity(0.16))
            if full { plate.strokeBorder(Color.black.opacity(0.08), lineWidth: 1) }
            mark(full)
        }
        .frame(width: size, height: size)
        .overlay(alignment: .bottomTrailing) {
            FixedGlyph(glyph, size: badge * 0.56)
                .foregroundStyle(Color.white)
                .frame(width: badge, height: badge)
                .background(full ? field?.fill ?? Palette.ink2 : Color.primary.opacity(0.5), in: Circle())
                .padding(1.5)
                .background(full ? ring : .clear, in: Circle())
                .offset(x: badge * 0.32, y: badge * 0.32)
        }
        .widgetAccentable()
        .accessibilityHidden(true)
    }

    @ViewBuilder private func mark(_ full: Bool) -> some View {
        let key = service.hasPrefix("mcp") ? "mcp" : service
        if !key.isEmpty, UIImage(named: "service-\(key)") != nil {
            Image("service-\(key)")
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .foregroundStyle(full ? (key == "github" ? Palette.onFieldInk : serviceColor(key)) : Color.primary)
                .frame(width: size * 0.56, height: size * 0.56)
        } else {
            FixedGlyph(glyph == .phone ? .phone : glyph == .link ? .link : .help, size: size * 0.5)
                .foregroundStyle(full ? Palette.onFieldInk : Color.primary)
        }
    }
}

/// Who asks, on one short line above the ask (the board's who line): the request's icon · the AI's avatar and its
/// name. A pairing or a new phone has no AI yet: its icon and Reins' own word for it.
struct GlanceWhoLine: View {
    var kind: Snapshot.Item.Kind
    var service: String
    var glyph: Glyph
    var field: FieldKind
    var connection: String
    var icon: String?
    var blobatar: BlobatarLook?
    var label: String
    /// What the icon's badge sits on.
    var ring: Color
    var color: Color = .white

    var body: some View {
        HStack(spacing: 8) {
            GlanceRequestIcon(service: service, glyph: glyph, field: field, size: 24, ring: ring)
                .padding(.trailing, 4)
            if kind == .pairing || kind == .join || connection.isEmpty {
                Text(label).font(RFont.fixedSans(14, .semibold)).lineLimit(1)
            } else {
                Text("·").font(RFont.fixedSans(14, .semibold)).opacity(0.6).accessibilityHidden(true)
                // On its white plate, as the boards draw it on a field and on the island's black.
                GlanceAvatar(label: connection, kind: kind, icon: icon, blobatar: blobatar, size: 20)
                    .environment(\.colorScheme, .light)
                Text(Glance.who(connection)).font(RFont.fixedSans(14, .semibold)).lineLimit(1)
            }
        }
        .foregroundStyle(color)
        .accessibilityElement(children: .combine)
    }
}

/// Time left in an answer window, counting down by itself ("0:45", "1:02:03").
struct Countdown: View {
    var from: Date
    var to: Date
    var size: CGFloat = 12
    var color: Color = Palette.ink2
    var weight: RFont.Weight = .medium

    var body: some View {
        Text(timerInterval: min(from, to)...to, countsDown: true)
            .font(RFont.fixedMono(size, weight))
            .monospacedDigit()
            .foregroundStyle(color)
            .multilineTextAlignment(.trailing)
            .lineLimit(1)
    }
}

extension Countdown {
    init(_ item: Snapshot.Item, size: CGFloat = 12, color: Color = Palette.ink2) {
        self.init(
            from: Date(timeIntervalSince1970: TimeInterval(item.createdAt)),
            to: Date(timeIntervalSince1970: TimeInterval(item.expiresAt)),
            size: size,
            color: color
        )
    }
}

/// A ring that empties by itself as a time runs out. Widgets and Live Activities animate a timer `ProgressView`
/// without any updates; the app (the debug gallery) draws it as a spinner, so there it is drawn by hand.
struct TimerRing: View {
    var from: Date
    var to: Date
    var tint: Color
    var lineWidth: CGFloat = 4

    var body: some View {
        #if REINS_WIDGETS
        ProgressView(timerInterval: min(from, to)...to, countsDown: true) {
            EmptyView()
        } currentValueLabel: {
            EmptyView()
        }
        .progressViewStyle(.circular)
        .tint(tint)
        #else
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let total = max(1, to.timeIntervalSince(min(from, to)))
            let left = min(1, max(0, to.timeIntervalSince(context.date) / total))
            ZStack {
                Circle().stroke(tint.opacity(0.28), lineWidth: lineWidth)
                Circle().trim(from: 0, to: left).stroke(tint, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)).rotationEffect(.degrees(-90))
            }
            .padding(lineWidth / 2)
        }
        #endif
    }
}

/// The answer window as the asks show it: a small draining ring and the time left (the board's `timer`).
struct RingTimer: View {
    var from: Date
    var to: Date
    var color: Color = .white
    var size: CGFloat = 13.5

    var body: some View {
        HStack(spacing: 6) {
            TimerRing(from: from, to: to, tint: color, lineWidth: 2.2).frame(width: 18, height: 18)
            Countdown(from: from, to: to, size: size, color: color).frame(maxWidth: 58, alignment: .leading)
        }
    }
}

/// A field's colour with its pre-baked grain: behind a widget or a Live Activity that waits on a decision.
struct GlanceField: View {
    var kind: FieldKind

    var body: some View {
        ZStack {
            kind.fill
            WidgetGrain()
        }
    }
}

/// A widget's quiet heading ("Latest", "Mode", "3 waiting"), with an optional glyph.
struct GlanceHeading: View {
    var text: String
    var glyph: Glyph? = nil
    var color: Color = Palette.inkMeta

    var body: some View {
        HStack(spacing: 6) {
            if let glyph { FixedGlyph(glyph, size: 16) }
            Text(text).font(RFont.fixedSans(13, .semibold)).lineLimit(1)
        }
        .foregroundStyle(color)
    }
}

/// The capsule on widget and Live Activity buttons, in the colours the place gives it.
struct GlanceButtonLabel: View {
    var title: String
    var foreground: Color
    var background: Color
    var height: CGFloat = 40
    var glyph: Glyph? = nil
    var mode: ModeKind? = nil

    var body: some View {
        HStack(spacing: 6) {
            if let mode { FixedModeGlyph(mode: mode, size: 16) } else if let glyph { FixedGlyph(glyph, size: 16) }
            Text(title).font(RFont.fixedSans(14.5, .semibold)).lineLimit(1).minimumScaleFactor(0.8)
        }
        .foregroundStyle(foreground)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(Capsule().fill(background))
        .contentShape(Capsule())
    }
}

/// "+2": how many more wait, on a field or the island's black.
struct MorePill: View {
    var count: Int
    var background: Color = Color.white.opacity(0.22)

    var body: some View {
        Text("+\(count)")
            .font(RFont.fixedSans(13, .semibold))
            .monospacedDigit()
            .foregroundStyle(.white)
            .padding(.horizontal, 8)
            .frame(minWidth: 30, minHeight: 24)
            .background(Capsule().fill(background))
            .accessibilityLabel("\(count) more waiting")
    }
}

/// Nothing to show: signed out, nothing waits, nothing happened. On paper; the pond when nothing waits.
struct GlanceEmpty: View {
    var title: String
    var message: String
    var pond = false
    var compact = false

    var body: some View {
        VStack(alignment: compact ? .leading : .center, spacing: 4) {
            if pond {
                Image("pond")
                    .resizable()
                    .scaledToFit()
                    .frame(maxHeight: compact ? 54 : 64)
                    .frame(maxWidth: .infinity, alignment: compact ? .leading : .center)
                    .accessibilityHidden(true)
                    .padding(.bottom, 2)
            }
            Text(title).font(RFont.fixedDisplay(compact ? 16 : 15)).foregroundStyle(Palette.ink)
            Text(message)
                .font(RFont.fixedSans(11))
                .foregroundStyle(Palette.ink2)
                .multilineTextAlignment(compact ? .leading : .center)
                .lineLimit(3)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: compact ? .bottomLeading : .center)
    }
}

extension GlanceEmpty {
    static func signedOut(compact: Bool = false) -> GlanceEmpty {
        GlanceEmpty(title: "Sign in to Reins", message: "Open the app to sign in.", compact: compact)
    }

    static func nothingWaits(_ s: Snapshot, compact: Bool = false) -> GlanceEmpty {
        GlanceEmpty(
            title: "Nothing waits.",
            message: s.approvalDevice || !s.signedIn ? "Requests from your AIs show up here." : "Requests go to your other phone.",
            pond: true,
            compact: compact
        )
    }
}

// MARK: Waiting

/// One waiting item in a list: its request icon, the ask, who asks (the avatar), time left. Opens the item; nothing is
/// approved from here.
struct WaitingRow: View {
    var item: Snapshot.Item

    var body: some View {
        HStack(spacing: 10) {
            GlanceRequestIcon(service: item.service, glyph: Glance.actionGlyph(item), field: Glance.field(item), size: 26)
                .padding(.trailing, 3)
            Text(Glance.operation(item)).font(RFont.fixedSans(13.5, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if item.kind != .pairing, item.kind != .join, !item.connection.isEmpty {
                GlanceAvatar(label: item.connection, kind: item.kind, icon: item.connectionIcon, blobatar: item.connectionBlobatar, size: 20)
            }
            Countdown(item, size: 12, color: Palette.ink2)
                .frame(width: 40, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(item.kind == .request && !item.connection.isEmpty ? "\(Glance.who(item.connection)): \(Glance.operation(item))" : Glance.operation(item))
    }
}

/// "Waiting", small: a field on the Home Screen only while something waits (the newest's colour), how many, the newest
/// ask; with nothing waiting, paper and the pond.
struct WaitingSmallView: View {
    var snapshot: Snapshot
    var now: Int64
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        if !snapshot.signedIn {
            GlanceEmpty.signedOut(compact: true)
        } else if let newest = list.first {
            let ink: Color = renderingMode == .fullColor ? Palette.onField : .primary
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    FixedGlyph(.tray, size: 18)
                    Spacer(minLength: 4)
                    RingTimer(
                        from: Date(timeIntervalSince1970: TimeInterval(newest.createdAt)),
                        to: Date(timeIntervalSince1970: TimeInterval(newest.expiresAt)), color: ink
                    )
                    .fixedSize()
                }
                Text("\(list.count)")
                    .font(RFont.fixedDisplay(42))
                    .padding(.top, 2)
                    .widgetAccentable()
                Spacer(minLength: 2)
                Text(Glance.operation(newest))
                    .font(RFont.fixedDisplay(15.5))
                    .lineLimit(2)
                    .minimumScaleFactor(0.85)
            }
            .foregroundStyle(ink)
            .accessibilityElement(children: .combine)
            .accessibilityLabel("\(Glance.countLine(list.count)). Newest: \(Glance.operation(newest))")
        } else {
            GlanceEmpty.nothingWaits(snapshot, compact: true)
        }
    }
}

/// "Waiting", medium and large: how many, then the list on paper, each row opening its item.
struct WaitingListView: View {
    var snapshot: Snapshot
    var now: Int64
    var rows: Int

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        VStack(alignment: .leading, spacing: 0) {
            GlanceHeading(text: list.isEmpty ? "Waiting" : Glance.countLine(list.count), glyph: .tray)
            if !snapshot.signedIn {
                GlanceEmpty.signedOut()
            } else if list.isEmpty {
                GlanceEmpty.nothingWaits(snapshot)
            } else {
                Spacer(minLength: 6)
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(list.prefix(rows)) { item in
                        Link(destination: Glance.link(item)) { WaitingRow(item: item) }
                    }
                }
                Spacer(minLength: 0)
                if list.count > rows {
                    Text("+\(list.count - rows) more")
                        .font(RFont.fixedSans(11.5, .medium))
                        .foregroundStyle(Palette.inkMeta)
                        .padding(.top, 4)
                }
            }
        }
    }
}

// MARK: Autopilot

/// "Mode": the mode in force with its glyph, a bypass's countdown, and Lock down / End lockdown / Stop bypass as
/// buttons that run their App Intents (which ask for the phone's owner) without opening Reins.
struct AutopilotWidgetView: View {
    var snapshot: Snapshot
    var now: Int64
    var medium: Bool

    var body: some View {
        if !snapshot.signedIn {
            GlanceEmpty.signedOut(compact: !medium)
        } else if medium {
            HStack(alignment: .top, spacing: 14) {
                summary
                VStack(spacing: 8) {
                    Spacer(minLength: 0)
                    buttons
                }
                .frame(width: 140)
            }
        } else {
            VStack(alignment: .leading, spacing: 0) {
                summary
                Spacer(minLength: 6)
                primaryButton
            }
        }
    }

    private var mode: Glance.Mode { Glance.mode(snapshot, now: now) }
    private var bypassEnd: Int64? { Glance.bypassEnd(snapshot, now: now) }

    private var summary: some View {
        VStack(alignment: .leading, spacing: 2) {
            GlanceHeading(text: "Mode")
            Spacer(minLength: 4)
            HStack(spacing: 8) {
                FixedModeGlyph(mode: GlanceStyle.kind(mode), size: 20)
                    .foregroundStyle(mode == .manual ? Palette.ink : GlanceStyle.tint(mode))
                    .widgetAccentable()
                Text(mode.name)
                    .font(RFont.fixedDisplay(25.5))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .minimumScaleFactor(0.6)
            }
            if let end = bypassEnd {
                let someAIs = Glance.connectionBypassOnly(snapshot, now: now)
                HStack(spacing: 4) {
                    if AutopilotText.isUntilOff(end) {
                        // No end to count down to.
                        Text(someAIs ? "Bypass for some AIs · until you stop it" : "Until you stop it")
                            .font(RFont.fixedSans(11.5))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                    } else {
                        Text(someAIs ? "Bypass for some AIs" : "Ends in")
                            .font(RFont.fixedSans(11.5))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                        Countdown(from: Date(timeIntervalSince1970: TimeInterval(now)), to: Date(timeIntervalSince1970: TimeInterval(end)), size: 11.5, color: Palette.danger)
                            .frame(maxWidth: 48, alignment: .leading)
                    }
                }
            } else if medium {
                Text(mode.line).font(RFont.fixedSans(11.5)).foregroundStyle(Palette.ink2).lineLimit(2)
            }
            if medium {
                Spacer(minLength: 0)
                Text(snapshot.activeGrants == 1 ? "1 active grant" : "\(snapshot.activeGrants) active grants")
                    .font(RFont.fixedSans(11.5, .medium))
                    .foregroundStyle(Palette.inkMeta)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The one button a small widget has room for: Stop while a bypass runs, else Lockdown on or off.
    @ViewBuilder private var primaryButton: some View {
        if bypassEnd != nil {
            stopButton
        } else {
            lockdownButton
        }
    }

    @ViewBuilder private var buttons: some View {
        if bypassEnd != nil { stopButton }
        lockdownButton
    }

    private var stopButton: some View {
        Button(intent: StopBypassIntent()) {
            GlanceButtonLabel(title: "Stop bypass", foreground: Palette.onField, background: Palette.fieldDelete)
        }
        .buttonStyle(.plain)
    }

    private var lockdownButton: some View {
        let on = mode == .lockdown
        // Both ways run `SetLockdownIntent`, which asks for the phone's owner first.
        return Button(intent: SetLockdownIntent(value: !on)) {
            GlanceButtonLabel(
                title: on ? "End lockdown" : "Lock down",
                foreground: on ? Palette.onField : Palette.onInverse,
                background: on ? Palette.fieldWarn : Palette.inverse,
                mode: on ? nil : .lockdown
            )
        }
        .buttonStyle(.plain)
    }
}

// MARK: Latest activity

struct GlanceActivityRow: View {
    var entry: Snapshot.Entry
    var now: Int64

    var body: some View {
        HStack(spacing: 10) {
            GlanceRequestIcon(
                service: entry.service ?? "", glyph: Glance.actionGlyph(entry.action, field: nil),
                field: Glance.entryField(entry.action), size: 26
            )
            .padding(.trailing, 3)
            Text(entry.title).font(RFont.fixedSans(13.5, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(Glance.shortAgo(entry.at, now: now)).font(RFont.fixedMono(11.5)).foregroundStyle(Palette.inkMeta)
            FixedGlyph(entry.approved ? .check : .close, size: entry.approved ? 17 : 16)
                .foregroundStyle(entry.approved ? Palette.success : Palette.danger)
                .frame(width: 18)
                .widgetAccentable()
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(entry.title), \(entry.approved ? "went through" : entry.outcome.lowercased())")
    }
}

struct LatestActivityView: View {
    var snapshot: Snapshot
    var now: Int64
    var rows: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GlanceHeading(text: "Latest")
            if !snapshot.signedIn {
                GlanceEmpty.signedOut()
            } else if snapshot.latest.isEmpty {
                GlanceEmpty(title: "Nothing happened yet", message: "What your AIs did shows up here.")
            } else {
                Spacer(minLength: 6)
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(snapshot.latest.prefix(rows)) { entry in
                        Link(destination: DeepLink.activity(id: entry.id).url) { GlanceActivityRow(entry: entry, now: now) }
                    }
                }
                Spacer(minLength: 0)
            }
        }
    }
}

// MARK: Lock Screen accessories

/// A ring that fills with what waits, the count inside.
struct WaitingCircularView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let n = Glance.waiting(snapshot, now: now).count
        if snapshot.signedIn {
            ZStack {
                AccessoryWidgetBackground()
                Gauge(value: Double(n), in: 0...Double(max(5, n))) {
                    FixedGlyph(.tray, size: 12)
                } currentValueLabel: {
                    Text("\(n)").font(RFont.fixedMono(18, .semibold))
                }
                .gaugeStyle(.accessoryCircularCapacity)
                .padding(4)
                .widgetAccentable()
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Glance.countLine(n))
        } else {
            ZStack {
                AccessoryWidgetBackground()
                FixedGlyph(.person, size: 22)
            }
            .accessibilityLabel("Sign in to Reins")
        }
    }
}

/// The newest request: how many, time left, what. Opens it.
struct WaitingRectangularView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 5) {
                FixedGlyph(.tray, size: 14)
                Text(snapshot.signedIn ? "\(list.count)" : "Reins").font(RFont.fixedSans(13, .semibold)).lineLimit(1)
                Spacer(minLength: 2)
                if let newest = list.first {
                    Countdown(newest, size: 12, color: .primary).frame(maxWidth: 48, alignment: .trailing)
                }
            }
            .widgetAccentable()
            if !snapshot.signedIn {
                Text("Open the app to sign in").font(RFont.fixedSans(13)).foregroundStyle(.secondary).lineLimit(2)
            } else if let newest = list.first {
                Text(Glance.operation(newest)).font(RFont.fixedSans(14, .medium)).lineLimit(2)
            } else {
                Text("Nothing waits").font(RFont.fixedSans(13)).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

struct WaitingInlineView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        if !snapshot.signedIn {
            Label { Text("Sign in to Reins") } icon: { Image(glyph: .tray) }
        } else {
            Label { Text(Glance.countLine(list.count)) } icon: { Image(glyph: .tray) }
        }
    }
}

// MARK: Live Activity: requests waiting

/// The field leaves the app: a waiting request is a Live Activity in its own field's colour, grain and all: the time
/// left and how many more wait, who asks (icon · avatar Name), the ask, its detail, Deny and Review (and Approve for a
/// routine one). One no longer waiting is plain paper. The Dynamic Island stays black; the field arrives there as a
/// dot, the timer and the Review button.
struct ApprovalLiveViews {
    var state: ApprovalActivityAttributes.ContentState
    var stale: Bool

    var link: URL { DeepLink.item(kind: state.kind, id: state.itemId).url }

    /// The field Reins picked from the core's kind (never from the ask's words).
    var field: FieldKind { Glance.field(state.field, kind: state.kind) }

    var label: String {
        if let label = state.label, !label.isEmpty { return label }
        return Glance.label(item)
    }

    private var item: Snapshot.Item {
        Snapshot.Item(
            id: state.itemId, kind: state.kind, title: state.title, subtitle: state.subtitle, connection: state.connection,
            service: state.service, createdAt: 0, expiresAt: 0, suggestion: nil, field: state.field, label: state.label
        )
    }

    var glyph: Glyph { Glance.actionGlyph(item) }

    var title: String { stale ? "No longer waiting" : state.title }

    var more: Int { max(0, state.count - 1) }

    /// One-tap Approve: only a routine request, only while it waits, and only where who asks is shown with it.
    var offersApprove: Bool { state.kind == .request && state.quick == true && !stale }

    /// The who line, on `ring` (the field, or the island's black).
    func who(ring: Color, color: Color = .white) -> GlanceWhoLine {
        GlanceWhoLine(
            kind: state.kind, service: state.service, glyph: glyph, field: field, connection: state.connection,
            icon: state.connectionIcon, blobatar: state.connectionBlobatar, label: label, ring: ring, color: color
        )
    }

    /// Deny (requests only: pairings and files are answered in the app), Approve for a routine request (the same
    /// one-tap answer as the notification's), and Review. On the field, or on the island's black.
    @ViewBuilder func buttons(island: Bool) -> some View {
        let quick = offersApprove
        let wash = island ? Color.white.opacity(0.16) : Color.white.opacity(0.18)
        let solidText = island ? Color.white : field.fill
        let solid = island ? field.fill : Palette.onField
        HStack(spacing: 10) {
            if state.kind == .request {
                Button(intent: DenyRequestIntent(requestId: state.itemId)) {
                    GlanceButtonLabel(title: "Deny", foreground: .white, background: wash)
                }
                .buttonStyle(.plain)
            }
            if quick {
                Button(intent: ApproveQuickIntent(requestId: state.itemId)) {
                    GlanceButtonLabel(title: "Approve", foreground: solidText, background: solid)
                }
                .buttonStyle(.plain)
            }
            Link(destination: link) {
                GlanceButtonLabel(title: "Review", foreground: quick ? .white : solidText, background: quick ? wash : solid)
            }
        }
    }

    var lockScreen: some View {
        let ink: Color = stale ? Palette.ink : Palette.onField
        return VStack(alignment: .leading, spacing: 10) {
            if !stale {
                HStack {
                    RingTimer(from: state.createdAt, to: state.expiresAt, color: ink).fixedSize()
                    Spacer(minLength: 6)
                    if more > 0 { MorePill(count: more) }
                }
            }
            who(ring: stale ? Palette.paper : field.fill, color: ink)
            Text(title)
                .font(RFont.fixedDisplay(stale ? 16 : 22.5))
                .foregroundStyle(ink)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
            if !state.subtitle.isEmpty, !stale {
                Text(state.subtitle)
                    .font(RFont.fixedMono(13))
                    .foregroundStyle(Color.white.opacity(0.85))
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .padding(.top, -4)
            }
            if !stale { buttons(island: false) }
        }
        .padding(.horizontal, 16)
        .padding(.top, 14)
        .padding(.bottom, 16)
        .background { if !stale { WidgetGrain() } }
    }

    /// Behind the Lock Screen view: the field while it waits, paper once it does not.
    var background: Color { stale ? Palette.paper : field.fill }

    // The Dynamic Island: black, with a spark of the field.

    /// A dot in the kind's colour, glowing a little.
    func dot(_ size: CGFloat) -> some View {
        Circle()
            .fill(field.fill)
            .frame(width: size, height: size)
            .shadow(color: field.fill.opacity(0.9), radius: size * 0.45)
            .accessibilityHidden(true)
    }

    var compactLeading: some View { dot(10).padding(.leading, 4) }

    var compactTrailing: some View {
        Countdown(from: state.createdAt, to: state.expiresAt, size: 13, color: field.ink, weight: .semibold)
            .environment(\.colorScheme, .dark)
            .frame(maxWidth: 46)
    }

    var minimal: some View {
        dot(12).accessibilityLabel(Glance.countLine(state.count))
    }

    var expandedLeading: some View {
        who(ring: .black)
            .padding(.leading, 6)
            .padding(.top, 4)
    }

    var expandedTrailing: some View {
        HStack(spacing: 8) {
            if more > 0 { MorePill(count: more, background: Color.white.opacity(0.2)) }
            Countdown(from: state.createdAt, to: state.expiresAt, size: 15, color: field.ink, weight: .semibold)
                .environment(\.colorScheme, .dark)
                .frame(maxWidth: 52, alignment: .trailing)
        }
        .padding(.trailing, 6)
        .padding(.top, 4)
    }

    var expandedCenter: some View { EmptyView() }

    var expandedBottom: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(RFont.fixedDisplay(19.5)).foregroundStyle(.white).lineLimit(2)
            buttons(island: true)
        }
        .padding(.horizontal, 6)
        .padding(.top, 4)
    }
}

// MARK: Live Activity: bypass

/// A running bypass: a red field with its glyph and name, what it approved so far, the time left big and Stop now; an
/// Overnight the same in its colours, with the moon. One that runs until turned off says so instead of counting down.
/// Ended, it is paper.
struct BypassLiveViews {
    var state: BypassActivityAttributes.ContentState
    var stale: Bool

    var untilOff: Bool { state.untilOff ?? false }

    var overnight: Bool { state.overnight ?? false }

    var ended: Bool { stale || (!untilOff && state.until <= Date()) }

    var mode: ModeKind { overnight ? .overnight : .bypass }

    var name: String { overnight ? AutopilotText.overnightName : "Bypass" }

    /// The field: red for a bypass, Overnight's night blue.
    var fill: Color { overnight ? Palette.modeOvernight : Palette.fieldDelete }

    /// Its colour on the island's black.
    var islandInk: Color { overnight ? Color(uiColor: UIColor(hex: 0x9DB1EA)) : Color(uiColor: UIColor(hex: 0xF87171)) }

    /// The colour the system tints its own controls with.
    var tint: Color { fill }

    /// "Bypass", or "Bypass · Claude" when it is not for every AI.
    var heading: String {
        if ended { return "\(name) ended" }
        return state.scope == Glance.bypassScope(global: true, connections: []) ? name : "\(name) · \(state.scope)"
    }

    var title: String {
        if ended { return "\(name) ended" }
        return "\(name) on · \(state.scope)"
    }

    var approved: String { state.approvedCount == 0 ? "Nothing approved yet" : "\(state.approvedCount) approved so far" }

    /// Under an ended one's heading: what it let through.
    var endedLine: String { state.approvedCount == 0 ? "Nothing was approved" : "\(state.approvedCount) approved" }

    @ViewBuilder func timer(size: CGFloat, color: Color) -> some View {
        if untilOff {
            Text("Until you stop it")
                .font(RFont.fixedDisplay(size * 0.5))
                .foregroundStyle(color)
                .lineLimit(1)
        } else {
            Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
                .font(RFont.fixedDisplay(size))
                .monospacedDigit()
                .foregroundStyle(color)
                .lineLimit(1)
        }
    }

    /// Stop: on the Lock Screen a pill sized to its words beside the countdown, on the island a button beside its timer.
    @ViewBuilder func stop(island: Bool) -> some View {
        let foreground = island ? Color.white : fill
        let background = island ? fill : Palette.onField
        let label = GlanceButtonLabel(title: island ? "Stop" : "Stop now", foreground: foreground, background: background, height: island ? 40 : 36)
            .fixedSize(horizontal: !island, vertical: false)
        Group {
            if overnight {
                Button(intent: StopOvernightIntent()) { label }
            } else {
                Button(intent: StopBypassIntent()) { label }
            }
        }
        .buttonStyle(.plain)
    }

    /// The Lock Screen's card, well inside the system's 160 pt (about 110): the heading row, then the countdown with
    /// Stop beside it. What a bypass still asks is the Bypass sheet's, in the app. Taller content is cut off at the top
    /// by the system.
    var lockScreen: some View {
        let ink: Color = ended ? Palette.ink : Palette.onField
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                FixedModeGlyph(mode: mode, size: 16)
                Text(heading).font(RFont.fixedSans(14, .semibold)).lineLimit(1)
                Spacer(minLength: 6)
                if !ended {
                    HStack(spacing: 4) {
                        FixedGlyph(.check, size: 13)
                        Text("\(state.approvedCount)").font(RFont.fixedSans(13.5, .semibold)).monospacedDigit()
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(approved)
                }
            }
            if !ended {
                HStack(alignment: .center, spacing: 12) {
                    timer(size: 40, color: ink)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    stop(island: false)
                }
            } else {
                Text(endedLine).font(RFont.fixedSans(12)).foregroundStyle(Palette.ink2).lineLimit(1)
            }
        }
        .foregroundStyle(ink)
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .background { if !ended { WidgetGrain() } }
    }

    var background: Color { ended ? Palette.paper : fill }

    var compactLeading: some View {
        FixedModeGlyph(mode: mode, size: 17).foregroundStyle(islandInk).padding(.leading, 4)
    }

    @ViewBuilder var compactTrailing: some View {
        if untilOff {
            Text("on").font(RFont.fixedMono(13, .semibold)).foregroundStyle(islandInk)
        } else {
            Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
                .font(RFont.fixedMono(13, .semibold))
                .monospacedDigit()
                .foregroundStyle(islandInk)
                .multilineTextAlignment(.trailing)
                // Room for hours ("22:25:22") when it runs an hour or more, else for "47:12".
                .frame(width: Self.compactTimerWidth(state), alignment: .trailing)
        }
    }

    /// The compact timer's width: enough for the longest it shows, which is its whole length.
    static func compactTimerWidth(_ state: BypassActivityAttributes.ContentState) -> CGFloat {
        state.until.timeIntervalSince(min(state.startedAt, state.until)) >= 3_600 ? 64 : 46
    }

    var minimal: some View {
        FixedModeGlyph(mode: mode, size: 15).foregroundStyle(islandInk)
            .accessibilityLabel(untilOff && !ended ? "\(title), until you stop it" : title)
    }

    var expandedLeading: some View {
        HStack(spacing: 6) {
            FixedModeGlyph(mode: mode, size: 16)
            Text(heading).font(RFont.fixedSans(14, .semibold)).lineLimit(1)
        }
        .foregroundStyle(islandInk)
        .padding(.leading, 6)
        .padding(.top, 6)
    }

    var expandedTrailing: some View { EmptyView() }

    var expandedCenter: some View { EmptyView() }

    var expandedBottom: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center) {
                timer(size: 34, color: .white)
                Spacer(minLength: 8)
                stop(island: true).frame(width: 110)
            }
            Text(approved).font(RFont.fixedSans(12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .padding(.horizontal, 6)
    }
}

// MARK: Live Activity: model download

/// Autopilot's model coming down: nothing to decide, so paper. Its bolt on a violet tile with a download badge, the
/// percentage, a bar and the megabytes.
struct ModelDownloadLiveViews {
    var state: ModelDownloadActivityAttributes.ContentState

    var title: String {
        if state.failed { return "The model did not download" }
        if state.finished { return "Autopilot's model is ready" }
        return "Autopilot model"
    }

    var badgeGlyph: Glyph { state.failed ? .warning : state.finished ? .check : .download }

    var tint: Color { state.failed ? Palette.warning : Palette.violet }

    var percent: String { "\(Int((Glance.downloadFraction(state) * 100).rounded()))%" }

    var tile: some View {
        Bolt(size: 20)
            .foregroundStyle(Palette.violet)
            .frame(width: 38, height: 38)
            .background(Palette.violet.opacity(0.14), in: RoundedRectangle(cornerRadius: 11, style: .continuous))
            .overlay(alignment: .bottomTrailing) {
                FixedGlyph(badgeGlyph, size: 11)
                    .foregroundStyle(.white)
                    .frame(width: 18, height: 18)
                    .background(state.failed ? Palette.fieldWarn : Palette.violetFill, in: Circle())
                    .padding(2)
                    .background(Palette.paper, in: Circle())
                    .offset(x: 7, y: 7)
            }
            .accessibilityHidden(true)
    }

    var bar: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(Palette.rule2)
                Capsule().fill(tint).frame(width: geo.size.width * Glance.downloadFraction(state))
            }
        }
        .frame(height: 6)
    }

    var lockScreen: some View {
        HStack(spacing: 14) {
            tile
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .firstTextBaseline) {
                    Text(title).font(RFont.fixedSans(14.5, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                    Spacer(minLength: 4)
                    if !state.finished && !state.failed {
                        Text(percent).font(RFont.fixedMono(13, .semibold)).foregroundStyle(Palette.ink)
                    }
                }
                .padding(.bottom, 8)
                if !state.finished && !state.failed { bar }
                Text(Glance.downloadLine(state)).font(RFont.fixedMono(12)).foregroundStyle(Palette.inkMeta).lineLimit(1).padding(.top, 6)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 16)
    }

    var compactLeading: some View {
        Bolt(size: 16).foregroundStyle(Color(uiColor: UIColor(hex: 0xA196FF))).padding(.leading, 4)
    }

    var compactTrailing: some View {
        HStack(spacing: 5) {
            if !state.finished { FixedGlyph(.download, size: 14) }
            Text(state.finished ? "Ready" : percent).font(RFont.fixedMono(13, .semibold))
        }
        .foregroundStyle(.white)
    }

    /// A drawn ring: the app updates the activity on every whole percent.
    var minimal: some View {
        ZStack {
            Circle().stroke(Color.white.opacity(0.25), lineWidth: 3)
            Circle().trim(from: 0, to: Glance.downloadFraction(state)).stroke(Color(uiColor: UIColor(hex: 0xA196FF)), style: StrokeStyle(lineWidth: 3, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .padding(1.5)
        .accessibilityLabel(title)
    }

    var expandedLeading: some View {
        tile.environment(\.colorScheme, .dark).padding(.leading, 4).padding(.top, 4)
    }

    var expandedTrailing: some View {
        Text(state.finished ? "" : percent).font(RFont.fixedMono(17, .semibold)).foregroundStyle(.white).padding(.trailing, 6).padding(.top, 12)
    }

    var expandedCenter: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(.white).lineLimit(1)
            Text(Glance.downloadLine(state)).font(RFont.fixedMono(12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    @ViewBuilder var expandedBottom: some View {
        if !state.finished && !state.failed { bar.environment(\.colorScheme, .dark).padding(.top, 8) }
    }
}
#endif
