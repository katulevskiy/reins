import Observation
import SwiftUI
import UIKit

/// "Join as a second phone", on a phone that is signed in but neither approves nor views (phone roles): the core's
/// viewer enrollment (`enroll_begin`, then `enroll_poll`), seen as steps. The approving phone decides; this phone only
/// shows the code to compare.
enum ViewerJoinStep: Equatable {
    /// Nothing asked yet.
    case idle
    /// Asking the server.
    case starting
    /// Asked; the approving phone has not answered yet.
    case waiting
    /// The code to compare with the approving phone's ("482 193").
    case compare(String)
    case enrolled
    case denied
    case expired
    case failed(String)

    /// Still waiting on the approving phone: keep polling.
    var polling: Bool {
        switch self {
        case .waiting, .compare: true
        default: false
        }
    }

    /// Over: Try again starts a new request (none after an enrollment).
    var ended: Bool {
        switch self {
        case .denied, .expired, .failed: true
        default: false
        }
    }

    /// The step after a poll. A code once shown is never taken back by a late "waiting" (the core keeps the first
    /// challenge and refuses another); the end states are final.
    func after(_ progress: EnrollProgress) -> ViewerJoinStep {
        switch progress {
        case .waiting:
            if case .compare = self { return self }
            return .waiting
        case let .compare(code): return .compare(Self.spaced(code))
        case .enrolled: return .enrolled
        case .denied: return .denied
        case .expired: return .expired
        }
    }

    /// A poll that failed: a lost connection keeps waiting, an enrollment the core no longer has is over.
    func after(failure: Error) -> ViewerJoinStep {
        switch failure {
        case CoreError.Network: self
        case CoreError.NotFound: .expired
        default: .failed(failure.userMessage)
        }
    }

    /// "482193" as "482 193": two groups of three to compare at a glance.
    static func spaced(_ code: String) -> String {
        code.count == 6 && !code.contains(" ") ? String(code.prefix(3)) + " " + String(code.suffix(3)) : code
    }
}

@Observable
@MainActor
final class ViewerJoinModel {
    private(set) var step: ViewerJoinStep = .idle
    var pollInterval: Duration = .seconds(2)
    private var task: Task<Void, Never>?

    /// Asks the approving phone (again, after an end), then follows the answer.
    func start(_ app: AppModel, deviceName: String = UIDevice.current.name) {
        guard step != .starting, !step.polling, step != .enrolled else { return }
        step = .starting
        task?.cancel()
        task = Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await app.core.enrollBegin(deviceName: deviceName)
                step = .waiting
            } catch {
                app.feedback.play(.error)
                step = .failed(error.userMessage)
                return
            }
            while step.polling, !Task.isCancelled {
                try? await Task.sleep(for: pollInterval)
                if Task.isCancelled { return }
                let next: ViewerJoinStep
                do {
                    next = step.after(try await app.core.enrollPoll())
                } catch {
                    next = step.after(failure: error)
                }
                if case .compare = next, !isComparing { app.feedback.play(.open) }
                step = next
            }
            switch step {
            case .enrolled:
                app.feedback.play(.connected)
                // A viewer now: what the server keeps for the account's phones loads.
                await app.refreshConnections()
            case .denied:
                app.feedback.play(.denied)
            default:
                break
            }
        }
    }

    private var isComparing: Bool {
        if case .compare = step { return true }
        return false
    }

    /// Closed before the end: the request is withdrawn (the approving phone can no longer approve it).
    func cancel(_ app: AppModel) {
        let asked = step.polling
        task?.cancel()
        task = nil
        step = .idle
        if asked {
            // It expires on the server anyway.
            Task { try? await app.core.enrollCancel() }
        }
    }
}

/// The sheet: short, as on the boards (Pair and join).
struct ViewerJoinSheet: View {
    var vm: ViewerJoinModel
    var onClose: () -> Void
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Join as a second phone")
                    .font(RFont.sans(20, .semibold))
                    .foregroundStyle(Palette.ink)
                Spacer()
                Button {
                    vm.cancel(model)
                    onClose()
                } label: {
                    GlyphIcon(.close, size: 16)
                        .foregroundStyle(Palette.ink2)
                        .frame(width: 36, height: 36)
                        .background(Palette.paperSunken, in: Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close")
                .accessibilityIdentifier("viewerJoinCloseSheet")
            }
            content
            Spacer(minLength: 0)
        }
        .padding(24)
        .presentationDetents([.medium])
        .presentationCornerRadius(32)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("viewerJoin")
    }

    @ViewBuilder private var content: some View {
        switch vm.step {
        case .idle:
            Text(ViewerJoinWords.intro)
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
            Button("Ask the approving phone") { vm.start(model) }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .accessibilityIdentifier("viewerJoinStart")
        case .starting, .waiting:
            ProgressView()
            Text("Open Reins on the approving phone.")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .accessibilityIdentifier("viewerJoinWaiting")
        case let .compare(code):
            Text(verbatim: code)
                .font(RFont.mono(44, .medium))
                .foregroundStyle(Palette.ink)
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityLabel("Code \(code)")
                .accessibilityIdentifier("viewerJoinCode")
            Text("Approve there only if it shows this code.")
                .font(RFont.sans(15, .medium))
                .foregroundStyle(Palette.ink2)
        case .enrolled:
            Text("This phone sees your connections now.")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink)
                .accessibilityIdentifier("viewerJoinDone")
            Button("Done", action: onClose)
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .accessibilityIdentifier("viewerJoinClose")
        case .denied, .expired, .failed:
            Text(ViewerJoinWords.ended(vm.step))
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("viewerJoinEnded")
            Button("Try again") { vm.start(model) }
                .buttonStyle(SignalButtonLook(kind: .outline))
                .accessibilityIdentifier("viewerJoinRetry")
        }
    }
}

enum ViewerJoinWords {
    static let intro =
        "Your approving phone adds this one as a second phone. It sees your connections and activity; it decides nothing."

    static func ended(_ step: ViewerJoinStep) -> String {
        switch step {
        case .denied: "The approving phone said no."
        case .expired: "The request ran out."
        case let .failed(message): message
        default: ""
        }
    }
}
