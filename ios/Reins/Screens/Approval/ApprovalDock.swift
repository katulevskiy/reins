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
                Label("Asked every time", systemImage: "lock.fill")
                    .font(RFont.sans(13, .medium))
                    .foregroundStyle(Palette.secondary)
                    .frame(maxWidth: .infinity)
                    .padding(11)
                    .background(Palette.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .accessibilityIdentifier("noStanding")
            } else {
                if ApprovalWords.likeThisOn(view, vm.draft) { LikeThisNote() }
                DockSegments(vm: vm, view: view)
            }
            buttons
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 8)
        .background {
            UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                .fill(Palette.dynamic(0xFFFFFF, 0x101012))
                .overlay(alignment: .top) {
                    UnevenRoundedRectangle(topLeadingRadius: 30, topTrailingRadius: 30, style: .continuous)
                        .strokeBorder(Palette.dynamic(0xE2E2E6, 0x1E1E22), lineWidth: 1)
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
                    .font(RFont.sans(17, .semibold))
                    .foregroundStyle(Palette.text)
                    .frame(maxWidth: .infinity, minHeight: 54)
                    .background(Palette.dynamic(0xE7E7EB, 0x232327), in: Capsule())
                    .contentShape(Capsule())
            }
            .buttonStyle(.plain)
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
                            ProgressView().tint(Palette.background)
                        } else {
                            Image(systemName: Biometry.symbol).font(.system(size: 18, weight: .semibold))
                        }
                        Text(ApprovalWords.approveLabel(view, vm.draft)).lineLimit(1)
                    }
                    .font(RFont.sans(17, .semibold))
                    .foregroundStyle(Palette.background)
                    .frame(maxWidth: .infinity, minHeight: 54)
                    .background(Palette.text, in: Capsule())
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("approve")
            }
        }
        .disabled(vm.busy)
        .frame(height: 54)
    }
}

/// Once · 1 hour · 1 day · Custom ▾, as a segmented control.
private struct DockSegments: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let on = ApprovalWords.segment(vm.draft)
        HStack(spacing: 0) {
            segment("Once", on == .once, id: "lifetime:once") { set(.once) }
            segment(view.kind == .accounts ? "1 hour" : ApprovalWords.shortLabel(view), on == .short, id: "lifetime:hour") { set(.hour) }
            segment("1 day", on == .day, id: "lifetime:day") { set(.day) }
            segment(ApprovalWords.customLabel(vm.draft), on == .custom, id: "lifetimeCustom", menu: true) {
                nav.overlay = .remember
                feedback.play(.open)
            }
        }
        .padding(3)
        .background(Palette.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .animation(.smooth(duration: 0.2), value: on)
    }

    private func set(_ kind: LifetimeKind) {
        vm.draft.lifetime = kind
        vm.draft.allMail = nil
        feedback.play(.selection)
    }

    private func segment(_ title: String, _ selected: Bool, id: String, menu: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 3) {
                Text(title).lineLimit(1).minimumScaleFactor(0.85)
                if menu { Image(systemName: "chevron.down").font(.system(size: 10, weight: .bold)) }
            }
            .font(RFont.sans(13.5, .semibold))
            .foregroundStyle(selected ? Palette.text : Palette.secondary)
            .frame(maxWidth: .infinity, minHeight: 36)
            .background(selected ? Palette.dynamic(0xFFFFFF, 0x232327) : Color.clear, in: RoundedRectangle(cornerRadius: 11, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityIdentifier(id)
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
        Button {
            nav.overlay = .grant
            feedback.play(.open)
        } label: {
            HStack(spacing: 4) {
                Text("For \(ApprovalWords.period(secs))")
                Image(systemName: "chevron.down").font(.system(size: 11, weight: .bold)).foregroundStyle(Palette.secondary)
            }
            .font(RFont.sans(13.5, .semibold))
            .foregroundStyle(Palette.text)
            .frame(maxWidth: .infinity)
            .padding(11)
            .background(Palette.background, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
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
