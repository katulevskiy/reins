import SwiftUI

// Activity's layout pieces in Next (Next-Activity, Next-Wallpaper, Next-More): a raised group drawn one row at a time
// so the river stays lazy, the day's header (over its group on paper, inside it on a wallpaper), the header's search
// bar, the action slot that proposes the missing half, and a new account's first screen.

/// One row's share of a raised group (the kit's `grp`, radius 20, 16 pt in from the sides): its corners round only at
/// the group's ends, the hairline above it starts after the icon (62) and, on glass, stops short of the right edge.
/// On a wallpaper the group is a frosted pane (solid under Reduce Transparency). Each row draws its own share, so a
/// long day stays in a lazy stack.
struct GroupSlice: ViewModifier {
    var first: Bool
    var last: Bool
    /// A hairline above this row.
    var hairline: Bool
    var inset: CGFloat = 62
    @Environment(\.colorScheme) private var scheme
    @Environment(\.onWallpaper) private var onWallpaper
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    func body(content: Content) -> some View {
        let r: CGFloat = 20
        let shape = UnevenRoundedRectangle(
            topLeadingRadius: first ? r : 0, bottomLeadingRadius: last ? r : 0, bottomTrailingRadius: last ? r : 0,
            topTrailingRadius: first ? r : 0, style: .continuous
        )
        let glass = onWallpaper && !reduceTransparency
        let dark = scheme == .dark
        content
            .frame(maxWidth: .infinity, alignment: .leading)
            .overlay(alignment: .top) {
                if hairline {
                    Rectangle()
                        .fill(glass ? (dark ? Color.white.opacity(0.09) : Color.black.opacity(0.08)) : Palette.rule)
                        .frame(height: glass ? 0.5 : 1)
                        .padding(.leading, inset)
                        .padding(.trailing, glass ? 16 : 0)
                }
            }
            .background {
                if glass {
                    // The shared glass: every slice draws the same blurred art at its place on screen, so the rows of a
                    // day read as one pane with no seam between them.
                    FrostedGlass()
                } else {
                    Palette.paperRaised
                }
            }
            .overlay {
                // The ring of the whole group: drawn past this row's cut edges, so only its own part shows.
                let ring: Color = glass ? Color.white.opacity(dark ? 0.08 : 0.35) : (onWallpaper || !dark ? Palette.rule : .clear)
                RoundedRectangle(cornerRadius: r, style: .continuous)
                    .strokeBorder(ring, lineWidth: 1)
                    .overlay(alignment: .top) {
                        if glass && first {
                            // The lit top edge of the pane.
                            RoundedRectangle(cornerRadius: r, style: .continuous)
                                .strokeBorder(
                                    LinearGradient(colors: [Color.white.opacity(dark ? 0.16 : 0.8), .clear], startPoint: .top, endPoint: .init(x: 0.5, y: 0.25)),
                                    lineWidth: 1
                                )
                        }
                    }
                    .padding(.top, first ? 0 : -40)
                    .padding(.bottom, last ? 0 : -40)
                    .allowsHitTesting(false)
            }
            .clipShape(shape)
            .padding(.horizontal, 16)
    }
}

extension View {
    func groupSlice(first: Bool, last: Bool, hairline: Bool) -> some View {
        modifier(GroupSlice(first: first, last: last, hairline: hairline))
    }
}

/// A day of the river (the kit's `sec` with `dayc`): its name, its counts as glyphs (approved, Autopilot, denied, and
/// what needs a look), and a chevron that says it folds. On paper it sits over its group; on a wallpaper it is the
/// group's first row (`inGroup`), so it never sits on the art. A heading that is also its toggle.
struct DayHeader: View {
    var title: String
    var day: ActivityDay
    var open: Bool
    var inGroup = false
    var toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 6) {
                Text(title)
                    .font(RFont.sans(13, .semibold))
                    .tracking(13 * TextRole.section.trackingEm)
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
                Spacer(minLength: 8)
                DayCounts(day: day)
                GlyphIcon(open ? .chevronDown : .chevronRight, size: 14)
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.leading, 2)
            }
            .padding(.leading, inGroup ? 14 : 24)
            .padding(.trailing, inGroup ? 14 : 24)
            .padding(.top, inGroup ? (open ? 12 : 14) : 22)
            .padding(.bottom, inGroup ? (open ? 4 : 14) : 8)
            .frame(minHeight: 44)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(ActivityDays.spoken(title, day))
        .accessibilityValue(open ? "Expanded" : "Collapsed")
        .accessibilityHint(open ? "Folds the day" : "Opens the day")
        .accessibilityAddTraits([.isHeader, .isButton])
        .accessibilityAction { toggle() }
        .accessibilityIdentifier("day:\(day.key)")
    }
}

/// "✓7 ⚡2 ✕1": a day's counts, each its glyph and number in its colour; only those above nought.
struct DayCounts: View {
    var day: ActivityDay

    var body: some View {
        HStack(spacing: 10) {
            count(day.approved, glyph: .check, tint: Palette.success)
            count(day.auto, glyph: .autopilot, tint: Palette.violet)
            count(day.denied, glyph: .close, tint: Palette.danger)
            count(day.failed, glyph: .circleAlert, tint: Palette.warning)
        }
        .font(RFont.sans(13, .semibold))
        .monospacedDigit()
        .lineLimit(1)
        .fixedSize()
    }

    @ViewBuilder private func count(_ n: Int, glyph: Glyph, tint: Color) -> some View {
        if n > 0 {
            HStack(spacing: 3) {
                GlyphIcon(glyph, size: 13)
                Text("\(n)")
            }
            .foregroundStyle(tint)
        }
    }
}

/// The header while searching (Next-Activity, 04): the field, its clear button, and Cancel in violet.
struct ActivitySearchBar: View {
    @Binding var text: String
    var cancel: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            ActivitySearchField(text: $text, focusOnAppear: true)
            Button("Cancel", action: cancel)
                .font(RFont.sans(16, .medium))
                .foregroundStyle(Palette.violet)
                .frame(minHeight: 44)
                .buttonStyle(PressDimsStyle())
                .accessibilityIdentifier("cancelSearch")
        }
    }
}

/// The missing half of a setup, in the action slot where a waiting card would sit (Next-Wallpaper, 05 and 05b): only
/// computers are connected, so it proposes an AI app; only AI apps, so a computer. Ink-inverse, with the grain.
struct ActionSlotCard: View {
    enum Missing { case aiApp, computer }

    var missing: Missing
    var connect: () -> Void
    var scan: () -> Void
    var typeCode: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                if missing == .aiApp {
                    HStack(spacing: -10) {
                        if let claude = Providers.byKey("claude") { ProviderAvatar(provider: claude, size: 34) }
                        if let openai = Providers.byKey("openai") {
                            ProviderAvatar(provider: openai, size: 34)
                                .background(Circle().fill(Palette.inverse).padding(-2))
                        }
                    }
                } else {
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(Palette.paper)
                        .frame(width: 34, height: 34)
                        .overlay { GlyphIcon(.laptop, size: 18).foregroundStyle(Palette.ink) }
                }
                Spacer(minLength: 0)
                if missing == .computer {
                    Text(verbatim: "reins login").font(RFont.mono(13)).opacity(0.7)
                }
            }
            .accessibilityHidden(true)
            Text(missing == .aiApp ? "Connect an AI app" : "Connect a computer")
                .textRole(.cardAsk)
                .accessibilityAddTraits(.isHeader)
            if missing == .aiApp {
                Button(action: connect) {
                    Label { Text("Connect") } icon: { GlyphIcon(.plus, size: 18) }
                }
                .buttonStyle(InverseCardButton(primary: true))
                .accessibilityIdentifier("slotConnectAi")
            } else {
                HStack(spacing: 8) {
                    Button(action: scan) {
                        Label { Text("Scan") } icon: { GlyphIcon(.qrCode, size: 18) }
                    }
                    .buttonStyle(InverseCardButton(primary: true))
                    .accessibilityIdentifier("slotScan")
                    Button(action: typeCode) {
                        Label { Text("Type code") } icon: { GlyphIcon(.pencil, size: 16) }
                    }
                    .buttonStyle(InverseCardButton(primary: false))
                    .accessibilityIdentifier("slotTypeCode")
                }
            }
        }
        .foregroundStyle(Palette.onInverse)
        .padding(16)
        .background {
            ZStack { Palette.inverse; Grain() }
                .clipShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
        }
        .padding(.horizontal, 16)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("actionSlot")
    }
}

/// A button on the inverse card: paper with ink (the action), or a grey wash.
private struct InverseCardButton: ButtonStyle {
    var primary: Bool

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .labelStyle(SignalLabelStyle())
            .font(TextRole.buttonSmall.font())
            .foregroundStyle(primary ? Palette.ink : Palette.onInverse)
            .lineLimit(1)
            .padding(.horizontal, 16)
            .frame(maxWidth: .infinity, minHeight: 42)
            .background(primary ? Palette.paper : Color.gray.opacity(0.22), in: Capsule())
            .contentShape(Capsule())
            .pressDims(pressed: configuration.isPressed)
    }
}

/// A new account (Next-More, 01): the wallpaper is the focal point; low on the page, "Nothing yet" and the two ways
/// to connect something.
struct NewAccountActivity: View {
    var connectComputer: () -> Void
    var connectAi: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            Text("Nothing yet")
                .textRole(.cardAsk)
                .foregroundStyle(Palette.ink)
                .multilineTextAlignment(.center)
                .accessibilityIdentifier("noActivity")
            VStack(spacing: 10) {
                Button(action: connectComputer) {
                    Label { Text("Connect a computer") } icon: { GlyphIcon(.laptop, size: 18) }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .accessibilityIdentifier("emptyConnectComputer")
                Button(action: connectAi) {
                    Label {
                        Text("Connect an AI app")
                    } icon: {
                        HStack(spacing: -6) {
                            if let claude = Providers.byKey("claude") { ProviderAvatar(provider: claude, size: 24) }
                            if let openai = Providers.byKey("openai") { ProviderAvatar(provider: openai, size: 24) }
                        }
                    }
                }
                .buttonStyle(SignalButtonStyle(.soft))
                .accessibilityIdentifier("emptyConnectAi")
            }
            .padding(.horizontal, 32)
            .padding(.top, 24)
        }
        .frame(maxWidth: .infinity)
    }
}
