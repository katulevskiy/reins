#if REINS_APP || REINS_WIDGETS
import AppIntents
import SwiftUI
import WidgetKit

// The views of every widget and Live Activity. They live in Shared so the app's debug gallery (`-widgetGallery`)
// draws exactly what the widget extension draws. Fixed type sizes (widgets do not follow Dynamic Type the way
// screens do), Signal's paper, ink and fields, and in the accented and vibrant rendering modes no colour that carries
// meaning.

enum GlanceStyle {
    /// The widget plate: paper. A field takes its place only while something waits.
    static let background = Palette.paper
    /// Behind a Live Activity with nothing to decide (a download): paper.
    static let liveBackground = Palette.paper

    static func tint(_ mode: Glance.Mode) -> Color {
        switch mode {
        case .manual: Palette.kindRead
        case .auto: Palette.violet
        case .overnight: Palette.violet
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }
}

// MARK: Pieces

/// The AI behind an item: its provider's logo on the app's round plate when one is picked or its name suggests one,
/// else its blobatar, the same figure the app draws (in the tinted and vibrant renderings, its initial in a disc; a
/// link glyph for pairings, which have no AI yet).
struct GlanceAvatar: View {
    var label: String
    var kind: Snapshot.Item.Kind = .request
    var icon: String? = nil
    var size: CGFloat = 28
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        if kind != .pairing, let provider = Providers.resolve(label: label, pick: icon) {
            logo(provider)
        } else if kind != .pairing, renderingMode == .fullColor, !Glance.initial(label).isEmpty {
            blob
        } else {
            initial
        }
    }

    private func logo(_ provider: Provider) -> some View {
        let full = renderingMode == .fullColor
        return Image(provider.asset)
            .renderingMode(provider.mono || !full ? .template : .original)
            .resizable()
            .scaledToFit()
            .foregroundStyle(full ? Palette.ink : Color.primary)
            .frame(width: size * 0.56, height: size * 0.56)
            .frame(width: size, height: size)
            .background(full ? Palette.dynamic(0xFFFFFF, 0x1D1D21) : Color.primary.opacity(0.16), in: Circle())
            .overlay(Circle().strokeBorder(full ? Palette.rule : .clear, lineWidth: 1))
            .widgetAccentable()
            .accessibilityHidden(true)
    }

    private var blob: some View {
        let marks = BlobatarDrawing.marks(for: label)
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
                        Image(systemName: kind == .pairing ? "link" : kind == .join ? "iphone.gen3" : "key.fill")
                            .font(.system(size: size * 0.4, weight: .semibold))
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

/// Time left in an answer window, counting down by itself ("0:45", "1:02:03").
struct Countdown: View {
    var from: Date
    var to: Date
    var size: CGFloat = 12
    var color: Color = Palette.ink2

    var body: some View {
        Text(timerInterval: min(from, to)...to, countsDown: true)
            .font(RFont.fixedMono(size, .medium))
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
                Circle().stroke(tint.opacity(0.2), lineWidth: lineWidth)
                Circle().trim(from: 0, to: left).stroke(tint, style: StrokeStyle(lineWidth: lineWidth, lineCap: .round)).rotationEffect(.degrees(-90))
            }
            .padding(lineWidth / 2)
        }
        #endif
    }
}

/// The answer window's ring with the time left inside.
struct CountdownRing: View {
    var from: Date
    var to: Date
    var size: CGFloat = 44
    var tint: Color = Palette.violet

    var body: some View {
        ZStack {
            TimerRing(from: from, to: to, tint: tint)
            Text(timerInterval: min(from, to)...to, countsDown: true, showsHours: false)
                .font(RFont.fixedMono(size * 0.24, .semibold))
                .monospacedDigit()
                .multilineTextAlignment(.center)
                .frame(width: size * 0.8)
        }
        .frame(width: size, height: size)
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

/// The spaced label over an ask or a widget ("GITHUB · PUSH", "WAITING").
struct GlanceLabel: View {
    var text: String
    var color: Color

    var body: some View {
        Text(text.uppercased())
            .font(RFont.fixedMono(10.5, .medium))
            .tracking(1.2)
            .foregroundStyle(color)
            .lineLimit(1)
    }
}

/// The capsule on widget and Live Activity buttons: words only, in the colours the place gives it.
struct GlanceButtonLabel: View {
    var title: String
    var foreground: Color
    var background: Color
    var height: CGFloat = 36

    var body: some View {
        Text(title)
            .font(RFont.fixedSans(14, .semibold))
            .lineLimit(1)
            .foregroundStyle(foreground)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
            .background(Capsule().fill(background))
            .contentShape(Capsule())
    }
}

/// A mode's dot in its colour (Manual has none of its own: ink).
struct ModeDot: View {
    var mode: Glance.Mode
    var size: CGFloat = 8
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        Circle()
            .fill(renderingMode == .fullColor ? GlanceStyle.tint(mode) : Color.primary)
            .frame(width: size, height: size)
            .widgetAccentable()
            .accessibilityHidden(true)
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
            Text(title).font(RFont.fixedSerif(compact ? 22 : 20)).foregroundStyle(Palette.ink)
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

/// One waiting item in a list: a bar in its field's colour, the ask, who and for which account, time left.
struct WaitingRow: View {
    var item: Snapshot.Item
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        HStack(spacing: 10) {
            Capsule()
                .fill(renderingMode == .fullColor ? Glance.field(item).fill : Color.primary)
                .frame(width: 4, height: 30)
                .widgetAccentable()
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(Glance.operation(item)).font(RFont.fixedSans(13, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                Text([Glance.label(item), Glance.byline(item)].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(RFont.fixedSans(11))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
            }
            Spacer(minLength: 6)
            Countdown(item, color: Palette.ink2)
                .frame(maxWidth: 56, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
    }
}

/// "Waiting", small: a field on the Home Screen only while something waits (the newest's colour, its count and its
/// ask in the serif); with nothing waiting, paper and the pond.
struct WaitingSmallView: View {
    var snapshot: Snapshot
    var now: Int64
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        if !snapshot.signedIn {
            GlanceEmpty.signedOut(compact: true)
        } else if let newest = list.first {
            let full = renderingMode == .fullColor
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    GlanceLabel(text: "Waiting", color: full ? Palette.onField2 : .secondary)
                    Spacer(minLength: 4)
                    Countdown(newest, size: 11, color: full ? Palette.onField : .primary)
                        .frame(maxWidth: 52, alignment: .trailing)
                }
                Text("\(list.count)")
                    .font(RFont.fixedSerif(44))
                    .foregroundStyle(full ? Palette.onField : .primary)
                    .widgetAccentable()
                Spacer(minLength: 4)
                Text(Glance.operation(newest))
                    .font(RFont.fixedSerif(17))
                    .foregroundStyle(full ? Palette.onField : .primary)
                    .lineLimit(3)
                    .minimumScaleFactor(0.85)
            }
            .accessibilityElement(children: .combine)
        } else {
            GlanceEmpty.nothingWaits(snapshot, compact: true)
        }
    }
}

/// "Waiting", medium and large: the list on paper, each row opening its item.
struct WaitingListView: View {
    var snapshot: Snapshot
    var now: Int64
    var rows: Int

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        VStack(alignment: .leading, spacing: 0) {
            GlanceLabel(text: list.isEmpty ? "Waiting" : "Waiting · \(list.count)", color: Palette.inkMeta)
            if !snapshot.signedIn {
                GlanceEmpty.signedOut()
            } else if list.isEmpty {
                GlanceEmpty.nothingWaits(snapshot)
            } else {
                Spacer(minLength: 6)
                VStack(alignment: .leading, spacing: rows > 3 ? 10 : 8) {
                    ForEach(list.prefix(rows)) { item in
                        Link(destination: Glance.link(item)) { WaitingRow(item: item) }
                    }
                }
                Spacer(minLength: 0)
                if list.count > rows {
                    Text("and \(list.count - rows) more")
                        .font(RFont.fixedSans(11, .medium))
                        .foregroundStyle(Palette.inkMeta)
                        .padding(.top, 4)
                }
            }
        }
    }
}

// MARK: Autopilot

/// "Autopilot": the mode in force in the serif, a bypass's countdown, and Lock down / Stop bypass as buttons that run
/// their App Intents without opening Reins.
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
                .frame(width: 128)
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
            GlanceLabel(text: "Autopilot", color: Palette.inkMeta)
                .padding(.bottom, 4)
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                Text(mode.name)
                    .font(RFont.fixedSerif(medium ? 32 : 28))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                ModeDot(mode: mode)
            }
            if let end = bypassEnd {
                let someAIs = Glance.connectionBypassOnly(snapshot, now: now)
                HStack(spacing: 4) {
                    if AutopilotText.isUntilOff(end) {
                        // No end to count down to.
                        Text(someAIs ? "Bypass for some AIs · until you stop it" : "Until you stop it")
                            .font(RFont.fixedSans(11))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                    } else {
                        Text(someAIs ? "Bypass for some AIs" : "Ends in")
                            .font(RFont.fixedSans(11))
                            .foregroundStyle(Palette.ink2)
                            .lineLimit(1)
                        Countdown(from: Date(timeIntervalSince1970: TimeInterval(now)), to: Date(timeIntervalSince1970: TimeInterval(end)), size: 11, color: Palette.danger)
                            .frame(maxWidth: 44, alignment: .leading)
                    }
                }
            } else {
                Text(mode.line).font(RFont.fixedSans(11)).foregroundStyle(Palette.ink2).lineLimit(2)
            }
            if medium {
                Spacer(minLength: 0)
                Text(snapshot.activeGrants == 1 ? "1 active grant" : "\(snapshot.activeGrants) active grants")
                    .font(RFont.fixedSans(11, .medium))
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
            // Words only: stopping a bypass carries no icon anywhere.
            GlanceButtonLabel(title: "Stop bypass", foreground: Palette.onField, background: Palette.fieldDelete)
        }
        .buttonStyle(.plain)
    }

    private var lockdownButton: some View {
        let on = mode == .lockdown
        return Button(intent: SetLockdownIntent(value: !on)) {
            // Lockdown's own colour only while it is on.
            GlanceButtonLabel(
                title: on ? "End lockdown" : "Lock down",
                foreground: on ? Palette.onField : Palette.onInverse,
                background: on ? Palette.fieldWarn : Palette.inverse
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
            GlyphIcon(entry.approved ? .check : .close, size: 14)
                .foregroundStyle(entry.approved ? Palette.success : Palette.danger)
                .widgetAccentable()
            Text(entry.title).font(RFont.fixedSans(13, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
            Spacer(minLength: 6)
            Text(Glance.ago(entry.at, now: now)).font(RFont.fixedMono(11)).foregroundStyle(Palette.inkMeta)
        }
        .accessibilityElement(children: .combine)
    }
}

struct LatestActivityView: View {
    var snapshot: Snapshot
    var now: Int64
    var rows: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            GlanceLabel(text: "Latest", color: Palette.inkMeta)
            if !snapshot.signedIn {
                GlanceEmpty.signedOut()
            } else if snapshot.latest.isEmpty {
                GlanceEmpty(title: "Nothing happened yet", message: "What your AIs did shows up here.")
            } else {
                Spacer(minLength: 6)
                VStack(alignment: .leading, spacing: rows > 3 ? 12 : 9) {
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
            Gauge(value: Double(n), in: 0...Double(max(5, n))) {
                Image(systemName: "tray.full")
            } currentValueLabel: {
                Text("\(n)").font(RFont.fixedMono(18, .semibold))
            }
            .gaugeStyle(.accessoryCircularCapacity)
            .widgetAccentable()
            .accessibilityLabel(Glance.countLine(n))
        } else {
            ZStack {
                AccessoryWidgetBackground()
                Image(systemName: "person.crop.circle.badge.questionmark").font(.system(size: 22))
            }
            .accessibilityLabel("Sign in to Reins")
        }
    }
}

/// The newest request: what, who, time left.
struct WaitingRectangularView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        VStack(alignment: .leading, spacing: 1) {
            HStack(spacing: 4) {
                Image(systemName: "tray.full.fill").font(.system(size: 11, weight: .semibold))
                Text(snapshot.signedIn ? Glance.countLine(list.count) : "Reins").font(RFont.fixedSans(13, .semibold)).lineLimit(1)
                Spacer(minLength: 2)
                if let newest = list.first {
                    Countdown(newest, size: 12, color: .primary).frame(maxWidth: 48, alignment: .trailing)
                }
            }
            .widgetAccentable()
            if !snapshot.signedIn {
                Text("Open the app to sign in").font(RFont.fixedSans(13)).foregroundStyle(.secondary).lineLimit(2)
            } else if let newest = list.first {
                Text(Glance.operation(newest)).font(RFont.fixedSans(13, .medium)).lineLimit(1)
                Text(Glance.byline(newest)).font(RFont.fixedSans(12)).foregroundStyle(.secondary).lineLimit(1)
            } else {
                Text("Requests from your AIs show up here").font(RFont.fixedSans(12)).foregroundStyle(.secondary).lineLimit(2)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct WaitingInlineView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        if !snapshot.signedIn {
            Label("Sign in to Reins", systemImage: "tray")
        } else if let newest = list.first, newest.kind != .pairing, !newest.connection.isEmpty {
            Label("\(Glance.countLine(list.count)) · \(newest.connection)", systemImage: "tray.full.fill")
        } else {
            Label(Glance.countLine(list.count), systemImage: list.isEmpty ? "tray" : "tray.full.fill")
        }
    }
}

// MARK: Live Activity: requests waiting

/// The field leaves the app: a waiting request is a Live Activity in its own field's colour, grain and all, with the
/// ask in the serif. One no longer waiting is plain paper. The Dynamic Island stays black; the field arrives there as
/// a dot, the timer and the Review button.
struct ApprovalLiveViews {
    var state: ApprovalActivityAttributes.ContentState
    var stale: Bool

    var link: URL { DeepLink.item(kind: state.kind, id: state.itemId).url }

    /// The field Reins picked from the core's kind (never from the ask's words).
    var field: FieldKind { Glance.field(state.field, kind: state.kind) }

    var label: String {
        if let label = state.label, !label.isEmpty { return label }
        return Glance.label(Snapshot.Item(
            id: state.itemId, kind: state.kind, title: state.title, subtitle: state.subtitle, connection: state.connection,
            service: state.service, createdAt: 0, expiresAt: 0, suggestion: nil
        ))
    }

    /// "Push", the label's last word, for the island's small space.
    var verb: String { label.components(separatedBy: " · ").last ?? label }

    var byline: String {
        [state.kind == .pairing || state.kind == .join ? "" : state.connection, state.suggestion ?? state.subtitle]
            .filter { !$0.isEmpty }
            .joined(separator: " · ")
    }

    var title: String { stale ? "No longer waiting" : state.title }

    var more: String? { state.count > 1 ? "+\(state.count - 1) more waiting" : nil }

    /// Deny (requests only: pairings and files are answered in the app), Approve for a routine request (the same
    /// one-tap answer as the notification's), and Review. On the field, or on the island's black.
    @ViewBuilder func buttons(island: Bool) -> some View {
        let quick = state.kind == .request && state.quick == true && !stale
        let wash = island ? Color.white.opacity(0.16) : Palette.onFieldWash
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
        let ink2: Color = stale ? Palette.ink2 : Palette.onField2
        return VStack(alignment: .leading, spacing: 6) {
            HStack {
                GlanceLabel(text: label, color: ink2)
                Spacer(minLength: 6)
                if !stale {
                    Countdown(from: state.createdAt, to: state.expiresAt, size: 13, color: ink)
                        .frame(maxWidth: 60, alignment: .trailing)
                }
            }
            Text(title)
                .font(RFont.fixedSerif(stale ? 20 : 25))
                .foregroundStyle(ink)
                .lineLimit(2)
                .minimumScaleFactor(0.8)
            if !byline.isEmpty {
                Text(byline).font(RFont.fixedSans(12.5)).foregroundStyle(ink2).lineLimit(1)
            }
            if let more, !stale {
                Text(more).font(RFont.fixedSans(12, .medium)).foregroundStyle(ink)
            }
            if !stale { buttons(island: false).padding(.top, 6) }
        }
        .padding(16)
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
        Countdown(from: state.createdAt, to: state.expiresAt, size: 13, color: field.ink)
            .environment(\.colorScheme, .dark)
            .frame(maxWidth: 46)
    }

    var minimal: some View {
        dot(12).accessibilityLabel(Glance.countLine(state.count))
    }

    var expandedLeading: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(state.kind == .pairing || state.kind == .join ? label : state.connection)
                .font(RFont.fixedSans(12, .medium))
                .foregroundStyle(.white.opacity(0.7))
                .lineLimit(1)
            GlanceLabel(text: verb, color: field.ink)
                .environment(\.colorScheme, .dark)
        }
        .padding(.leading, 6)
        .padding(.top, 4)
    }

    var expandedTrailing: some View {
        Countdown(from: state.createdAt, to: state.expiresAt, size: 15, color: field.ink)
            .environment(\.colorScheme, .dark)
            .frame(maxWidth: 70, alignment: .trailing)
            .padding(.trailing, 6)
            .padding(.top, 4)
    }

    var expandedCenter: some View { EmptyView() }

    var expandedBottom: some View {
        VStack(alignment: .leading, spacing: 8) {
            // The same sentence as everywhere else, in the serif.
            Text(title).font(RFont.fixedSerif(21)).foregroundStyle(.white).lineLimit(2)
            if let more { Text(more).font(RFont.fixedSans(12)).foregroundStyle(.white.opacity(0.65)) }
            buttons(island: true)
        }
        .padding(.horizontal, 6)
        .padding(.top, 2)
    }
}

// MARK: Live Activity: bypass

/// A running bypass: a red field with the time left in the serif and Stop; an Overnight the same in its violet. One
/// that runs until turned off says so instead of counting down. Ended, it is paper.
struct BypassLiveViews {
    var state: BypassActivityAttributes.ContentState
    var stale: Bool

    var untilOff: Bool { state.untilOff ?? false }

    var overnight: Bool { state.overnight ?? false }

    var ended: Bool { stale || (!untilOff && state.until <= Date()) }

    var name: String { overnight ? AutopilotText.overnightName : "Bypass" }

    var field: FieldKind { overnight ? .grant : .delete }

    /// The colour the system tints its own controls with.
    var tint: Color { field.fill }

    var title: String {
        if ended { return "\(name) ended" }
        return "\(name) on · \(state.scope)"
    }

    var line: String { overnight ? AutopilotText.overnightText(scope: state.scope) : AutopilotText.bypassStillAsks }

    var approved: String { state.approvedCount == 0 ? "Nothing approved yet" : "\(state.approvedCount) approved so far" }

    @ViewBuilder func timer(size: CGFloat, color: Color) -> some View {
        if untilOff {
            Text("Until you stop it")
                .font(RFont.fixedSerif(size * 0.6))
                .foregroundStyle(color)
                .lineLimit(1)
        } else {
            HStack(alignment: .lastTextBaseline, spacing: 6) {
                // A timer text takes the widest width it could ever need: give it the one it needs now, so "left"
                // sits beside it ("47:12", or "1:47:12" with hours).
                Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
                    .font(RFont.fixedSerif(size))
                    .monospacedDigit()
                    .foregroundStyle(color)
                    .lineLimit(1)
                    .frame(width: size * (state.until.timeIntervalSinceNow >= 3_600 ? 3.3 : 2.25), alignment: .leading)
                Text("left").font(RFont.fixedSerif(size * 0.5, italic: true)).foregroundStyle(color)
            }
        }
    }

    @ViewBuilder func stop(island: Bool) -> some View {
        let foreground = island ? Color.white : field.fill
        let background = island ? field.fill : Palette.onField
        Group {
            if overnight {
                Button(intent: StopOvernightIntent()) {
                    GlanceButtonLabel(title: "Stop", foreground: foreground, background: background)
                }
            } else {
                Button(intent: StopBypassIntent()) {
                    GlanceButtonLabel(title: "Stop", foreground: foreground, background: background)
                }
            }
        }
        .buttonStyle(.plain)
        .frame(width: 96)
    }

    var lockScreen: some View {
        let ink: Color = ended ? Palette.ink : Palette.onField
        let ink2: Color = ended ? Palette.ink2 : Palette.onField2
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top) {
                GlanceLabel(text: title, color: ink2)
                Spacer(minLength: 6)
                if !ended { stop(island: false) }
            }
            if !ended { timer(size: 42, color: ink) }
            Text(ended ? line : "\(approved) · \(line)")
                .font(RFont.fixedSans(12))
                .foregroundStyle(ink2)
                .lineLimit(2)
        }
        .padding(16)
        .background { if !ended { WidgetGrain() } }
    }

    var background: Color { ended ? Palette.paper : field.fill }

    func dot(_ size: CGFloat) -> some View {
        Circle()
            .fill(field.fill)
            .frame(width: size, height: size)
            .shadow(color: field.fill.opacity(0.9), radius: size * 0.45)
            .accessibilityHidden(true)
    }

    var compactLeading: some View { dot(10).padding(.leading, 4) }

    @ViewBuilder var compactTrailing: some View {
        if untilOff {
            Text("on").font(RFont.fixedMono(13, .semibold)).foregroundStyle(field.ink).environment(\.colorScheme, .dark)
        } else {
            Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
                .font(RFont.fixedMono(13, .semibold))
                .monospacedDigit()
                .foregroundStyle(field.ink)
                .environment(\.colorScheme, .dark)
                .frame(maxWidth: 46)
        }
    }

    var minimal: some View { dot(12).accessibilityLabel(untilOff && !ended ? "\(title), until you stop it" : title) }

    var expandedLeading: some View {
        GlanceLabel(text: name, color: field.ink)
            .environment(\.colorScheme, .dark)
            .padding(.leading, 6)
            .padding(.top, 6)
    }

    var expandedTrailing: some View { EmptyView() }

    var expandedCenter: some View { EmptyView() }

    var expandedBottom: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center) {
                timer(size: 30, color: .white)
                Spacer(minLength: 8)
                stop(island: true)
            }
            Text("For \(state.scope) · \(approved.lowercased())").font(RFont.fixedSans(12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .padding(.horizontal, 6)
    }
}

// MARK: Live Activity: model download

/// Autopilot's model coming down: nothing to decide, so paper.
struct ModelDownloadLiveViews {
    var state: ModelDownloadActivityAttributes.ContentState

    var title: String {
        if state.failed { return "The model did not download" }
        if state.finished { return "Autopilot's model is ready" }
        return "Downloading Autopilot's model"
    }

    var glyph: Glyph { state.failed ? .warning : state.finished ? .check : .download }

    var tint: Color { state.failed ? Palette.warning : Palette.violet }

    var percent: String { "\(Int((Glance.downloadFraction(state) * 100).rounded()))%" }

    var badge: some View {
        GlyphIcon(glyph, size: 18)
            .foregroundStyle(tint)
            .frame(width: 40, height: 40)
            .background(Circle().fill(Palette.paperSunken))
            .accessibilityHidden(true)
    }

    var bar: some View {
        ProgressView(value: Glance.downloadFraction(state)).progressViewStyle(.linear).tint(tint)
    }

    var lockScreen: some View {
        HStack(spacing: 12) {
            badge
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                    Spacer(minLength: 4)
                    if !state.finished && !state.failed {
                        Text(percent).font(RFont.fixedMono(13, .semibold)).foregroundStyle(Palette.ink2)
                    }
                }
                if !state.finished && !state.failed { bar }
                Text(Glance.downloadLine(state)).font(RFont.fixedSans(12)).foregroundStyle(Palette.ink2).lineLimit(1)
            }
        }
        .padding(16)
    }

    var compactLeading: some View {
        GlyphIcon(glyph, size: 15).foregroundStyle(tint).environment(\.colorScheme, .dark)
    }

    var compactTrailing: some View {
        Text(state.finished ? "Ready" : percent).font(RFont.fixedMono(13, .semibold)).foregroundStyle(.white)
    }

    /// A drawn ring: the app updates the activity on every whole percent.
    var minimal: some View {
        ZStack {
            Circle().stroke(Color.white.opacity(0.25), lineWidth: 3)
            Circle().trim(from: 0, to: Glance.downloadFraction(state)).stroke(Color.white, style: StrokeStyle(lineWidth: 3, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .padding(1.5)
        .accessibilityLabel(title)
    }

    var expandedLeading: some View { badge.environment(\.colorScheme, .dark).padding(.leading, 4).padding(.top, 4) }

    var expandedTrailing: some View {
        Text(state.finished ? "" : percent).font(RFont.fixedMono(17, .semibold)).foregroundStyle(.white).padding(.trailing, 6).padding(.top, 12)
    }

    var expandedCenter: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(.white).lineLimit(1)
            Text(Glance.downloadLine(state)).font(RFont.fixedSans(12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    @ViewBuilder var expandedBottom: some View {
        if !state.finished && !state.failed { bar.padding(.top, 8) }
    }
}
#endif
