import SwiftUI

// The hero of the approval screen (the Android app's ApprovalHero.kt): tinted by what is asked, the AI → the service,
// one sentence, and Autopilot's score (or why there is none).

extension HeroTone {
    var color: Color {
        switch self {
        case .write, .send: Palette.send
        case .search: Palette.search
        case .grant: Palette.accent
        case .danger: Palette.danger
        }
    }
}

/// An Activity entry shown again with the approval's own screen, read-only (the Android app's `ApprovalRecord`): how
/// it ended, which the hero shows in place of Autopilot's score.
struct ApprovalRecord: Equatable {
    var outcome: String
    var symbol: String
    var tint: Color
    /// The risk the model saw in it, when it rated one.
    var risk: RiskView? = nil
}

private struct ApprovalRecordKey: EnvironmentKey {
    static let defaultValue: ApprovalRecord? = nil
}

extension EnvironmentValues {
    /// Set for an Activity entry's request as its approval showed it; nil for a request that waits.
    var approvalRecord: ApprovalRecord? {
        get { self[ApprovalRecordKey.self] }
        set { self[ApprovalRecordKey.self] = newValue }
    }
}

/// The main page: the hero, then the facts of the request, by kind.
struct MainPage: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    var gone: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Hero(vm: vm, view: view, autopilot: autopilot)
            if gone {
                Label("No longer waiting", systemImage: "exclamationmark.triangle.fill")
                    .font(RFont.sans(13.5, .semibold))
                    .foregroundStyle(Palette.warning)
                    .padding(.horizontal, 22)
                    .accessibilityIdentifier("noLongerWaiting")
            }
            Facts(vm: vm, view: view).padding(.top, 4)
        }
        .animation(.smooth(duration: 0.25), value: vm.error)
    }
}

private struct Hero: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel

    var body: some View {
        let tone = ApprovalWords.tone(view)
        let color = tone.color
        let danger = tone == .danger
        let shape = RoundedRectangle(cornerRadius: 30, style: .continuous)
        VStack(spacing: 0) {
            Stage(view: view, tone: color)
            Sentence(view: view).padding(.top, 20)
            HeroPills(vm: vm, view: view, autopilot: autopilot).padding(.top, 16)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 22)
        .padding(.top, 26)
        .padding(.bottom, 20)
        .background(LinearGradient(colors: [color.opacity(danger ? 0.22 : 0.20), color.opacity(0.05)], startPoint: .top, endPoint: .bottom), in: shape)
        .overlay(shape.strokeBorder(color.opacity(danger ? 0.32 : 0.26), lineWidth: 1))
        .padding(.horizontal, 12)
        .padding(.top, 4)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hero")
    }
}

/// The AI's avatar, three dots (the last in the hero's colour), the service's logo with the action's badge.
private struct Stage: View {
    var view: ApprovalView
    var tone: Color

    var body: some View {
        HStack(spacing: 14) {
            ConnectionIcon(connectionId: view.connectionId, label: view.connectionLabel, size: 58)
            HStack(spacing: 5) {
                ForEach(0..<3, id: \.self) { i in
                    Circle().fill(i == 2 ? tone : Palette.tertiary).frame(width: 5, height: 5)
                }
            }
            ApprovalServiceTile(service: ApprovalWords.service(view), badge: ApprovalWords.badge(view).symbol, tone: tone)
        }
        .accessibilityHidden(true)
    }
}

private struct ApprovalServiceTile: View {
    var service: String
    var badge: String
    var tone: Color

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 17, style: .continuous)
        ZStack {
            shape.fill(Palette.dynamic(0xFFFFFF, 0x1C1C20))
            shape.strokeBorder(Palette.dynamic(0xE2E2E6, 0x2C2C31), lineWidth: 1)
            if UIImage(named: "service-\(service)") != nil {
                Image("service-\(service)")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(serviceColor(service))
                    .frame(width: 30, height: 30)
            } else {
                Image(systemName: "desktopcomputer").font(.system(size: 24, weight: .medium)).foregroundStyle(Palette.secondary)
            }
        }
        .frame(width: 58, height: 58)
        .overlay(alignment: .bottomTrailing) {
            Image(systemName: badge)
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(tone)
                .frame(width: 25, height: 25)
                .background(tone.mix(with: Palette.dynamic(0xFFFFFF, 0x111111), by: 0.72), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(tone.mix(with: Palette.dynamic(0xFFFFFF, 0x111111), by: 0.92), lineWidth: 2.5))
                .offset(x: 6, y: 6)
        }
        .accessibilityLabel(serviceName(service))
    }
}

/// "Wants to **push 8 commits**\nto **octo/app**": the key nouns bold, the rest quieter.
private struct Sentence: View {
    var view: ApprovalView

    var body: some View {
        let text = ApprovalWords.sentence(view).reduce(Text("")) { out, span in
            out + Text(span.text)
                .font(RFont.sans(26, span.bold ? .semibold : .medium))
                .foregroundStyle(span.bold ? Palette.text : Palette.secondary)
        }
        text
            .multilineTextAlignment(.center)
            .lineSpacing(2)
            .lineLimit(5)
            .minimumScaleFactor(0.8)
            .frame(maxWidth: .infinity)
            .accessibilityAddTraits(.isHeader)
            // A question is answered, so it is the question itself.
            .accessibilityIdentifier(view.ask != nil ? "askQuestion" : "what")
    }
}

/// Under the sentence: a danger tag with its (?), Autopilot's score, or the offer to download its model. An Activity
/// entry shows how it ended instead, beside what made it risky.
private struct HeroPills: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback
    @Environment(\.approvalRecord) private var record

    var body: some View {
        let danger = ApprovalWords.dangerTag(view)
        if let record {
            let outcome = EntryOutcomePill(text: record.outcome, symbol: record.symbol, tint: record.tint)
            VStack(spacing: 8) {
                if let danger {
                    let tag = WarnTag(text: danger.label, tint: danger.destructive ? Palette.danger : Palette.warning, symbol: danger.symbol)
                        .accessibilityIdentifier(danger.id)
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) { tag; outcome }
                        VStack(spacing: 8) { tag; outcome }
                    }
                } else {
                    outcome
                }
                if let risk = record.risk { RiskChips(risk: risk) }
            }
        } else {
            VStack(spacing: 10) {
                if let danger {
                    HStack(spacing: 8) {
                        WarnTag(text: danger.label, tint: danger.destructive ? Palette.danger : Palette.warning, symbol: danger.symbol)
                            .accessibilityIdentifier(danger.id)
                        HelpDot(id: danger.id, text: danger.help)
                    }
                }
                // A guard rule that can be undone is an ordinary question: Autopilot's score shows under it.
                if danger?.destructive != true {
                    pills
                }
                // The risk the model rated, the destructive ones above all: what Overnight weighs.
                if let risk = vm.suggestion?.risk {
                    RiskChips(risk: risk)
                }
            }
        }
    }

    @ViewBuilder private var pills: some View {
        if let s = vm.suggestion, s.judged, !s.floor {
            AutopilotPill(suggestion: s) {
                nav.overlay = .autopilot
                feedback.play(.open)
            }
        } else if let model = autopilot.model, model.state != .installed, vm.suggestion?.floor != true {
            DownloadPill(autopilot: autopilot, model: model)
        } else if let s = vm.suggestion, s.floor {
            Button {
                nav.overlay = .autopilot
                feedback.play(.open)
            } label: {
                HStack(spacing: 4) {
                    Text("Always asked").font(RFont.sans(13.5, .semibold))
                    Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold))
                }
                .foregroundStyle(Palette.secondary)
                .padding(.leading, 13)
                .padding(.trailing, 9)
                .padding(.vertical, 7)
                .background(Palette.controlFill, in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("suggestion")
        }
    }
}

/// A tinted tag with a real symbol (the warning sign unless said otherwise).
struct WarnTag: View {
    var text: String
    var tint: Color
    var small = false
    var symbol = "exclamationmark.triangle.fill"

    var body: some View {
        Label(text, systemImage: symbol)
            .labelStyle(.titleAndIcon)
            .font(RFont.sans(small ? 11.5 : 13.5, .semibold))
            .foregroundStyle(tint)
            .padding(.horizontal, small ? 8 : 12)
            .padding(.vertical, small ? 4 : 7)
            .background(tint.opacity(0.14), in: Capsule())
            .lineLimit(1)
    }
}

/// "Approve ▬ 97% ›": what Autopilot would do, how sure, in the verdict's colour. Opens its reasons.
private struct AutopilotPill: View {
    var suggestion: SuggestionView
    var onOpen: () -> Void

    var body: some View {
        let s = suggestion
        let tint = SuggestionText.verdictTint(s.verdict)
        let p = Double(s.verdict == .deny ? s.pDeny : s.pApprove)
        Button(action: onOpen) {
            HStack(spacing: 8) {
                Text(ApprovalWords.verdict(s.verdict))
                Capsule()
                    .fill(Color.white.opacity(0.12))
                    .frame(width: 28, height: 4)
                    .overlay(alignment: .leading) { Capsule().fill(tint).frame(width: 28 * min(max(p, 0), 1), height: 4) }
                Text(SuggestionText.percent(Float(p)))
                Image(systemName: "chevron.right").font(.system(size: 11, weight: .semibold)).opacity(0.7)
            }
            .font(RFont.sans(13.5, .semibold))
            .foregroundStyle(tint)
            .padding(.leading, 13)
            .padding(.trailing, 9)
            .padding(.vertical, 7)
            .background(tint.opacity(0.12), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Autopilot: \(ApprovalWords.verdict(s.verdict)), \(SuggestionText.percent(Float(p)))")
        .accessibilityIdentifier("suggestion")
    }
}

/// No model on this phone: "Get Autopilot's score [Download]"; while it downloads, how far.
private struct DownloadPill: View {
    var autopilot: AutopilotModel
    var model: ModelStatus
    @State private var askMobileData = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let blue = Palette.search
        HStack(spacing: 8) {
            switch (model.state, autopilot.downloadJob) {
            case (.downloading, _):
                let pct = model.sizeBytes > 0 ? Int(model.downloadedBytes * 100 / model.sizeBytes) : nil
                Text(pct.map { "Downloading… \($0)%" } ?? "Downloading…")
                    .padding(.trailing, 7)
                    .accessibilityIdentifier("modelProgress")
            case (_, .waiting):
                Text(autopilot.waitingForWifi ? "Waiting for Wi-Fi" : "Waiting for a network")
                    .padding(.trailing, 7)
                    .accessibilityIdentifier("modelProgress")
            default:
                Text("Get Autopilot's score")
                Button {
                    feedback.play(.selection)
                    if autopilot.needsMobileDataConsent() { askMobileData = true } else { autopilot.download() }
                } label: {
                    Label("Download", systemImage: "arrow.down.circle.fill")
                        .font(RFont.sans(12.5, .semibold))
                        .foregroundStyle(Palette.dynamic(0xFFFFFF, 0x04122A))
                        .padding(.leading, 8)
                        .padding(.trailing, 11)
                        .padding(.vertical, 4)
                        .background(blue, in: Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("downloadModel")
            }
        }
        .font(RFont.sans(13, .semibold))
        .foregroundStyle(blue)
        .padding(.leading, 13)
        .padding(.trailing, 6)
        .padding(.vertical, 6)
        .background(blue.opacity(0.12), in: Capsule())
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("getAutopilot")
        .confirmationDialog("Download over mobile data?", isPresented: $askMobileData, titleVisibility: .visible) {
            Button("Download now") { autopilot.downloadNow() }
            Button("Wait for Wi-Fi") { autopilot.download() }
        }
    }
}
