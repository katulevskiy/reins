import SwiftUI

/// The dock (the kit's `dock`; the Android app's ApprovalDock.kt): how long to remember it (Once · 1 hour · 1 day ·
/// Custom ▾, or "Every time" when nothing may be remembered), then Deny in outline and Approve in ink with the phone's
/// biometry, on paper with a hairline ring and round top corners. It stays under every page of the screen. At
/// accessibility text sizes (Next-AX) how long is one "For  Once ▾" row and Approve sits over Deny, both full width.
struct ApprovalDock: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(AppModel.self) private var model
    @Environment(\.dynamicTypeSize) private var typeSize

    /// Others in the owner's Circle decide it too: who, and how far it got.
    private var circle: CircleProgress? { model.pending.first { $0.id == view.requestId }?.circle }

    /// The owner approved; it runs once the others have.
    private var approvedByYou: Bool { circle?.youApproved == true }

    var body: some View {
        let ax = typeSize.isAccessibilitySize
        VStack(spacing: 10) {
            if let circle {
                CircleProgressLine(progress: circle)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .accessibilityIdentifier("dockCircle")
            }
            if let error = vm.error {
                Text(error)
                    .font(RFont.sans(13.5, .medium))
                    .foregroundStyle(Palette.danger)
                    .lineLimit(ax ? nil : 3)
                    .fixedSize(horizontal: false, vertical: ax)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 6)
                    .accessibilityIdentifier("approvalError")
            }
            if view.kind == .grant, view.grant != nil {
                GrantPeriodRow(vm: vm, view: view)
            } else if ApprovalWords.rememberLocked(view) || ApprovalWords.held(view) {
                Chooser.asked(ApprovalWords.mailPermissionSettings(view) ? "Approve once" : "Every time")
                    .accessibilityIdentifier("noStanding")
                if ApprovalWords.mailPermissionSettings(view) {
                    Text("For ongoing access, use account or temporary permissions in Trust. Sensitive requests can still require review.")
                        .font(RFont.sans(13)).foregroundStyle(Palette.ink2).fixedSize(horizontal: false, vertical: true).accessibilityIdentifier("accountPermissionHelp")
                }
            } else {
                if ApprovalWords.likeThisOn(view, vm.draft) { LikeThisNote() }
                if let wider = ApprovalWords.secretsWider(view, vm.draft) { SecretsCommandNote(command: wider) }
                if ax { DockMenu(vm: vm, view: view) } else { DockSegments(vm: vm, view: view) }
            }
            // Later, Why? and a denial with a note, for a request that waits here.
            if !vm.readOnly, let item = model.pending.first(where: { $0.id == view.requestId }), item.kind == .request {
                TicketDockRow(vm: vm, item: item)
            }
            if ax {
                VStack(spacing: 10) {
                    approve
                    deny
                }
                .disabled(vm.busy)
            } else {
                RatioRow(spacing: 10, ratio: 1 / 1.35) {
                    deny
                    approve
                }
                .disabled(vm.busy)
                .frame(height: 54)
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 12)
        .padding(.bottom, 6)
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

    private var deny: some View {
        Button {
            Task { await vm.deny(model) }
        } label: {
            Text(ApprovalWords.denyLabel(view))
        }
        .buttonStyle(SignalButtonLook(kind: .outline, height: typeSize.isAccessibilitySize ? 64 : nil))
        .accessibilityIdentifier("deny")
    }

    @ViewBuilder private var approve: some View {
        if approvedByYou {
            // Done on the owner's side: nothing more to press until the others answer.
            Label("Approved", systemImage: "checkmark")
                .font(TextRole.button.font())
                .foregroundStyle(Palette.success)
                .frame(maxWidth: .infinity)
                .frame(height: typeSize.isAccessibilitySize ? 64 : 54)
                .background(Palette.success.opacity(0.12), in: Capsule())
                .accessibilityIdentifier("approvedByYou")
        } else if ApprovalWords.held(view) {
            HoldButton(
                title: "Hold to \(ApprovalWords.approveLabel(view, vm.draft).lowercased())", symbol: Biometry.symbol, busy: vm.busy,
                height: typeSize.isAccessibilitySize ? 64 : 54
            ) {
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
                            BiometryMark(size: typeSize.isAccessibilitySize ? 24 : 18)
                    }
                    Text(ApprovalWords.approveLabel(view, vm.draft)).lineLimit(1)
                }
            }
            .buttonStyle(SignalButtonLook(kind: .inverse, height: typeSize.isAccessibilitySize ? 64 : nil))
            .accessibilityIdentifier("approve")
        }
    }
}

/// The phone's biometry on a button: the Face ID glyph of the set, else the system's mark (Touch ID, Optic ID, the
/// passcode's lock).
struct BiometryMark: View {
    var size: CGFloat = 18

    var body: some View {
        if Biometry.symbol == "faceid" {
            GlyphIcon(.faceId, size: size)
        } else {
            Image(systemName: Biometry.symbol).font(.system(size: size, weight: .semibold))
        }
    }
}

/// Once · 1 hour · 1 day · Custom ▾, as the kit's chooser.
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

/// The same choices at accessibility sizes: one "For  Once ▾" row on the sunken capsule, a menu of them.
private struct DockMenu: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let short = view.kind == .accounts ? "1 hour" : ApprovalWords.shortLabel(view)
        let current = switch ApprovalWords.segment(vm.draft) {
        case .once: "Once"
        case .short: short
        case .day: "1 day"
        case .custom: ApprovalWords.customLabel(vm.draft)
        }
        Menu {
            Button("Once") { set(.once) }.accessibilityIdentifier("lifetime:once")
            Button(short) { set(.hour) }.accessibilityIdentifier("lifetime:hour")
            Button("1 day") { set(.day) }.accessibilityIdentifier("lifetime:day")
            Button("Custom…") {
                nav.overlay = .remember
                feedback.play(.open)
            }
            .accessibilityIdentifier("lifetimeCustom")
        } label: {
            HStack(spacing: 8) {
                Text("For").foregroundStyle(Palette.ink2)
                Spacer(minLength: 8)
                Text(current).foregroundStyle(Palette.ink).multilineTextAlignment(.trailing)
                GlyphIcon(.chevronDown, size: 16).foregroundStyle(Palette.ink)
            }
            .font(RFont.sans(14, .semibold))
            .padding(.horizontal, 20)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, minHeight: 54)
            .background(Palette.paperSunken, in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("For \(current)")
        .accessibilityIdentifier("lifetimeMenu")
    }

    private func set(_ kind: LifetimeKind) {
        vm.draft.lifetime = kind
        vm.draft.allMail = nil
        feedback.play(.selection)
    }
}

/// A permission the AI asked for (Next-Kinds 04): "For 1 hour ▾", which can only be made shorter in its sheet.
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
            HStack(spacing: 8) {
                GlyphIcon(.timer, size: 16)
                Text("For \(ApprovalWords.period(secs))")
                GlyphIcon(.chevronDown, size: 13)
            }
            .font(RFont.sans(14, .semibold))
            .foregroundStyle(Palette.ink2)
            .padding(.horizontal, 18)
            .frame(maxWidth: .infinity, minHeight: 46)
            .background(Palette.paperSunken, in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel("Allow for \(ApprovalWords.period(secs))")
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
                Text(command).font(RFont.mono(12.5, .medium)).fixedSize(horizontal: false, vertical: true)
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
