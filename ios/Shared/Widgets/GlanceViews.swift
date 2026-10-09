#if REINS_APP || REINS_WIDGETS
import AppIntents
import SwiftUI
import WidgetKit

// The views of every widget and Live Activity. They live in Shared so the app's debug gallery (`-widgetGallery`)
// draws exactly what the widget extension draws. Fixed type sizes (widgets do not follow Dynamic Type the way
// screens do), the Reins palette, and in the accented and vibrant rendering modes no colour that carries meaning.

enum GlanceStyle {
    /// The widget plate: white in light mode, true black in dark.
    static let background = Palette.dynamic(0xFFFFFF, 0x060606)
    /// Behind a Live Activity on the Lock Screen.
    static let liveBackground = Palette.dynamic(0xFFFFFF, 0x060606, alpha: 0.94)

    static func tint(_ mode: Glance.Mode) -> Color {
        switch mode {
        case .manual: Palette.secondary
        case .assisted: Palette.search
        case .auto: Palette.accent
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
            .foregroundStyle(full ? Palette.text : Color.primary)
            .frame(width: size * 0.56, height: size * 0.56)
            .frame(width: size, height: size)
            .background(full ? Palette.dynamic(0xFFFFFF, 0x1D1D21) : Color.primary.opacity(0.16), in: Circle())
            .overlay(Circle().strokeBorder(full ? Palette.hairline : .clear, lineWidth: 1))
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
        .overlay(Circle().strokeBorder(Palette.hairline, lineWidth: 1))
        .clipShape(Circle())
        .accessibilityHidden(true)
    }

    private var initial: some View {
        let full = renderingMode == .fullColor
        return Circle()
            .fill(full ? Palette.accentSoft : Color.primary.opacity(0.16))
            .overlay {
                Group {
                    if kind == .pairing || kind == .join || Glance.initial(label).isEmpty {
                        Image(systemName: kind == .pairing ? "link" : kind == .join ? "iphone.gen3" : "key.fill")
                            .font(.system(size: size * 0.4, weight: .semibold))
                    } else {
                        Text(Glance.initial(label)).font(RFont.fixedSans(size * 0.44, .semibold))
                    }
                }
                .foregroundStyle(full ? Palette.accent : Color.primary)
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
    var color: Color = Palette.secondary

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
    init(_ item: Snapshot.Item, size: CGFloat = 12, color: Color = Palette.secondary) {
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
    var tint: Color = Palette.accent

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

/// The capsule on widget and Live Activity buttons.
struct GlanceButtonLabel: View {
    var title: String
    var symbol: String?
    var tint: Color
    var filled = false
    var height: CGFloat = 34

    var body: some View {
        HStack(spacing: 5) {
            if let symbol { Image(systemName: symbol).font(.system(size: 12, weight: .semibold)) }
            Text(title).font(RFont.fixedSans(13, .semibold)).lineLimit(1)
        }
        .foregroundStyle(filled ? Color.white : tint)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, minHeight: height, maxHeight: height)
        .background(Capsule().fill(filled ? tint : tint.opacity(0.15)))
        .contentShape(Capsule())
    }
}

/// A mode's glyph in its colour.
struct ModeBadge: View {
    var mode: Glance.Mode
    var size: CGFloat = 30
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        let tint = renderingMode == .fullColor ? GlanceStyle.tint(mode) : Color.primary
        RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
            .fill(tint.opacity(0.16))
            .overlay(Image(systemName: mode.symbol).font(.system(size: size * 0.45, weight: .semibold)).foregroundStyle(tint))
            .widgetAccentable()
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// The small title line at the top of a widget.
struct GlanceHeader: View {
    var title: String
    var count: Int?
    var symbol: String

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).font(.system(size: 11, weight: .semibold)).foregroundStyle(Palette.accent).widgetAccentable()
            Text(title).font(RFont.fixedSans(13, .semibold)).foregroundStyle(Palette.text)
            if let count {
                Text("\(count)").font(RFont.fixedMono(13, .semibold)).foregroundStyle(Palette.accent).widgetAccentable()
            }
            Spacer(minLength: 0)
        }
    }
}

/// Nothing to show: signed out, nothing waits, nothing happened.
struct GlanceEmpty: View {
    var symbol: String
    var title: String
    var message: String
    var compact = false

    var body: some View {
        VStack(alignment: compact ? .leading : .center, spacing: 4) {
            Image(systemName: symbol)
                .font(.system(size: compact ? 20 : 24, weight: .medium))
                .foregroundStyle(Palette.accent)
                .widgetAccentable()
                .padding(.bottom, 4)
            Text(title).font(RFont.fixedSans(14, .semibold)).foregroundStyle(Palette.text)
            Text(message)
                .font(RFont.fixedSans(11))
                .foregroundStyle(Palette.secondary)
                .multilineTextAlignment(compact ? .leading : .center)
                .lineLimit(3)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: compact ? .bottomLeading : .center)
    }
}

extension GlanceEmpty {
    static func signedOut(compact: Bool = false) -> GlanceEmpty {
        GlanceEmpty(symbol: "person.crop.circle.badge.questionmark", title: "Sign in to Reins", message: "Open the app to sign in.", compact: compact)
    }

    static func nothingWaits(_ s: Snapshot, compact: Bool = false) -> GlanceEmpty {
        GlanceEmpty(
            symbol: "checkmark.circle",
            title: "Nothing waits",
            message: s.approvalDevice || !s.signedIn ? "Requests from your AIs show up here." : "Requests go to your other phone.",
            compact: compact
        )
    }
}

// MARK: Waiting

/// One waiting item in a list: avatar, operation, who and for which account, time left.
struct WaitingRow: View {
    var item: Snapshot.Item

    var body: some View {
        HStack(spacing: 10) {
            GlanceAvatar(label: item.connection, kind: item.kind, icon: item.connectionIcon, size: 28)
            VStack(alignment: .leading, spacing: 1) {
                Text(Glance.operation(item)).font(RFont.fixedSans(13, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                let byline = Glance.byline(item)
                if !byline.isEmpty {
                    Text(byline).font(RFont.fixedSans(11)).foregroundStyle(Palette.secondary).lineLimit(1)
                }
            }
            Spacer(minLength: 6)
            Countdown(item)
                .frame(maxWidth: 56, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
    }
}

/// "Waiting", small: how many, and the newest with its countdown.
struct WaitingSmallView: View {
    var snapshot: Snapshot
    var now: Int64

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        if !snapshot.signedIn {
            GlanceEmpty.signedOut(compact: true)
        } else if let newest = list.first {
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .top) {
                    Text("\(list.count)")
                        .font(RFont.fixedMono(36, .semibold))
                        .foregroundStyle(Palette.accent)
                        .widgetAccentable()
                    Spacer(minLength: 4)
                    GlanceAvatar(label: newest.connection, kind: newest.kind, icon: newest.connectionIcon, size: 30)
                }
                Text(list.count == 1 ? "waits for you" : "wait for you")
                    .font(RFont.fixedSans(12, .medium))
                    .foregroundStyle(Palette.secondary)
                Spacer(minLength: 6)
                Text(Glance.operation(newest))
                    .font(RFont.fixedSans(14, .semibold))
                    .foregroundStyle(Palette.text)
                    .lineLimit(2)
                HStack(spacing: 4) {
                    Text(newest.kind == .pairing ? "New connection" : newest.kind == .join ? "Another phone" : newest.connection)
                        .font(RFont.fixedSans(11))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(1)
                    Spacer(minLength: 2)
                    Countdown(newest, size: 11, color: Palette.accent)
                        .frame(maxWidth: 52, alignment: .trailing)
                }
                .padding(.top, 2)
            }
            .accessibilityElement(children: .combine)
        } else {
            GlanceEmpty.nothingWaits(snapshot, compact: true)
        }
    }
}

/// "Waiting", medium and large: the list, each row opening its item.
struct WaitingListView: View {
    var snapshot: Snapshot
    var now: Int64
    var rows: Int

    var body: some View {
        let list = Glance.waiting(snapshot, now: now)
        VStack(alignment: .leading, spacing: 0) {
            GlanceHeader(title: "Waiting", count: list.isEmpty ? nil : list.count, symbol: "tray.full.fill")
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
                        .foregroundStyle(Palette.tertiary)
                        .padding(.top, 4)
                }
            }
        }
    }
}

// MARK: Autopilot

/// "Autopilot": the mode in force, a bypass's countdown, and Lockdown / Stop as buttons.
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
            HStack(alignment: .top) {
                ModeBadge(mode: mode)
                Spacer(minLength: 4)
                Text("Autopilot").font(RFont.fixedSans(11, .medium)).foregroundStyle(Palette.tertiary)
            }
            .padding(.bottom, 6)
            Text(mode.name)
                .font(RFont.fixedSans(medium ? 20 : 18, .semibold))
                .foregroundStyle(mode == .manual ? Palette.text : GlanceStyle.tint(mode))
                .widgetAccentable()
            if let end = bypassEnd {
                HStack(spacing: 4) {
                    Text(Glance.connectionBypassOnly(snapshot, now: now) ? "Bypass for some AIs" : "Ends in")
                        .font(RFont.fixedSans(11))
                        .foregroundStyle(Palette.secondary)
                        .lineLimit(1)
                    Countdown(from: Date(timeIntervalSince1970: TimeInterval(now)), to: Date(timeIntervalSince1970: TimeInterval(end)), size: 11, color: Palette.danger)
                        .frame(maxWidth: 44, alignment: .leading)
                }
            } else {
                Text(mode.line).font(RFont.fixedSans(11)).foregroundStyle(Palette.secondary).lineLimit(2)
            }
            if medium {
                Spacer(minLength: 0)
                Text(snapshot.activeGrants == 1 ? "1 active grant" : "\(snapshot.activeGrants) active grants")
                    .font(RFont.fixedSans(11, .medium))
                    .foregroundStyle(Palette.tertiary)
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
            GlanceButtonLabel(title: "Stop bypass", symbol: "stop.fill", tint: Palette.danger, filled: true)
        }
        .buttonStyle(.plain)
    }

    private var lockdownButton: some View {
        let on = mode == .lockdown
        return Button(intent: SetLockdownIntent(value: !on)) {
            GlanceButtonLabel(title: on ? "End lockdown" : "Lock down", symbol: on ? "lock.open.fill" : "lock.fill", tint: Palette.warning, filled: on)
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
            Image(systemName: entry.approved ? "checkmark.circle.fill" : "xmark.circle.fill")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(entry.approved ? Palette.success : Palette.danger)
                .widgetAccentable()
            Text(entry.title).font(RFont.fixedSans(13, .medium)).foregroundStyle(Palette.text).lineLimit(1)
            Spacer(minLength: 6)
            Text(Glance.ago(entry.at, now: now)).font(RFont.fixedMono(11)).foregroundStyle(Palette.tertiary)
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
            GlanceHeader(title: "Latest activity", count: nil, symbol: "clock.fill")
            if !snapshot.signedIn {
                GlanceEmpty.signedOut()
            } else if snapshot.latest.isEmpty {
                GlanceEmpty(symbol: "clock", title: "Nothing happened yet", message: "What your AIs did shows up here.")
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

struct ApprovalLiveViews {
    var state: ApprovalActivityAttributes.ContentState
    var stale: Bool

    var link: URL { DeepLink.item(kind: state.kind, id: state.itemId).url }

    var byline: String {
        [state.kind == .pairing || state.kind == .join ? "" : state.connection, state.subtitle].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    var title: String { stale ? "No longer waiting" : state.title }

    var more: String? { state.count > 1 ? "+\(state.count - 1) more waiting" : nil }

    var avatar: GlanceAvatar { GlanceAvatar(label: state.connection, kind: state.kind, icon: state.connectionIcon, size: 40) }

    var ring: CountdownRing { CountdownRing(from: state.createdAt, to: state.expiresAt, size: 44) }

    /// Deny (requests only: pairings and files are answered in the app), Approve for a routine request (the same
    /// one-tap answer as the notification's), and Review.
    @ViewBuilder var buttons: some View {
        let quick = state.kind == .request && state.quick == true && !stale
        HStack(spacing: 10) {
            if state.kind == .request {
                Button(intent: DenyRequestIntent(requestId: state.itemId)) {
                    GlanceButtonLabel(title: "Deny", symbol: "xmark", tint: Palette.danger)
                }
                .buttonStyle(.plain)
            }
            if quick {
                Button(intent: ApproveQuickIntent(requestId: state.itemId)) {
                    GlanceButtonLabel(title: "Approve", symbol: "checkmark", tint: Palette.success, filled: true)
                }
                .buttonStyle(.plain)
            }
            Link(destination: link) {
                GlanceButtonLabel(title: "Review", symbol: "arrow.up.forward", tint: Palette.accent, filled: !quick)
            }
        }
    }

    var text: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(Palette.text).lineLimit(2)
            if !byline.isEmpty {
                Text(byline).font(RFont.fixedSans(12)).foregroundStyle(Palette.secondary).lineLimit(1)
            }
            if let more {
                Text(more).font(RFont.fixedSans(11, .medium)).foregroundStyle(Palette.accent)
            }
        }
    }

    var lockScreen: some View {
        VStack(spacing: 12) {
            HStack(spacing: 12) {
                avatar
                text
                Spacer(minLength: 4)
                if !stale { ring }
            }
            if !stale { buttons }
        }
        .padding(16)
    }

    // The Dynamic Island.

    var compactLeading: some View { GlanceAvatar(label: state.connection, kind: state.kind, icon: state.connectionIcon, size: 24) }

    var compactTrailing: some View {
        Countdown(from: state.createdAt, to: state.expiresAt, size: 13, color: Palette.accent)
            .frame(maxWidth: 46)
    }

    var minimal: some View {
        ZStack {
            TimerRing(from: state.createdAt, to: state.expiresAt, tint: Palette.accent, lineWidth: 3)
            Text("\(state.count)").font(RFont.fixedMono(11, .semibold)).foregroundStyle(.white)
        }
        .accessibilityLabel(Glance.countLine(state.count))
    }

    var expandedLeading: some View { avatar.padding(.leading, 4).padding(.top, 4) }

    var expandedTrailing: some View { CountdownRing(from: state.createdAt, to: state.expiresAt, size: 40).padding(.trailing, 4).padding(.top, 4) }

    var expandedCenter: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(.white).lineLimit(1)
            Text([byline, more].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                .font(RFont.fixedSans(12))
                .foregroundStyle(.white.opacity(0.65))
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    var expandedBottom: some View { buttons.padding(.top, 6) }
}

// MARK: Live Activity: bypass

/// A running bypass, drawn as the "surge" state: red, the bolt, a countdown and Stop.
struct BypassLiveViews {
    var state: BypassActivityAttributes.ContentState
    var stale: Bool

    var ended: Bool { stale || state.until <= Date() }

    var title: String { ended ? "Bypass ended" : "Bypass on" }

    var line: String { "Requests from \(state.scope) are approved without asking, except the riskiest." }

    var approved: String { state.approvedCount == 0 ? "Nothing approved yet" : "\(state.approvedCount) approved so far" }

    var bolt: some View {
        Circle()
            .fill(Palette.danger.opacity(0.18))
            .overlay(Image(systemName: "bolt.fill").font(.system(size: 18, weight: .bold)).foregroundStyle(Palette.danger))
            .frame(width: 40, height: 40)
            .accessibilityHidden(true)
    }

    var timer: some View {
        Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
            .font(RFont.fixedMono(22, .semibold))
            .monospacedDigit()
            .foregroundStyle(Palette.danger)
            .multilineTextAlignment(.trailing)
            .frame(maxWidth: 90, alignment: .trailing)
    }

    var bar: some View {
        ProgressView(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true) {
            EmptyView()
        } currentValueLabel: {
            EmptyView()
        }
        .progressViewStyle(.linear)
        .tint(Palette.danger)
    }

    var stop: some View {
        Button(intent: StopBypassIntent()) {
            GlanceButtonLabel(title: "Stop", symbol: "stop.fill", tint: Palette.danger, filled: true)
        }
        .buttonStyle(.plain)
        .frame(width: 112)
    }

    var lockScreen: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 12) {
                bolt
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(Palette.text)
                    Text(line).font(RFont.fixedSans(12)).foregroundStyle(Palette.secondary).lineLimit(2)
                }
                Spacer(minLength: 4)
                if !ended { timer }
            }
            if !ended {
                bar
                HStack {
                    Text(approved).font(RFont.fixedSans(12, .medium)).foregroundStyle(Palette.secondary)
                    Spacer()
                    stop
                }
            }
        }
        .padding(16)
    }

    var compactLeading: some View {
        Image(systemName: "bolt.fill").font(.system(size: 14, weight: .bold)).foregroundStyle(Palette.danger)
    }

    var compactTrailing: some View {
        Text(timerInterval: min(state.startedAt, state.until)...state.until, countsDown: true)
            .font(RFont.fixedMono(13, .semibold))
            .monospacedDigit()
            .foregroundStyle(Palette.danger)
            .frame(maxWidth: 46)
    }

    var minimal: some View {
        ZStack {
            TimerRing(from: state.startedAt, to: state.until, tint: Palette.danger, lineWidth: 3)
            Image(systemName: "bolt.fill").font(.system(size: 10, weight: .bold)).foregroundStyle(Palette.danger)
        }
        .accessibilityLabel(title)
    }

    var expandedLeading: some View { bolt.padding(.leading, 4).padding(.top, 4) }

    var expandedTrailing: some View { timer.padding(.trailing, 4).padding(.top, 8) }

    var expandedCenter: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(.white)
            Text("For \(state.scope) · \(approved.lowercased())").font(RFont.fixedSans(12)).foregroundStyle(.white.opacity(0.65)).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.top, 4)
    }

    var expandedBottom: some View {
        HStack(spacing: 12) {
            bar
            stop
        }
        .padding(.top, 6)
    }
}

// MARK: Live Activity: model download

struct ModelDownloadLiveViews {
    var state: ModelDownloadActivityAttributes.ContentState

    var title: String {
        if state.failed { return "The model did not download" }
        if state.finished { return "Autopilot's model is ready" }
        return "Downloading Autopilot's model"
    }

    var symbol: String { state.failed ? "exclamationmark.triangle.fill" : state.finished ? "checkmark" : "arrow.down" }

    var tint: Color { state.failed ? Palette.warning : Palette.accent }

    var percent: String { "\(Int((Glance.downloadFraction(state) * 100).rounded()))%" }

    var badge: some View {
        Circle()
            .fill(tint.opacity(0.16))
            .overlay(Image(systemName: symbol).font(.system(size: 17, weight: .bold)).foregroundStyle(tint))
            .frame(width: 40, height: 40)
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
                    Text(title).font(RFont.fixedSans(15, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                    Spacer(minLength: 4)
                    if !state.finished && !state.failed {
                        Text(percent).font(RFont.fixedMono(13, .semibold)).foregroundStyle(tint)
                    }
                }
                if !state.finished && !state.failed { bar }
                Text(Glance.downloadLine(state)).font(RFont.fixedSans(12)).foregroundStyle(Palette.secondary).lineLimit(1)
            }
        }
        .padding(16)
    }

    var compactLeading: some View {
        Image(systemName: state.finished ? "checkmark.circle.fill" : "arrow.down.circle.fill")
            .font(.system(size: 15, weight: .semibold))
            .foregroundStyle(tint)
    }

    var compactTrailing: some View {
        Text(state.finished ? "Ready" : percent).font(RFont.fixedMono(13, .semibold)).foregroundStyle(tint)
    }

    /// A drawn ring: the app updates the activity on every whole percent.
    var minimal: some View {
        ZStack {
            Circle().stroke(tint.opacity(0.25), lineWidth: 3)
            Circle().trim(from: 0, to: Glance.downloadFraction(state)).stroke(tint, style: StrokeStyle(lineWidth: 3, lineCap: .round)).rotationEffect(.degrees(-90))
        }
        .padding(1.5)
        .accessibilityLabel(title)
    }

    var expandedLeading: some View { badge.padding(.leading, 4).padding(.top, 4) }

    var expandedTrailing: some View {
        Text(state.finished ? "" : percent).font(RFont.fixedMono(17, .semibold)).foregroundStyle(tint).padding(.trailing, 6).padding(.top, 12)
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
