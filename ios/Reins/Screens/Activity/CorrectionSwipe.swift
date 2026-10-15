import SwiftUI

/// "Correct in one swipe" (iOS-Activity.png, 04): a river row Autopilot decided can be swiped left to show "This was
/// wrong" in violet; letting go past it asks what should have happened, as the entry's page does, and Autopilot learns.
/// What was done stays done. Only rows the core says can still be corrected; others do not move.
struct CorrectionSwipe: ViewModifier {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var offset: CGFloat = 0
    @State private var sideways: Bool?
    @State private var asking = false

    /// How far the row opens.
    static let reveal: CGFloat = 116

    func body(content: Content) -> some View {
        if ThisWasWrongDock.offered(entry) {
            content
                .background(Palette.paper)
                .offset(x: offset)
                .background(alignment: .trailing) { action }
                .clipped()
                .simultaneousGesture(swipe)
                .accessibilityAction(named: "This was wrong") { asking = true }
                .alert(Correction.question(entry), isPresented: $asking) {
                    Button(Correction.answer(entry)) {
                        model.feedback.quietClose()
                        Task {
                            if let failed = await Correction.apply(model, entry: entry, shouldHave: correctionFor(entry)) {
                                model.notice = failed
                            } else {
                                model.notice = Correction.thanks
                            }
                        }
                    }
                    Button("Cancel", role: .cancel) {}
                } message: {
                    Text(Correction.message)
                }
                .presentationFeedback(asking)
        } else {
            content
        }
    }

    private var action: some View {
        Button {
            close()
            asking = true
        } label: {
            VStack(spacing: 4) {
                GlyphIcon(.undo, size: 18)
                Text("This was wrong").font(RFont.sans(12.5, .semibold)).multilineTextAlignment(.center)
            }
            .foregroundStyle(Palette.onField)
            .frame(width: Self.reveal)
            .frame(maxHeight: .infinity)
            .background(Palette.violetFill)
        }
        .buttonStyle(PressDimsStyle())
        .opacity(offset < 0 ? 1 : 0)
        .accessibilityHidden(true)
        .accessibilityIdentifier("swipeWrong:\(entry.id)")
    }

    private var swipe: some Gesture {
        DragGesture(minimumDistance: 14)
            .onChanged { drag in
                if sideways == nil { sideways = abs(drag.translation.width) > abs(drag.translation.height) }
                guard sideways == true else { return }
                let start: CGFloat = offset <= -Self.reveal / 2 ? -Self.reveal : 0
                offset = min(0, start + drag.translation.width)
            }
            .onEnded { drag in
                defer { sideways = nil }
                guard sideways == true else { return }
                if offset < -Self.reveal * 1.6 {
                    // A long swipe asks at once.
                    close()
                    asking = true
                } else {
                    withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) {
                        offset = offset < -Self.reveal / 2 ? -Self.reveal : 0
                    }
                }
            }
    }

    private func close() {
        withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) { offset = 0 }
    }
}

extension View {
    /// Swipe left on a river row Autopilot decided: "This was wrong".
    func correctionSwipe(_ entry: ActivityEntry) -> some View { modifier(CorrectionSwipe(entry: entry)) }
}
