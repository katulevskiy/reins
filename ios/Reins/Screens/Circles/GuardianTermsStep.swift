import SwiftUI

/// A guardianship's terms (decisions v2 P7, 04 §4), in the Add sheet when the role is Guardian: the notice the ward
/// picks before the link and the code, 0, 3 or 7 days. Leaving ends it after this notice; adults can always leave.
struct GuardianTermsStep: View {
    @Binding var days: UInt8?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Notice to leave", quiet: true) {
                HelpButton(text: GuardianLogic.noticeHelp)
                    .accessibilityIdentifier("noticeHelp")
            }
            Chooser(GuardianLogic.noticeChoices.map { d in
                Chooser.Segment(id: "notice:\(d)", title: GuardianLogic.noticeLabel(d), selected: days == d) { days = d }
            })
            .padding(.horizontal, 20)
            HStack(spacing: 6) {
                Image(systemName: "clock").font(.system(size: 12, weight: .semibold))
                Text(days.map(GuardianLogic.noticeLine) ?? "Pick one")
            }
            .textRole(.meta)
            .foregroundStyle(Palette.inkMeta)
            .padding(.horizontal, 20)
            .padding(.top, 8)
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("noticeLine")
        }
    }
}

/// The notice as one fact row: in the Join sheet (the ward's pick, shown before Accept) and on the Add sheet's code.
struct GuardianNoticeFact: View {
    var days: UInt8

    var body: some View {
        TileRow(GuardianLogic.noticeLine(days), chevron: false) { SymbolTile(symbol: "clock") } trailing: { EmptyView() }
            .flatRow()
            .accessibilityIdentifier("guardianNotice")
    }
}
