import SwiftUI

/// Reins Secrets, "Revoke on all computers" (a grant for an environment, an Activity entry that released one): the
/// permissions for the environment end on this phone and every paired computer is told to drop the values it holds.
/// A computer not told yet is named ("Not yet delivered to studio"); only when every one was told does it say so.
struct RevokeEverywhereSection: View {
    var project: String
    var env: String?
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var confirming = false
    @State private var busy = false
    @State private var result: SecretsRevoked?
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GroupCard {
                Button(role: .destructive) {
                    confirming = true
                } label: {
                    HStack(spacing: 14) {
                        Image(systemName: "nosign")
                            .font(.system(size: 17, weight: .medium))
                            .frame(width: 22)
                        Text("Revoke on all computers").font(RFont.sans(16, .medium)).lineLimit(1)
                        Spacer(minLength: 8)
                        if busy { ProgressView() }
                    }
                    .foregroundStyle(Palette.danger)
                    .padding(.horizontal, 16)
                    .frame(minHeight: 52)
                    .contentShape(Rectangle())
                }
                .buttonStyle(RowButtonStyle())
                .disabled(busy)
                .accessibilityIdentifier("revokeEverywhere")
                if let result {
                    if result.notDelivered.isEmpty {
                        Hairline()
                        Label("Revoked on all computers", systemImage: "checkmark.circle.fill")
                            .font(RFont.sans(14.5, .medium))
                            .foregroundStyle(Palette.success)
                            .padding(.horizontal, 16)
                            .frame(minHeight: 48)
                            .accessibilityIdentifier("revokedEverywhere")
                    } else {
                        ForEach(Array(result.notDelivered.enumerated()), id: \.offset) { i, name in
                            Hairline()
                            HStack(spacing: 10) {
                                Label("Not yet delivered to \(untrusted(name))", systemImage: "exclamationmark.triangle.fill")
                                    .font(RFont.sans(14.5, .medium))
                                    .foregroundStyle(Palette.warning)
                                    .lineLimit(2)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                if i == 0 {
                                    HelpDot(id: "notDelivered", text: "Sent again on each sync until it arrives. Ended on this phone already.", size: 28)
                                }
                            }
                            .padding(.leading, 16)
                            .padding(.trailing, 12)
                            .frame(minHeight: 48)
                            .accessibilityIdentifier("notDelivered:\(i)")
                        }
                    }
                }
            }
            if let error {
                FormBanner(text: error).padding(.horizontal, 16)
            }
        }
        .confirmationDialog("Revoke on all computers?", isPresented: $confirming, titleVisibility: .visible) {
            Button("Revoke", role: .destructive) { revoke() }
        } message: {
            Text([untrusted(project), env.map(untrusted)].compactMap { $0 }.joined(separator: " · "))
        }
        .presentationFeedback(confirming)
    }

    private func revoke() {
        guard !busy else { return }
        busy = true
        error = nil
        result = nil
        feedback.quietClose()
        let (target, environment) = (self.project, self.env)
        Task {
            let outcome = await model.revokeSecretsEverywhere(project: target, env: environment)
            busy = false
            result = outcome.revoked
            error = outcome.error
        }
    }
}
