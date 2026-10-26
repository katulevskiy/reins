import LocalAuthentication
import SwiftUI

// What the approval, pairing and upload sheets share: the header, the pinned decision buttons, the split layout on a
// bent iPhone Duo, and the owner check before anything is approved.

/// The details scroll; the decision stays pinned at the bottom on Liquid Glass. On a bent iPhone Duo with a sheet wide
/// (or tall) enough, the details and the decision take one half each, so the crease runs between them.
struct DecisionLayout<Details: View, Recap: View, Decision: View>: View {
    /// The details start with a `SheetField`, which has its own close button.
    var closeInField = false
    @ViewBuilder var details: Details
    /// What is being decided, in a line or two: shown above the buttons when they have a half to themselves.
    @ViewBuilder var recap: Recap
    @ViewBuilder var decision: Decision
    @Environment(\.hinge) private var hinge
    @Environment(AppModel.self) private var model

    var body: some View {
        GeometryReader { geo in
            let split = hinge == .bent && max(geo.size.width, geo.size.height) >= 640
            Group {
                if split {
                    let sideBySide = geo.size.width >= geo.size.height
                    let layout = sideBySide ? AnyLayout(HStackLayout(spacing: 0)) : AnyLayout(VStackLayout(spacing: 0))
                    layout {
                        ScrollView { details.padding(.bottom, 24) }
                            .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                        VStack(spacing: 16) {
                            Spacer(minLength: 0)
                            recap.frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 20)
                            decision
                        }
                        .frame(width: sideBySide ? geo.size.width / 2 : nil, height: sideBySide ? nil : geo.size.height / 2)
                        .background(Palette.paperRaised.opacity(0.5))
                    }
                } else {
                    ScrollView { details.padding(.bottom, 12) }
                        .scrollDismissesKeyboard(.interactively)
                        .safeAreaInset(edge: .bottom, spacing: 0) {
                            // The glass buttons float over the details; a fade above them and a fill underneath keep both
                            // readable, however tall the decision is (the shortcut and its caption sit above the buttons).
                            decision.background {
                                VStack(spacing: 0) {
                                    LinearGradient(
                                        colors: [Palette.paper.opacity(0), Palette.paper],
                                        startPoint: .top,
                                        endPoint: .bottom
                                    )
                                    .frame(height: 24)
                                    Palette.paper
                                }
                                .padding(.top, -24)
                                .ignoresSafeArea()
                                .allowsHitTesting(false)
                            }
                        }
                }
            }
        }
        .background(Palette.paper.ignoresSafeArea())
        .overlay(alignment: .topTrailing) {
            if !closeInField {
                GlassIconButton(symbol: "xmark", size: 36, label: "Close") { model.closeSheet() }
                    .padding(.top, 14)
                    .padding(.trailing, 14)
                    .accessibilityIdentifier("closeSheet")
            }
        }
    }
}

/// A sheet's ask as a field (Reins Signal, iOS-Connect): the close button, the label line and Reins' own sentence in
/// the serif. Reins alone picks the field and the words: a name the other side sent never goes here, only quoted on a
/// row of its own below.
struct SheetField: View {
    var kind: FieldKind
    var ask: AskLine
    @Environment(AppModel.self) private var model

    var body: some View {
        SignalField(kind: kind) {
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Spacer()
                    RoundHeaderButton(glyph: .close, label: "Close", onField: true, size: 36) { model.closeSheet() }
                        .accessibilityIdentifier("closeSheet")
                }
                AskHead(ask: ask, field: kind)
            }
            .padding(.horizontal, 22)
            .padding(.top, 16)
            .padding(.bottom, 26)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hero")
    }
}

/// Deny in outline and the action in ink with the phone's biometry, at the bottom of a sheet (the approval dock's
/// buttons).
struct DecisionBar: View {
    var denyTitle = "Deny"
    var approveTitle = "Approve"
    /// Accent for a permission that stays (Allow); the text colour for a one-off approval.
    var accent = false
    var busy = false
    var approveEnabled = true
    /// On the kit's dock (Next-Connect 05): paper with round top corners and a hairline ring, as under an ask.
    var docked = false
    var onDeny: () -> Void
    var onApprove: () -> Void
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        if docked {
            buttons
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
        } else {
            buttons
                .padding(.horizontal, 20)
                .padding(.top, 10)
                .padding(.bottom, 10)
        }
    }

    @ViewBuilder private var buttons: some View {
        if docked && typeSize.isAccessibilitySize {
            // Approve over Deny, full width (Next-AX).
            VStack(spacing: 10) {
                approve.buttonStyle(SignalButtonLook(kind: accent ? .violet : .inverse, height: 64))
                deny.buttonStyle(SignalButtonLook(kind: .outline, height: 64))
            }
            .disabled(busy)
        } else {
            RatioRow(spacing: 10, ratio: 1 / 1.35) {
                deny.buttonStyle(SignalButtonLook(kind: .outline))
                approve.buttonStyle(SignalButtonLook(kind: accent ? .violet : .inverse))
            }
            .disabled(busy)
            .frame(height: 54)
        }
    }

    private var deny: some View {
        Button(action: onDeny) { Text(denyTitle) }
            .accessibilityIdentifier("deny")
    }

    private var approve: some View {
        Button(action: onApprove) {
            HStack(spacing: 8) {
                if busy {
                    ProgressView().tint(accent ? .white : Palette.onInverse)
                } else {
                    BiometryMark()
                }
                Text(approveTitle).lineLimit(1)
            }
        }
        .disabled(!approveEnabled)
        .accessibilityIdentifier("approve")
    }
}

/// Two views side by side, the first `ratio` times as wide as the second (Deny is narrower than Approve), as tall as
/// the taller one needs.
struct RatioRow: Layout {
    var spacing: CGFloat
    var ratio: CGFloat

    private func widths(_ total: CGFloat) -> (CGFloat, CGFloat) {
        let free = max(total - spacing, 0)
        let first = free * ratio / (1 + ratio)
        return (first, free - first)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 360
        let (a, b) = widths(width)
        let heights = zip(subviews, [a, b]).map { $0.sizeThatFits(ProposedViewSize(width: $1, height: nil)).height }
        return CGSize(width: width, height: heights.max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let (a, b) = widths(bounds.width)
        var x = bounds.minX
        for (view, w) in zip(subviews, [a, b]) {
            view.place(at: CGPoint(x: x, y: bounds.midY), anchor: .leading, proposal: ProposedViewSize(width: w, height: bounds.height))
            x += w + spacing
        }
    }
}

/// Who asks and what, at the top of a sheet: the connection, when, the operation's tile and its name.
struct DecisionHeader<Extra: View>: View {
    var connectionId: String
    var label: String
    /// Under the name: when it was asked, or the client's host.
    var subtitle: String
    var monoSubtitle = false
    var kind: ActionKind
    var count: Int = 1
    var title: String
    /// A new client (a pairing, an upload) has no connection yet, so no icon was picked for it.
    var known = true
    @ViewBuilder var extra: Extra

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                if known {
                    ConnectionIcon(connectionId: connectionId, label: label, size: 40)
                } else {
                    ConnectionAvatar(label: untrusted(label), pick: nil, size: 40)
                }
                VStack(alignment: .leading, spacing: 1) {
                    Text(untrusted(label)).font(RFont.sans(17, .semibold)).foregroundStyle(Palette.ink).lineLimit(1)
                    Text(subtitle)
                        .font(monoSubtitle ? RFont.mono(13) : RFont.sans(12.5))
                        .foregroundStyle(monoSubtitle ? Palette.ink2 : Palette.inkMeta)
                        .lineLimit(1)
                }
            }
            HStack(spacing: 14) {
                ActionTile(kind: kind, count: count, size: 48)
                Text(title)
                    .font(RFont.sans(26, .semibold))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(2)
                    .minimumScaleFactor(0.8)
                    .accessibilityIdentifier("what")
                    .accessibilityAddTraits(.isHeader)
            }
            .padding(.top, 16)
            extra
        }
        .padding(.leading, 20)
        .padding(.trailing, 60)
        .padding(.top, 22)
        .padding(.bottom, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The phone's owner confirms an approval first (Face ID, Touch ID or the passcode). Fails closed: anything but a
/// confirmation approves nothing.
enum OwnerPrompt {
    enum Result { case confirmed, cancelled, unavailable }

    @MainActor
    static func confirm(_ model: AppModel, reason: String) async -> Result {
        if await model.authenticator.confirm(reason) { return .confirmed }
        var error: NSError?
        return LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: &error) ? .cancelled : .unavailable
    }

    static let unavailableMessage = "Set a passcode on this iPhone to approve. Approving always needs Face ID, Touch ID or the passcode."
}

/// What went wrong answering, in words: a request answered elsewhere or gone says so.
func decisionErrorMessage(_ error: Error) -> String {
    if case CoreError.NotFound = error { return "That request is no longer waiting." }
    if case let CoreError.Server(status, _) = error, status == 403 { return "Another phone is your approval device now." }
    return error.userMessage
}

/// While the sheet is loading or could not load.
struct DecisionLoading: View {
    var error: String?
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 18) {
            if let error {
                Banner(error, kind: .error)
                Button("Close") { model.closeSheet() }
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 48))
                    .frame(maxWidth: 220)
            } else {
                ProgressView().controlSize(.large).padding(.top, 40)
            }
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Palette.paper.ignoresSafeArea())
    }
}

/// A quoted line from the AI ("Claude says it is for").
struct QuoteBox: View {
    var caption: String
    var text: String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Caption(caption)
            Text("“\(text)”").font(RFont.sans(15)).foregroundStyle(Palette.ink).lineSpacing(2)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}
