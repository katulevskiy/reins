import SwiftUI

/// A request's icon (design/signal/ASK-TEXT.md): the integration's mark on its white plate (the provider's logo, or a
/// Signal glyph for local kinds: terminal for commands, file for files, key for keys and SSH, lock for secrets, help for
/// questions), and the action as a round badge at its bottom-right corner, filled with the request's field colour and
/// carrying a white glyph, ringed in the colour of what is behind it so it stands apart from the mark.
struct RequestIcon: View {
    var spec: AskIcon
    var size: CGFloat = 40
    /// What the icon sits on, for the badge's ring: paper, or a field's own colour.
    var ring: Color = Palette.paper

    var body: some View {
        let badge = (size * 0.48).rounded()
        let plate = RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
        ZStack {
            plate.fill(Color.white)
            plate.strokeBorder(Color.black.opacity(0.08), lineWidth: 1)
            mark
        }
        .frame(width: size, height: size)
        .overlay(alignment: .bottomTrailing) {
            GlyphIcon(spec.action, size: badge * 0.56)
                .foregroundStyle(Color.white)
                .frame(width: badge, height: badge)
                .background(spec.kind?.fill ?? Palette.ink2, in: Circle())
                .padding(2)
                .background(ring, in: Circle())
                .offset(x: badge * 0.3, y: badge * 0.3)
        }
        .padding(.trailing, badge * 0.3)
        .padding(.bottom, badge * 0.3)
        .accessibilityHidden(true)
    }

    @ViewBuilder private var mark: some View {
        switch spec.mark {
        case let .service(id):
            let key = id.hasPrefix(mcpPrefix) ? "mcp" : id
            if UIImage(named: "service-\(key)") != nil {
                Image("service-\(key)")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    // On the white plate in both themes: a black mark stays black.
                    .foregroundStyle(key == "github" ? Palette.onFieldInk : serviceColor(key))
                    .frame(width: size * 0.54, height: size * 0.54)
            } else {
                GlyphIcon(.link, size: size * 0.5).foregroundStyle(Palette.onFieldInk)
            }
        case let .glyph(glyph):
            GlyphIcon(glyph, size: size * 0.5).foregroundStyle(Palette.onFieldInk)
        }
    }
}

/// A sentence's runs in the serif at a row's size, its target in italic (river rows, lists).
struct AskRunsText: View {
    var runs: [AskText.Run]
    var size: CGFloat

    var body: some View {
        runs.reduce(Text(verbatim: "")) { sentence, run in
            Text("\(sentence)\(Text(verbatim: run.text).font(RFont.serif(size, italic: run.target)))")
        }
    }
}
