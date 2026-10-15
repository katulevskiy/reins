import SwiftUI

/// "Allow for": a few choices side by side in a sunken well, the chosen one raised (Once · 1 hour · 1 day · Custom ▾).
/// A segment can open more choices (its chevron); `locked` replaces the whole control with the one answer there is,
/// for what is asked every time ("Asked every time", with a lock). The approval dock, new grants and Resume use it.
struct Chooser: View {
    struct Segment: Identifiable {
        /// The accessibility identifier ("lifetime:once").
        var id: String
        var title: String
        var selected: Bool
        var enabled = true
        /// Opens more choices: a chevron, and `menu` when the choices are a list.
        var more = false
        var menu: [MenuItem] = []
        var action: () -> Void = {}
    }

    struct MenuItem: Identifiable {
        var id: String
        var title: String
        var enabled = true
        var action: () -> Void
    }

    var segments: [Segment]
    /// The control is locked to this one answer.
    var locked: String?

    init(_ segments: [Segment], locked: String? = nil) {
        self.segments = segments
        self.locked = locked
    }

    /// Asked every time: the lock and its words, in place of the segments.
    static func asked(_ text: String = "Asked every time") -> Chooser { Chooser([], locked: text) }

    @Environment(\.feedback) private var feedback

    var body: some View {
        if let locked {
            HStack(spacing: 8) {
                GlyphIcon(.lock, size: 16)
                Text(locked)
            }
            .font(RFont.sans(14, .medium))
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 46)
            .background(Palette.paperSunken, in: Capsule())
            .accessibilityElement(children: .combine)
        } else {
            HStack(spacing: 2) {
                ForEach(segments) { segment in
                    if segment.menu.isEmpty {
                        Button {
                            if !segment.selected || segment.more { segment.action() }
                        } label: {
                            look(segment)
                        }
                        .buttonStyle(.plain)
                        .disabled(!segment.enabled)
                        .accessibilityAddTraits(segment.selected ? .isSelected : [])
                        .accessibilityIdentifier(segment.id)
                    } else {
                        Menu {
                            ForEach(segment.menu) { item in
                                Button(item.title, action: item.action)
                                    .disabled(!item.enabled)
                                    .accessibilityIdentifier(item.id)
                            }
                        } label: {
                            look(segment)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier(segment.id)
                    }
                }
            }
            .padding(4)
            .background(Palette.paperSunken, in: Capsule())
            .animation(.easeOut(duration: Motion.fade), value: segments.map(\.selected))
        }
    }

    private func look(_ segment: Segment) -> some View {
        HStack(spacing: 4) {
            Text(segment.title).lineLimit(1).minimumScaleFactor(0.8)
            if segment.more { GlyphIcon(.chevronDown, size: 14) }
        }
        .font(RFont.sans(14, .medium))
        .foregroundStyle(segment.selected ? Palette.ink : Palette.ink2)
        .padding(.horizontal, 6)
        .frame(maxWidth: .infinity, minHeight: 38)
        .background {
            if segment.selected {
                Capsule()
                    .fill(Palette.paperRaised)
                    .shadow(color: .black.opacity(0.08), radius: 1, y: 1)
                    .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
            }
        }
        .contentShape(Capsule())
        .opacity(segment.enabled ? 1 : 0.4)
    }
}

/// "Allow for        1 hour · at most 5 uses ▾": a chosen length in one line, which opens where it is changed.
struct ChooserSummary: View {
    var label: String
    var value: String
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Text(label).foregroundStyle(Palette.ink2)
                Spacer(minLength: 8)
                Text(value).foregroundStyle(Palette.ink).lineLimit(1)
                GlyphIcon(.chevronDown, size: 14).foregroundStyle(Palette.ink)
            }
            .font(RFont.sans(14, .medium))
            .padding(.horizontal, 18)
            .frame(maxWidth: .infinity, minHeight: 46)
            .background(Palette.paperSunken, in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
    }
}
