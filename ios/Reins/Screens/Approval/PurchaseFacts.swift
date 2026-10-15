import SwiftUI

// A purchase on the approval (And-Pay 01), display-only on this phone: what the store or Reins flagged first, in full;
// the store and its domain; every line of the cart; the amount in the serif, because it is the point; the card it is
// paid with; and what is left of the spend limit. Everything the AI wrote (the store's name, the items, the note) is
// untrusted text, and nothing that is paid for is cut.

struct PurchaseFacts: View {
    var purchase: PurchaseView

    var body: some View {
        let p = purchase
        FactGroup {
            ForEach(Array(p.warnings.enumerated()), id: \.offset) { i, warning in
                WarnFactRow(text: untrusted(warning)).accessibilityIdentifier("purchaseWarning:\(i)")
            }
            FactRow(key: "Merchant", value: "\(untrusted(p.merchant)) · \(untrusted(p.domain))", valueId: "purchaseMerchant")
            ForEach(Array(p.items.enumerated()), id: \.offset) { i, item in
                ItemRow(item: item).accessibilityIdentifier("purchaseItem:\(i)")
            }
            if let shipping = p.shipping { FactRow(key: "Shipping", value: untrusted(shipping), mono: true) }
            if let tax = p.tax { FactRow(key: "Tax", value: untrusted(tax), mono: true) }
            if let discount = p.discount { FactRow(key: "Discount", value: "−\(untrusted(discount))", mono: true) }
            HStack(alignment: .firstTextBaseline, spacing: 14) {
                Text("Amount").font(RFont.sans(15.5)).foregroundStyle(Palette.ink2)
                Spacer(minLength: 8)
                Text(verbatim: "\(untrusted(p.total)) \(untrusted(p.currency))")
                    .font(RFont.serif(28))
                    .foregroundStyle(Palette.ink)
                    .multilineTextAlignment(.trailing)
                    .fixedSize(horizontal: false, vertical: true)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier("purchaseTotal")
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
            .accessibilityElement(children: .combine)
            FactRow(key: "Card", value: Self.card(p), valueId: "purchaseCard")
            ForEach(Array(p.budgetLines.enumerated()), id: \.offset) { i, line in
                FactRow(key: i == 0 ? "Spend limit" : "", value: untrusted(line), valueId: "purchaseBudget:\(i)")
            }
            if let note = p.note.map(untrusted), !note.isEmpty {
                FactRow(key: "The AI says", value: "“\(note)”", lines: 6)
            }
        }
        .accessibilityContainer("purchase")
    }

    /// The method it would be paid with: "Visa •• 4242 · 04/29", or that none is picked yet.
    static func card(_ p: PurchaseView) -> String {
        guard let m = p.methods.first(where: { $0.id == p.methodId }) else { return "Not picked" }
        var parts = [untrusted(m.name)]
        if let last4 = m.last4, !last4.isEmpty, !m.name.contains(last4) { parts.append("•• \(untrusted(last4))") }
        if let expiry = m.expiry, !expiry.isEmpty { parts.append(untrusted(expiry)) }
        return parts.joined(separator: " · ")
    }

    /// "2 × USB-C cable" and its line total; the name in full.
    private struct ItemRow: View {
        var item: PurchaseLineView

        var body: some View {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: "\(item.quantity) × \(untrusted(item.name))").font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.ink)
                    let each = item.quantity > 1 ? "\(untrusted(item.unitPrice)) each" : nil
                    let meta = [item.details.map(untrusted), each].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
                    if !meta.isEmpty { Text(meta).textRole(.meta).foregroundStyle(Palette.inkMeta) }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
                Text(untrusted(item.lineTotal)).font(TextRole.value.font()).foregroundStyle(Palette.ink)
                    .environment(\.layoutDirection, .leftToRight)
            }
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
            .accessibilityElement(children: .combine)
        }
    }
}

/// A warning as a fact row: the amber sign and the words in full, on an amber wash.
struct WarnFactRow: View {
    var text: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            GlyphIcon(.warning, size: 16).foregroundStyle(Palette.warning).padding(.top, 1)
            Text(text)
                .font(RFont.sans(14.5, .semibold))
                .foregroundStyle(Palette.warning)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
        .background(Palette.warning.opacity(0.10))
        .accessibilityElement(children: .combine)
    }
}
