import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI

// The pieces Circles share: an SF Symbol on the rows' tile, a person's avatar and name, the owner's progress line on a
// card, the role chip, the six digits and the invitation's QR code.

/// An SF Symbol on a row's 32 pt tile (`GlyphTile`'s look), for what the glyph set has no mark for.
struct SymbolTile: View {
    var symbol: String
    var tint: Color = Palette.ink2
    var side: CGFloat = 32

    var body: some View {
        RoundedRectangle(cornerRadius: side * 0.28, style: .continuous)
            .fill(Palette.paperSunken)
            .frame(width: side, height: side)
            .overlay { Image(systemName: symbol).font(.system(size: side * 0.44, weight: .semibold)).foregroundStyle(tint) }
            .accessibilityHidden(true)
    }
}

/// Someone in a Circle: their blobatar (from the email, the same everywhere) beside the short name, the email under it.
struct CirclePersonLabel: View {
    var email: String
    var size: CGFloat = 32
    var showEmail = true

    var body: some View {
        HStack(spacing: 12) {
            BlobAvatar(seed: untrusted(email), size: size)
            VStack(alignment: .leading, spacing: 1) {
                Text(verbatim: CircleLogic.name(email))
                    .textRole(.row)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                if showEmail {
                    Text(verbatim: untrustedName(email, limit: 64))
                        .font(RFont.sans(12.5))
                        .foregroundStyle(Palette.inkMeta)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                }
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// The role in a small chip: its symbol and its name.
struct CircleRoleChip: View {
    var role: CircleRole

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: CircleLogic.roleSymbol(role)).font(.system(size: 11, weight: .semibold))
            Text(CircleLogic.roleName(role)).lineLimit(1)
        }
        .font(RFont.sans(11.5, .semibold))
        .foregroundStyle(Palette.violet)
        .padding(.horizontal, 7)
        .frame(minHeight: 20)
        .background(Palette.violet.opacity(0.12), in: Capsule())
        .fixedSize()
        .accessibilityElement(children: .combine)
    }
}

/// On the owner's card: who else decides, and how far it got ("Needs Alex too", "Alex approved, waiting for you"), or,
/// once the owner approved, that their part is done.
struct CircleProgressLine: View {
    var progress: CircleProgress
    /// On a waiting card's coloured field rather than paper.
    var onField = false

    var body: some View {
        let line = CircleLogic.progressLine(progress)
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: progress.youApproved ? "checkmark.circle.fill" : "person.2")
                .font(.system(size: 13, weight: .semibold))
            Text(verbatim: line).lineLimit(2)
        }
        .font(RFont.sans(13.5, .semibold))
        .foregroundStyle(color)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(line)
    }

    private var color: Color {
        if onField { return progress.youApproved ? Palette.onField : Palette.onField2 }
        return progress.youApproved ? Palette.success : Palette.ink2
    }
}

/// The six digits both phones show, large, in ink.
struct CircleCode: View {
    var code: String

    var body: some View {
        Text(verbatim: code)
            .font(RFont.mono(44, .medium))
            .tracking(44 * 0.06)
            .monospacedDigit()
            .foregroundStyle(Palette.ink)
            .lineLimit(1)
            .minimumScaleFactor(0.5)
            .environment(\.layoutDirection, .leftToRight)
            .frame(maxWidth: .infinity)
            .accessibilityLabel("Code \(code.map { String($0) }.filter { $0 != " " }.joined(separator: " "))")
    }
}

/// A QR code for `text`, drawn crisp (no smoothing) in ink on white.
struct QRCodeView: View {
    var text: String
    var side: CGFloat = 200

    var body: some View {
        Group {
            if let image = Self.image(text) {
                Image(uiImage: image)
                    .interpolation(.none)
                    .resizable()
                    .scaledToFit()
            } else {
                Image(systemName: "qrcode").font(.system(size: side * 0.4)).foregroundStyle(Palette.inkMeta)
            }
        }
        .frame(width: side, height: side)
        .padding(12)
        .background(.white, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityLabel("QR code")
    }

    static func image(_ text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage,
              let cg = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}
