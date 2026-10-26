import SwiftUI

// The modes in the kit (the System board, Modes; the Mode picker and Running boards): Trust's mode card, the
// Manual/Auto switch with its one "For a while" row, a mode's row in the mode sheet, and the field a timed mode turns
// Trust into while it runs. Views only: what a tap does is the caller's.

/// A ring and a clock in mono, white on a field ("◔ 5:59:12"), or ink on paper.
struct RingClock: View {
    var text: String
    var fraction: Double
    var onField = true

    var body: some View {
        HStack(spacing: 6) {
            ArcRing(fraction: fraction, color: onField ? .white : Palette.ink, track: onField ? .white.opacity(0.28) : Palette.rule2, lineWidth: 2.2)
                .frame(width: 18, height: 18)
            Text(text).font(RFont.mono(13.5, .medium)).monospacedDigit().lineLimit(1)
        }
        .foregroundStyle(onField ? .white : Palette.ink)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(text) left")
    }
}

/// The mode now, Trust's first block (the System board, "The card"): calm modes are a raised card; the others are their
/// colour field with grain, a clock when they end on their own, and Stop or End.
struct ModeCard: View {
    var mode: ModeKind
    var clock: (text: String, fraction: Double)?
    var actionTitle: String
    var actionID = "modeCardAction"
    var action: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let field = mode != .manual
        HStack(spacing: 14) {
            RoundedRectangle(cornerRadius: 15, style: .continuous)
                .fill(field ? .white.opacity(0.2) : Palette.paperSunken)
                .frame(width: 48, height: 48)
                .overlay { ModeGlyph(mode, size: 22).foregroundStyle(field ? .white : Palette.ink) }
            VStack(alignment: .leading, spacing: 4) {
                Text(mode.name).textRole(.cardAsk).foregroundStyle(field ? .white : Palette.ink).lineLimit(1)
                Text(mode.effect).font(RFont.sans(13.5)).foregroundStyle(field ? .white.opacity(0.85) : Palette.inkMeta).lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityElement(children: .combine)
            VStack(alignment: .trailing, spacing: 8) {
                if let clock { RingClock(text: clock.text, fraction: clock.fraction) }
                Button(actionTitle, action: action)
                    .buttonStyle(SignalButtonStyle(field ? .onField : .soft, size: .small, fill: false))
                    .accessibilityIdentifier(actionID)
            }
        }
        .padding(16)
        .frame(minHeight: 96)
        .background {
            let shape = RoundedRectangle(cornerRadius: 26, style: .continuous)
            if field {
                ZStack { mode.fill; Grain() }.clipShape(shape)
            } else {
                shape.fill(Palette.paperRaised)
                if scheme == .light { shape.strokeBorder(Palette.rule, lineWidth: 1) }
            }
        }
        .padding(.horizontal, 16)
    }
}

/// Manual or Auto (the Mode picker board): a sunken track, 76 pt; the chosen half is filled like every mode when on,
/// Manual in ink (white in dark), Auto in its violet, both with grain. The thumb slides on a spring.
struct ModeSwitch: View {
    var selected: ModeKind
    var autoReady = true
    var onPick: (ModeKind) -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let auto = selected == .auto
        ZStack {
            Capsule().fill(Palette.paperSunken)
            GeometryReader { geo in
                let w = geo.size.width / 2 - 6
                ZStack { (auto ? Palette.violetFill : Palette.ink); Grain() }
                    .clipShape(Capsule())
                    .shadow(color: auto ? Palette.violetFill.opacity(0.6) : .black.opacity(0.45), radius: 8, y: 6)
                    .frame(width: w, height: geo.size.height - 12)
                    .offset(x: auto ? geo.size.width / 2 : 6, y: 6)
            }
            .opacity(selected.calm ? 1 : 0)
            HStack(spacing: 0) {
                half(.manual, on: selected == .manual)
                half(.auto, on: auto)
            }
        }
        .frame(height: 76)
        .padding(.horizontal, 16)
        .animation(reduceMotion ? .easeInOut(duration: Motion.fade) : .spring(duration: 0.3, bounce: 0.18), value: selected)
    }

    private func half(_ mode: ModeKind, on: Bool) -> some View {
        Button {
            if !on { onPick(mode) }
        } label: {
            HStack(spacing: 10) {
                ModeGlyph(mode, size: 20)
                Text(mode.name)
            }
            .font(RFont.sans(19, .semibold))
            .foregroundStyle(on ? (mode == .auto ? .white : Palette.paper) : Palette.ink2)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
        .disabled(mode == .auto && !autoReady && !on)
        .accessibilityLabel(mode.name)
        .accessibilityAddTraits(on ? .isSelected : [])
        .accessibilityIdentifier("modeSwitch:\(mode.rawValue)")
    }
}

/// The one quiet row that starts a timed mode (the Mode picker board): a timer glyph, "For a while", the three modes
/// as tiny tiles, a chevron. Tapping opens a menu of the three.
struct ForAWhileRow: View {
    var body: some View {
        HStack(spacing: 12) {
            GlyphIcon(.timer, size: 20).foregroundStyle(Palette.ink2)
            Text("For a while").textRole(.row).foregroundStyle(Palette.ink).frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 4) {
                ForEach(ModeKind.timed, id: \.self) { ModeTile(mode: $0) }
            }
            RowChevron()
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 52)
        .contentShape(Rectangle())
    }
}

/// A mode in the mode sheet: its tile in its colour, the name (Bypass in red) and what it does, then a check for the
/// mode in force or the length a timed one starts with.
struct ModeSheetRow: View {
    var mode: ModeKind
    var current: Bool
    var length: String?

    var body: some View {
        HStack(spacing: 12) {
            ModeTile(mode: mode, side: 32)
            VStack(alignment: .leading, spacing: 1) {
                Text(mode.name).textRole(.row).foregroundStyle(mode == .bypass ? Palette.danger : Palette.ink)
                Text(mode.effect).textRole(.sub).foregroundStyle(Palette.inkMeta)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if current {
                GlyphIcon(.check, size: 18).foregroundStyle(Palette.violet)
            } else if let length {
                Text(length).textRole(.meta).foregroundStyle(Palette.inkMeta)
                RowChevron()
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 9)
        .frame(minHeight: 56)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(current ? .isSelected : [])
    }
}

/// What a bypass never allows, as one-word chips on its field: a new AI, a new phone.
struct StillAsks: View {
    var body: some View {
        HStack(spacing: 6) {
            Text("Still asks").font(RFont.sans(13, .semibold)).opacity(0.85)
            Pill("New AI", tone: .onFieldWash, glyph: .link, size: .pill)
            Pill("New phone", tone: .onFieldWash, glyph: .phone, size: .pill)
        }
        .foregroundStyle(.white)
        .accessibilityElement(children: .combine)
    }
}
