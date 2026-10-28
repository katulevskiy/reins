import SwiftUI

// The email send hold in Activity (Decisions v2, 08 R10): an approved or Auto-approved Gmail send waits out its hold
// ("Sending in 23 s" · Undo) before the phone sends it; one the phone woke too late for asks "Still send?" (Send,
// Discard). The core keeps the hold on its Activity entry (`ActivityEntry.hold`) only while there is something to do.

/// What a held send offers, per its state.
enum HoldAction: CaseIterable {
    /// Waiting: it is not sent.
    case undo
    /// "Still send?": it goes now, as the owner's approval (the owner's check first).
    case send
    /// "Still send?": it is dropped.
    case discard

    var title: String {
        switch self {
        case .undo: "Undo"
        case .send: "Send"
        case .discard: "Discard"
        }
    }

    var symbol: String {
        switch self {
        case .undo: "arrow.uturn.backward"
        case .send: "paperplane"
        case .discard: "xmark"
        }
    }

    /// The accessibility identifier's prefix ("holdUndo:<hold id>").
    var identifier: String {
        switch self {
        case .undo: "holdUndo"
        case .send: "holdSend"
        case .discard: "holdDiscard"
        }
    }
}

/// A held send in words (pure, unit-tested).
enum HoldText {
    /// "Sending in 23 s" while it waits; "Sending" once its time has come.
    static func countdown(releaseAt: Int64, now: Int64) -> String {
        let left = releaseAt - now
        return left > 0 ? "Sending in \(left) s" : "Sending"
    }

    /// The line under the send: the countdown, "Sending", or the ask.
    static func status(_ hold: ActivityHold, now: Int64) -> String {
        switch hold.state {
        case .waiting: countdown(releaseAt: hold.releaseAt, now: now)
        case .sending: "Sending"
        case .late: lateTitle
        }
    }

    /// The buttons: Undo while it waits; nothing while it goes; Send and Discard on "Still send?".
    static func actions(_ state: HoldState) -> [HoldAction] {
        switch state {
        case .waiting: [.undo]
        case .sending: []
        case .late: [.send, .discard]
        }
    }

    /// Its time has come on this phone's clock: the app releases it (the core never sends one earlier).
    static func due(_ hold: ActivityHold, now: Int64) -> Bool { hold.state == .waiting && hold.releaseAt <= now }

    /// The entries with a hold to show, newest first.
    static func held(_ entries: [ActivityEntry]) -> [ActivityEntry] {
        entries.filter { $0.hold != nil }.sorted { ($0.at, $0.id) > ($1.at, $1.id) }
    }

    /// The outcome word of a held send's entries; nil for any other outcome. "held" never reads as sent: "Sending"
    /// while its hold runs, "Held" once the send has its own entry (sent, undone, not sent).
    static func outcome(_ outcome: String, hold: ActivityHold?) -> String? {
        switch outcome {
        case "held": hold == nil ? "Held" : "Sending"
        case "undone": "Undone"
        case "not_sent": "Not sent"
        default: nil
        }
    }

    static let lateTitle = "Still send?"

    /// The local notification when a wake found sends that came due too long ago (an ask, so it may notify).
    static func lateBody(_ count: UInt32) -> String {
        count == 1 ? "An email was not sent in time." : "\(count) emails were not sent in time."
    }

    // MARK: The setting

    static let settingTitle = "Send hold"
    static let settingHelp = "Approved emails wait this long before they go, so you can undo them. If your phone wakes up more than 15 minutes late, it asks you first."
    static let offered: [UInt32] = [0, 10, 30, 60, 120]

    /// The picker's choices: the offered ones, and one set elsewhere.
    static func choices(_ selected: UInt32) -> [UInt32] {
        offered.contains(selected) ? offered : (offered + [selected]).sorted()
    }

    /// "Off", "30 s".
    static func choiceLabel(_ secs: UInt32) -> String { secs == 0 ? "Off" : "\(secs) s" }
}

/// What the held sends on screen need now: a release (a hold came due) or a re-read (one is going).
struct HoldWork: Equatable {
    var due: [String]
    var sending: [String]

    static func of(_ entries: [ActivityEntry], now: Int64) -> HoldWork {
        let holds = entries.compactMap(\.hold)
        return HoldWork(due: holds.filter { HoldText.due($0, now: now) }.map(\.id), sending: holds.filter { $0.state == .sending }.map(\.id))
    }

    var idle: Bool { due.isEmpty && sending.isEmpty }
}

/// The held sends, above the river: each with its countdown and Undo, or "Still send?" with Send and Discard. When a
/// countdown reaches 0 the app releases it (`SendHolds.release`) and reads Activity again.
struct HeldSends: View {
    var entries: [ActivityEntry]
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let now = Int64(Timers.now(context.date))
            VStack(spacing: 6) {
                ForEach(entries, id: \.id) { entry in
                    if let hold = entry.hold { HeldSendRow(entry: entry, hold: hold, now: now) }
                }
            }
            .padding(.horizontal, 16)
            .task(id: HoldWork.of(entries, now: now)) { await work(HoldWork.of(entries, now: now)) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("heldSends")
    }

    /// Releases what came due (again a little later if it could not), or reads again while one is going.
    private func work(_ work: HoldWork) async {
        guard !work.idle, Timers.live else { return }
        while !Task.isCancelled, model.screensMayReadStore {
            if work.due.isEmpty {
                await model.refreshPending()
            } else {
                _ = try? await SendHolds.release(model, notifier: nil)
            }
            try? await Task.sleep(for: .seconds(3))
        }
    }
}

/// One held send: the request's icon, what it sends, the countdown or the ask, and its buttons.
private struct HeldSendRow: View {
    var entry: ActivityEntry
    var hold: ActivityHold
    var now: Int64
    @Environment(AppModel.self) private var model
    @State private var busy = false

    var body: some View {
        let late = hold.state == .late
        // One row when what it sends, its status and the buttons all fit; else what it sends keeps the width and the
        // buttons go under it. Send is never beside an email cut to "Send…".
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 10) {
                icon
                words(titleLines: 1)
                Spacer(minLength: 4)
                buttons
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .top, spacing: 10) {
                    icon
                    words(titleLines: 2).frame(maxWidth: .infinity, alignment: .leading)
                }
                HStack(spacing: 8) {
                    Spacer(minLength: 0)
                    buttons
                }
            }
        }
        .padding(.leading, 10)
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).stroke(late ? Palette.warning.opacity(0.5) : Palette.rule, lineWidth: 1))
        .disabled(busy)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hold:\(hold.id)")
    }

    private var icon: some View {
        RequestIcon(spec: EntryLook.line(entry).icon, size: 30, ring: Palette.paperRaised)
            .accessibilityHidden(true)
    }

    /// What it sends, then its countdown or "Still send?".
    private func words(titleLines: Int) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(verbatim: entry.label.map(RequestLabelLook.plain) ?? EntryText.title(entry))
                .font(RFont.sans(13.5, .semibold))
                .foregroundStyle(Palette.ink)
                .lineLimit(titleLines)
            Text(HoldText.status(hold, now: now))
                .font(RFont.sans(12.5, .medium))
                .monospacedDigit()
                .foregroundStyle(hold.state == .late ? Palette.warning : Palette.inkMeta)
                .lineLimit(1)
                .accessibilityIdentifier("holdStatus:\(hold.id)")
        }
    }

    @ViewBuilder private var buttons: some View {
        if hold.state == .sending {
            ProgressView().controlSize(.small)
        }
        ForEach(HoldText.actions(hold.state), id: \.self) { action in
            Button { run(action) } label: {
                Label(action.title, systemImage: action.symbol)
            }
            .buttonStyle(SignalButtonStyle(action == .send ? .inverse : .outline, size: .small, fill: false))
            .fixedSize()
            .accessibilityIdentifier("\(action.identifier):\(hold.id)")
        }
    }

    private func run(_ action: HoldAction) {
        busy = true
        Task {
            if let message = await SendHolds.perform(action, holdId: hold.id, model: model) { model.notice = message }
            busy = false
        }
    }
}
