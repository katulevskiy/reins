import SwiftUI
import UIKit
import UIKit.UIGestureRecognizerSubclass

// Gestures for what sits inside a scroll view, made so they never take a scroll away from it.
//
// A SwiftUI DragGesture on scroll content takes part in UIKit's gesture resolution against the scroll view's own pan
// (iOS 18 and later): whichever recognises first wins. Depending on the first few points of a touch that was sometimes
// the row's drag, so the list did not move and the row stayed pressed. These are UIKit recognisers with their
// relationships stated instead: a sideways pan that begins only for clearly sideways movement, and a press that waits a
// moment before it is one, as the buttons of a scroll view do.

/// Which way a touch is going (pure, unit-tested).
enum SwipeAxis {
    /// How much more sideways than up or down a movement must be to be a swipe: within about 34° of horizontal.
    static let margin: CGFloat = 1.5

    /// Whether a touch that has just started to move is a sideways swipe: what it moved so far, or how it is moving now,
    /// sideways by a clear margin, and neither of them leaning up or down. Anything else belongs to the scroll view.
    static func sideways(translation t: CGPoint, velocity v: CGPoint) -> Bool {
        if abs(t.y) >= abs(t.x) || abs(v.y) > abs(v.x) { return false }
        return clearly(t) || clearly(v)
    }

    private static func clearly(_ p: CGPoint) -> Bool { abs(p.x) > 0 && abs(p.x) >= abs(p.y) * margin }
}

/// Where a sideways swipe is.
struct SidewaysDrag {
    /// How far the finger has moved since it touched down, in the view's space.
    var translation: CGSize
    /// Where the finger is, in the view's space.
    var location: CGPoint
}

/// A sideways swipe as a UIKit pan, for rows and cards in a scroll view: it begins only when the finger moves clearly
/// sideways, so an up-and-down movement fails it at once and stays the scroll view's. It is never held back by the
/// scroll view's pan, nor by the row's own press; once it begins it stops the list's pan for the rest of the touch, so
/// the list does not drift while a row slides. A tap that the swiping finger ends is not a tap: actions check
/// `SidewaysPan.swiping`.
struct SidewaysPan: UIGestureRecognizerRepresentable {
    var isEnabled = true
    var onChanged: (SidewaysDrag) -> Void = { _ in }
    /// The finger lifted, or the system took the touch (`cancelled`).
    var onEnded: (_ drag: SidewaysDrag, _ cancelled: Bool) -> Void

    @MainActor private static var active = false
    @MainActor private static var endedAt: CFTimeInterval = 0

    /// A sideways swipe holds the finger, or let go of it a moment ago: a row or button that sees that finger lift
    /// does nothing.
    @MainActor static var swiping: Bool { active || CACurrentMediaTime() - endedAt < 0.25 }

    func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator() }

    func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
        let pan = UIPanGestureRecognizer()
        pan.maximumNumberOfTouches = 1
        pan.delegate = context.coordinator
        pan.isEnabled = isEnabled
        return pan
    }

    func updateUIGestureRecognizer(_ pan: UIPanGestureRecognizer, context: Context) {
        pan.isEnabled = isEnabled
    }

    func handleUIGestureRecognizerAction(_ pan: UIPanGestureRecognizer, context: Context) {
        let t = context.converter.translation(in: .local) ?? .zero
        let drag = SidewaysDrag(translation: CGSize(width: t.x, height: t.y), location: context.converter.localLocation)
        switch pan.state {
        case .began:
            Self.active = true
            context.coordinator.stopScrolling(pan)
            onChanged(drag)
        case .changed:
            onChanged(drag)
        case .ended, .cancelled:
            Self.active = false
            Self.endedAt = CACurrentMediaTime()
            onEnded(drag, pan.state == .cancelled)
        default:
            break
        }
    }

    @MainActor
    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        /// The pan of the list this touch is in, met while UIKit settled who gets the touch.
        private weak var scrollPan: UIGestureRecognizer?

        func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
            let space = pan.view?.window
            return SwipeAxis.sideways(translation: pan.translation(in: space), velocity: pan.velocity(in: space))
        }

        func gestureRecognizer(
            _ recognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
        ) -> Bool {
            if Self.isUpAndDownList(other) {
                // An up-and-down list's pan and this one never block each other: up and down never begins this one,
                // and sideways stops the list itself (stopScrolling).
                scrollPan = other
                return true
            }
            // Other pans (swipe back, a list that scrolls sideways) wait for this one instead (below). The row's press,
            // tap and context menu neither block the swipe nor are blocked by it: they give way to a moving finger on
            // their own, and actions check `swiping` for the lift.
            return !(other is UIPanGestureRecognizer)
        }

        func gestureRecognizer(_ recognizer: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
            // A sideways swipe that starts on this view is this view's, as with a slider: swiping back (from the edge
            // or, on iOS 26, anywhere) and sideways lists wait until it fails, which it does at once for up and down.
            other is UIPanGestureRecognizer && !Self.isUpAndDownList(other)
        }

        /// A sideways swipe began: the list keeps still for the rest of this touch. Turning a recogniser off and on
        /// cancels it and leaves it out of the touch already going on.
        func stopScrolling(_ swipe: UIGestureRecognizer) {
            guard let pan = scrollPan ?? Self.enclosingList(of: swipe.view), pan.isEnabled else { return }
            pan.isEnabled = false
            pan.isEnabled = true
        }

        /// The up-and-down list a view is in, when UIKit did not ask about its pan.
        private static func enclosingList(of view: UIView?) -> UIGestureRecognizer? {
            var next = view
            while let v = next {
                if let scroll = v as? UIScrollView, isUpAndDownList(scroll.panGestureRecognizer) { return scroll.panGestureRecognizer }
                next = v.superview
            }
            return nil
        }

        /// The pan of a scroll view that cannot scroll sideways.
        private static func isUpAndDownList(_ recognizer: UIGestureRecognizer) -> Bool {
            guard let scroll = recognizer.view as? UIScrollView, recognizer === scroll.panGestureRecognizer else { return false }
            let inset = scroll.adjustedContentInset
            return scroll.contentSize.width + inset.left + inset.right <= scroll.bounds.width + 1
        }
    }
}

/// A press that waits a moment before it is one, as the buttons of a scroll view do. A finger that moves first is a
/// scroll, and this fails; one that lifts first is a tap (`.ended`, never `.began`); one that stays still for `delay`
/// begins the press (`.began`), which then follows the finger (`.changed`) until it lifts.
final class DelayedPressRecognizer: UIGestureRecognizer {
    /// About a scroll view's own wait before its content sees a touch.
    var delay: TimeInterval = 0.15
    /// How far the finger may wander before the press begins.
    var slop: CGFloat = 10
    /// The press began; a touch that ends without it was a tap.
    private(set) var held = false
    private var start: CGPoint?

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesBegan(touches, with: event)
        // A finger joining one already down is left out; two at once are not a press.
        guard start == nil else {
            for touch in touches { ignore(touch, for: event) }
            return
        }
        guard touches.count == 1, let touch = touches.first else {
            state = .failed
            return
        }
        // A touch that stops a list still moving only stops it, as with a button.
        if enclosingScrollMoving() {
            state = .failed
            return
        }
        start = touch.location(in: nil)
        perform(#selector(hold), with: nil, afterDelay: delay)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesMoved(touches, with: event)
        switch state {
        case .possible:
            guard let start, let point = touches.first?.location(in: nil) else { return }
            if hypot(point.x - start.x, point.y - start.y) > slop { state = .failed }
        case .began, .changed:
            state = .changed
        default:
            break
        }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesEnded(touches, with: event)
        switch state {
        case .possible, .began, .changed: state = .ended
        default: break
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        super.touchesCancelled(touches, with: event)
        state = state == .possible ? .failed : .cancelled
    }

    override func reset() {
        super.reset()
        NSObject.cancelPreviousPerformRequests(withTarget: self, selector: #selector(hold), object: nil)
        held = false
        start = nil
    }

    @objc private func hold() {
        guard state == .possible, start != nil else { return }
        held = true
        state = .began
    }

    private func enclosingScrollMoving() -> Bool {
        var next = view
        while let v = next {
            if let scroll = v as? UIScrollView, scroll.isDecelerating { return true }
            next = v.superview
        }
        return false
    }
}

/// A delayed press and slide in SwiftUI (`DelayedPressRecognizer`), with the finger's place in `space`.
struct DelayedPress<Space: CoordinateSpaceProtocol>: UIGestureRecognizerRepresentable {
    var space: Space
    /// The finger stayed still: the press begins here.
    var onPress: (CGPoint) -> Void
    /// The pressed finger moved.
    var onSlide: (CGPoint) -> Void
    /// The finger lifted: after a press (`held`), or as a tap.
    var onRelease: (_ location: CGPoint, _ held: Bool) -> Void
    /// The system took a press away.
    var onCancel: () -> Void

    func makeUIGestureRecognizer(context: Context) -> DelayedPressRecognizer { DelayedPressRecognizer() }

    func handleUIGestureRecognizerAction(_ press: DelayedPressRecognizer, context: Context) {
        let location = context.converter.location(in: space)
        switch press.state {
        case .began: onPress(location)
        case .changed: onSlide(location)
        case .ended: onRelease(location, press.held)
        case .cancelled: onCancel()
        default: break
        }
    }
}
