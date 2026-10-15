import SwiftUI

/// The dock (the Android app's ApprovalDock.kt): how long to remember it (Once · 1 hour · 1 day · Custom ▾, or "Asked
/// every time" when nothing may be remembered), then Deny and Approve, on a raised plate with a hairline on top. It
/// stays under every page of the screen.
struct ApprovalDock: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 10) {
            if let error = vm.error {
                Text(error)
                    .font(RFont.sans(13.5, .medium))
                    .foregroundStyle(Palette.danger)
                    .lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .accessibilityIdentifier("approvalError")
            }
            if view.kind == .grant {
                GrantPeriodRow(vm: vm, view: view)
            } else if ApprovalWords.rememberLocked(view) {
                Chooser.asked()
                    .accessibilityIdentifier("noStanding")
            } else {
                if ApprovalWords.likeThisOn(view, vm.draft) { LikeThisNote() }
                if let wider = ApprovalWords.secretsWider(view, vm.draft) { SecretsCommandNote(command: wider) }
                DockSegments(vm: vm, view: view)
            }
            buttons
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 8)
        .background {
            UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                .fill(Palette.paper)
                .overlay(alignment: .top) {
                    UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                        .strokeBorder(Palette.rule, lineWidth: 1)
                }
                .ignoresSafeArea(edges: .bottom)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("dock")
    }

    private var buttons: some View {
        RatioRow(spacing: 10, ratio: 1 / 1.35) {
            Button {
                Task { await vm.deny(model) }
            } label: {
                Text(ApprovalWords.denyLabel(view))
            }
            .buttonStyle(SignalButtonLook(kind: .outline))
            .accessibilityIdentifier("deny")

            if ApprovalWords.destructive(view) {
                HoldButton(title: "Hold to \(ApprovalWords.approveLabel(view, vm.draft).lowercased())", symbol: Biometry.symbol, busy: vm.busy) {
                    Task { await vm.approve(model) }
                }
                .accessibilityIdentifier("approve")
            } else {
                Button {
                    Task { await vm.approve(model) }
                } label: {
                    HStack(spacing: 8) {
                        if vm.busy {
                            ProgressView().tint(Palette.onInverse)
                        } else {
                            Image(systemName: Biometry.symbol).font(.system(size: 18, weight: .semibold))
                        }
                        Text(ApprovalWords.approveLabel(view, vm.draft)).lineLimit(1)
                    }
                }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .accessibilityIdentifier("approve")
            }
        }
        .disabled(vm.busy)
        .frame(height: 54)
    }
}

/// Once · 1 hour · 1 day · Custom ▾, as the Signal chooser.
private struct DockSegments: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let on = ApprovalWords.segment(vm.draft)
        Chooser([
            Chooser.Segment(id: "lifetime:once", title: "Once", selected: on == .once) { set(.once) },
            Chooser.Segment(
                id: "lifetime:hour", title: view.kind == .accounts ? "1 hour" : ApprovalWords.shortLabel(view), selected: on == .short
            ) { set(.hour) },
            Chooser.Segment(id: "lifetime:day", title: "1 day", selected: on == .day) { set(.day) },
            Chooser.Segment(id: "lifetimeCustom", title: ApprovalWords.customLabel(vm.draft), selected: on == .custom, more: true) {
                nav.overlay = .remember
                feedback.play(.open)
            },
        ])
    }

    private func set(_ kind: LifetimeKind) {
        vm.draft.lifetime = kind
        vm.draft.allMail = nil
        feedback.play(.selection)
    }
}

/// A permission the AI asked for: how long, which can only be made shorter in its sheet.
private struct GrantPeriodRow: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let asked = view.grant?.durationSecs ?? 0
        let secs = vm.draft.grantSeconds.map { UInt64(max($0, 0)) }.map { min($0, asked) } ?? asked
        ChooserSummary(label: "Allow for", value: ApprovalWords.period(secs)) {
            nav.overlay = .grant
            feedback.play(.open)
        }
        .accessibilityIdentifier("grantPeriod")
    }
}

/// "Anything like this" was chosen: the dock says the remembered approval is wider than the screen.
private struct LikeThisNote: View {
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        Button {
            nav.overlay = .remember
        } label: {
            Label("Anything like this", systemImage: "arrow.up.left.and.arrow.down.right")
                .font(RFont.sans(12.5, .semibold))
                .foregroundStyle(Palette.warning)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(Palette.warning.opacity(0.13), in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("likeThisNote")
    }
}

/// A remembered release covers more than the exact command: the dock says which pattern ("npm run *").
private struct SecretsCommandNote: View {
    var command: String
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        Button {
            nav.overlay = .remember
        } label: {
            Label {
                Text(command).font(RFont.mono(12.5, .medium)).lineLimit(1).truncationMode(.middle)
            } icon: {
                Image(systemName: "terminal").font(.system(size: 11.5, weight: .semibold))
            }
            .foregroundStyle(Palette.warning)
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .background(Palette.warning.opacity(0.13), in: Capsule())
            .environment(\.layoutDirection, .leftToRight)
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("secretsCommandNote")
    }
}
