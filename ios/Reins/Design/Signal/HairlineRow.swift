import SwiftUI

/// A row of a Signal list: hairlines, never cards. At least 56 pt, 14 pt between its parts, a `rule` hairline under it.
/// Two shapes: a fact (key in ink-2 at the left, its value at the right) and an item (an optional tile, a title in `row`
/// with a `meta` line, something at the right).
struct HairlineRow<Leading: View, Trailing: View>: View {
    var title: String
    var subtitle: String?
    var hairline = true
    @ViewBuilder var leading: Leading
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 14) {
            leading
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(TextRole.row.font())
                    .foregroundStyle(Palette.ink)
                if let subtitle {
                    Text(subtitle).textRole(.meta).foregroundStyle(Palette.inkMeta)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            trailing
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
        .overlay(alignment: .bottom) {
            if hairline { Rectangle().fill(Palette.rule).frame(height: 1) }
        }
        .accessibilityElement(children: .combine)
    }
}

extension HairlineRow where Leading == EmptyView {
    init(title: String, subtitle: String? = nil, hairline: Bool = true, @ViewBuilder trailing: () -> Trailing) {
        self.init(title: title, subtitle: subtitle, hairline: hairline, leading: { EmptyView() }, trailing: trailing)
    }
}

/// "Branch        feature/passkeys": a fact. The value is ink (mono for machine values); a chevron when it opens more.
struct HairlineFact<Trailing: View>: View {
    var key: String
    var value: String?
    var mono = false
    var chevron = false
    var hairline = true
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 14) {
            Text(key)
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let value {
                Text(value)
                    .font(mono ? TextRole.value.font() : RFont.sans(15, .medium))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .truncationMode(mono ? .middle : .tail)
            }
            trailing
            if chevron { GlyphIcon(.chevronRight, size: 14).foregroundStyle(Palette.inkFaint) }
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
        .overlay(alignment: .bottom) {
            if hairline { Rectangle().fill(Palette.rule).frame(height: 1) }
        }
        .accessibilityElement(children: .combine)
    }
}

extension HairlineFact where Trailing == EmptyView {
    init(key: String, value: String?, mono: Bool = false, chevron: Bool = false, hairline: Bool = true) {
        self.init(key: key, value: value, mono: mono, chevron: chevron, hairline: hairline) { EmptyView() }
    }
}
