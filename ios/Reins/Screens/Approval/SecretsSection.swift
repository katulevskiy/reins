import SwiftUI

// Reins Secrets on the approval screen: a project's environment released to the computer for one command, values from
// the computer saved in the vault with one approval, and a git host's token for the phone. Everything shown came from
// the desktop app; values never reach the phone's screen, only names.

/// A project's environment for one command (`reins run`): who runs it, the command in full, a warning when the program
/// is not where it usually is, the environment, the variables' names, the folder, and how long the values are kept.
struct EnvReleaseFacts: View {
    @Bindable var vm: ApprovalModel
    var release: EnvReleaseView

    var body: some View {
        let warning = release.programWarning.map(untrusted).flatMap { $0.isEmpty ? nil : $0 }
        VStack(alignment: .leading, spacing: 10) {
            FactGroup(border: warning == nil ? nil : Palette.warning.opacity(0.4)) {
                // The program not being where it usually is comes first, before anything else about the release.
                if let warning {
                    ProgramWarningRow(warning: warning, program: untrusted(release.program))
                }
                FactRow(key: "Who", value: untrusted(release.runnerLabel), valueId: "envWho") {
                    if release.agent { TintTag(text: "Agent", tint: Palette.kindPush).accessibilityIdentifier("envAgent") }
                }
                // The command in full, wrapped and never cut: it is what the values are for.
                VStack(alignment: .leading, spacing: 6) {
                    Text("Command").font(RFont.sans(13, .medium)).foregroundStyle(Palette.inkMeta)
                    Text(untrusted(release.command))
                        .font(RFont.mono(14, .medium))
                        .foregroundStyle(Palette.ink)
                        .fixedSize(horizontal: false, vertical: true)
                        .environment(\.layoutDirection, .leftToRight)
                        .textSelection(.enabled)
                        .accessibilityIdentifier("envCommand")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 18)
                .padding(.vertical, 13)
                FactRow(key: "Env", value: "\(untrusted(release.project)) · \(untrusted(release.env))", mono: true, valueId: "envName")
                variables
                FactRow(key: "In", value: untrusted(release.root), mono: true, valueId: "envRoot")
                if release.print {
                    FactRow(key: "Value", value: "Printed", valueId: "envLease", tint: Palette.warning)
                } else {
                    FactRow(key: "Kept", value: ApprovalWords.kept(release.leaseSecs), valueId: "envLease")
                }
                if let purpose = release.purpose.map(untrusted), !purpose.isEmpty {
                    FactRow(key: "Says", value: "“\(purpose)”", lines: 3)
                }
            }
            .accessibilityContainer("envRelease")
            if !vm.readOnly { AlwaysAskMore(release: release) }
        }
    }

    /// The variables' names: "Every variable", a few inline, or a row that opens all of them.
    @ViewBuilder private var variables: some View {
        let names = release.variables.map(untrusted)
        if release.allVariables {
            FactRow(key: "Variables", value: "Every variable", valueId: "envVariables")
        } else if names.count <= 6 {
            FactRow(key: names.count == 1 ? "Variable" : "Variables", value: names.joined(separator: ", "), mono: true, valueId: "envVariables")
        } else {
            DrillRow(
                title: "\(names.count) variables", id: "envVariables",
                page: .text(title: "Variables", body: names.joined(separator: "\n"), mono: true, id: "envVariablesText")
            ) {
                Text(names.prefix(3).joined(separator: ", "))
                    .font(RFont.mono(13))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                    .environment(\.layoutDirection, .leftToRight)
            }
        }
    }
}

/// "In a temporary folder" and the program's whole path, in the warning colour.
private struct ProgramWarningRow: View {
    var warning: String
    var program: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Palette.warning)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: 3) {
                Text(warning).font(RFont.sans(14.5, .semibold)).foregroundStyle(Palette.warning)
                Text(program)
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
                    .accessibilityIdentifier("envProgram")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
        .background(Palette.warning.opacity(0.10))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("envProgramWarning")
    }
}

/// "More": whether this environment is asked every time. Asked every time by the computer (its reins.toml, a
/// production environment) it stays on here.
private struct AlwaysAskMore: View {
    var release: EnvReleaseView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var open = false
    /// What was set here; nil until then (the request's own answer stands).
    @State private var on: Bool?
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        let isOn = on ?? release.alwaysAsk
        let fixed = on == nil && release.alwaysAsk && !release.alwaysAskSet
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.smooth) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack {
                    Text("More").font(RFont.sans(15, .medium))
                    Spacer()
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).rotationEffect(.degrees(open ? 90 : 0))
                }
                .foregroundStyle(Palette.violet)
                .padding(.horizontal, 22)
                .padding(.vertical, 8)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("envMore")
            .accessibilityValue(open ? "Open" : "Closed")
            if open {
                FactGroup {
                    HStack(spacing: 10) {
                        Toggle(isOn: Binding(get: { isOn }, set: { set($0) })) {
                            Text("Ask every time").font(RFont.sans(15.5)).foregroundStyle(Palette.ink)
                        }
                        .tint(Palette.violet)
                        .disabled(busy || fixed)
                        .accessibilityIdentifier("envAlwaysAsk")
                        HelpDot(
                            id: "envAlwaysAsk",
                            text: fixed ? "Set on the computer, or a production environment." : "Every run with \(untrusted(release.env)) is asked. Nothing is remembered."
                        )
                    }
                    .padding(.leading, 18)
                    .padding(.trailing, 12)
                    .padding(.vertical, 10)
                }
                .transition(.opacity)
                if let error {
                    Text(error)
                        .font(RFont.sans(13.5, .medium))
                        .foregroundStyle(Palette.danger)
                        .lineLimit(3)
                        .padding(.horizontal, 22)
                        .accessibilityIdentifier("envAlwaysAskError")
                }
            }
        }
    }

    private func set(_ value: Bool) {
        guard !busy else { return }
        busy = true
        error = nil
        let (project, env) = (release.project, release.env)
        Task {
            let result = await model.setSecretsAlwaysAsk(project: project, env: env, always: value)
            busy = false
            if result.done { on = value } else { error = result.error }
        }
    }
}

/// Four check digits, large, that the terminal shows too; the user compares them.
struct CheckDigits: View {
    var digits: String
    var id: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            FactHeader(text: "Check digits") {
                HelpDot(id: id, text: "Compare with the terminal.")
            }
            FactGroup(border: Palette.violet.opacity(0.35)) {
                Text(untrusted(digits))
                    .font(RFont.mono(34, .semibold))
                    .tracking(6)
                    .foregroundStyle(Palette.ink)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 16)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityIdentifier(id)
            }
        }
    }
}

/// Values from the computer for the vault: the check digits first, then each environment (what is added, replaced,
/// removed, renamed or copied, by name), the SSH keys and the git hosts' tokens.
struct SecretsStoreFacts: View {
    var store: SecretsStoreView

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CheckDigits(digits: store.check, id: "storeCheck")
            if !store.envs.isEmpty {
                FactGroup {
                    ForEach(Array(store.envs.enumerated()), id: \.offset) { i, env in
                        EnvStoreRow(env: env).accessibilityIdentifier("storeEnv:\(i)")
                    }
                }
            }
            if !store.ssh.isEmpty {
                FactHeader("SSH keys")
                FactGroup {
                    ForEach(Array(store.ssh.enumerated()), id: \.offset) { i, key in
                        VStack(alignment: .leading, spacing: 3) {
                            HStack(spacing: 10) {
                                Image(systemName: "key.fill").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.violet)
                                Text(untrusted(key.name)).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.ink)
                            }
                            Text(untrusted(key.fingerprint))
                                .font(RFont.mono(12.5))
                                .foregroundStyle(Palette.inkMeta)
                                .fixedSize(horizontal: false, vertical: true)
                                .environment(\.layoutDirection, .leftToRight)
                                .textSelection(.enabled)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 18)
                        .padding(.vertical, 12)
                        .accessibilityElement(children: .combine)
                        .accessibilityIdentifier("storeSsh:\(i)")
                    }
                }
            }
            if !store.tokens.isEmpty {
                FactHeader("Tokens")
                FactGroup {
                    ForEach(Array(store.tokens.enumerated()), id: \.offset) { i, token in
                        TokenRow(token: token, showCheck: true).accessibilityIdentifier("storeToken:\(i)")
                    }
                }
            }
        }
        .accessibilityContainer("secretsStore")
    }
}

/// One environment of a store: its name, New when it is made, the counts, and the names compact.
private struct EnvStoreRow: View {
    var env: EnvStoreView

    var body: some View {
        let changes = ApprovalWords.storeChanges(env)
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text("\(untrusted(env.project)) · \(untrusted(env.env))")
                    .font(RFont.sans(15.5, .semibold))
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                if env.newEnv { TintTag(text: "New", tint: Palette.success) }
                Spacer(minLength: 6)
                Counts(
                    additions: env.added.isEmpty ? nil : UInt64(env.added.count),
                    deletions: env.removed.isEmpty ? nil : UInt64(env.removed.count)
                )
            }
            if !changes.isEmpty {
                Text(changes)
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .environment(\.layoutDirection, .leftToRight)
                    .textSelection(.enabled)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 18)
        .padding(.vertical, 13)
    }
}

/// A git host's token: the service, its host, the scopes the computer reported, and its check digits.
private struct TokenRow: View {
    var token: TokenStoreView
    var showCheck: Bool

    var body: some View {
        HStack(spacing: 12) {
            ServiceAvatar(service: token.service, size: 34)
            VStack(alignment: .leading, spacing: 2) {
                Text(serviceName(token.service)).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                Text(untrusted(token.host)).font(RFont.mono(12.5)).foregroundStyle(Palette.inkMeta)
                    .environment(\.layoutDirection, .leftToRight)
                if !token.scopes.isEmpty {
                    Text(token.scopes.map(untrusted).joined(separator: ", "))
                        .font(RFont.mono(12.5))
                        .foregroundStyle(Palette.ink2)
                        .environment(\.layoutDirection, .leftToRight)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if showCheck {
                Text(untrusted(token.check))
                    .font(RFont.mono(15, .semibold))
                    .foregroundStyle(Palette.ink)
                    .environment(\.layoutDirection, .leftToRight)
                    .accessibilityLabel("Check digits \(untrusted(token.check))")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityElement(children: .combine)
    }
}

/// One git host's token for the phone's integration: its check digits, then the service, host and scopes.
struct TokenStoreFacts: View {
    var token: TokenStoreView

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            CheckDigits(digits: token.check, id: "tokenCheck")
            FactGroup {
                TokenRow(token: token, showCheck: false)
                if token.scopes.isEmpty {
                    FactRow(key: "Scopes", value: "None reported", valueId: "tokenScopes")
                }
            }
        }
        .accessibilityContainer("tokenStore")
    }
}

#if DEBUG
#Preview("Environment") {
    PreviewHost { ApprovalSheet(requestId: "req43") }
}
#endif

#if DEBUG
#Preview("Store") {
    PreviewHost { ApprovalSheet(requestId: "req44") }
}
#endif
