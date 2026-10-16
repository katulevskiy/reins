import SwiftUI

/// "Correct in one swipe" (iOS-Activity.png, 04): a river row Autopilot decided can be swiped left to show "This was
/// wrong" in violet; letting go past it asks what should have happened, as the entry's page does, and Autopilot learns.
/// What was done stays done. Only rows the core says can still be corrected; others do not move.
struct CorrectionSwipe: ViewModifier {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var offset: CGFloat = 0
    /// A sideways swipe is going on: the row does not look pressed while it slides.
    @State private var swiping = false
    /// Where this swipe started: shut (0) or open (-reveal), read once when it begins. Read on every move instead, a
    /// row that passed half open mid-swipe jumped to the open place plus the finger's travel, and a short swipe asked.
    @State private var swipeStart: CGFloat?
    @State private var asking = false

    /// How far the row opens.
    static let reveal: CGFloat = 116

    func body(content: Content) -> some View {
        if ThisWasWrongDock.offered(entry) {
            content
                .disabled(swiping)
                .background(Palette.paper)
                .offset(x: offset)
                .background(alignment: .trailing) { action }
                .clipped()
                .gesture(swipe)
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
            // Not the finger that swiped the row shut, lifting on it.
            guard !SidewaysPan.swiping else { return }
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

    /// Sideways only (SidewaysPan): a touch that moves up or down is the list's from its first points.
    private var swipe: SidewaysPan {
        SidewaysPan(onChanged: { drag in
            swiping = true
            let start = swipeStart ?? (offset <= -Self.reveal / 2 ? -Self.reveal : 0)
            swipeStart = start
            offset = min(0, start + drag.translation.width)
        }, onEnded: { _, cancelled in
            swiping = false
            swipeStart = nil
            if !cancelled && offset < -Self.reveal * 1.6 {
                // A long swipe asks at once.
                close()
                asking = true
            } else {
                withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) {
                    offset = offset < -Self.reveal / 2 ? -Self.reveal : 0
                }
            }
        })
    }

    private func close() {
        withAnimation(Motion.of(Motion.cardSpring(0.3), reduce: reduceMotion)) { offset = 0 }
    }
}

extension View {
    /// Swipe left on a river row Autopilot decided: "This was wrong".
    func correctionSwipe(_ entry: ActivityEntry) -> some View { modifier(CorrectionSwipe(entry: entry)) }
}
