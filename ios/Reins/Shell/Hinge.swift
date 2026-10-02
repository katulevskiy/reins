import SwiftUI
import UIKit

/// The fold of an iPhone Duo, as SwiftUI sees it. `.flat` on phones and iPads without a hinge.
enum HingePosture: Equatable {
    /// No hinge, or fully open: one flat screen.
    case flat
    /// Partly open like a book or a laptop: content should stay off the crease.
    case bent
    /// Closed: only the outer screen is in use.
    case closed
}

private struct HingePostureKey: EnvironmentKey {
    static let defaultValue: HingePosture = .flat
}

extension EnvironmentValues {
    var hinge: HingePosture {
        get { self[HingePostureKey.self] }
        set { self[HingePostureKey.self] = newValue }
    }
}

/// Reads the hinge through UIKit's `UIHingeInteraction` (iOS 27.1) and publishes it to the views inside.
struct HingeReader<Content: View>: View {
    @State private var posture: HingePosture = .flat
    @ViewBuilder var content: (HingePosture) -> Content

    var body: some View {
        content(posture)
            .environment(\.hinge, posture)
            .background(HingeProbe(posture: $posture).allowsHitTesting(false))
    }
}

private struct HingeProbe: UIViewRepresentable {
    @Binding var posture: HingePosture

    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        view.isUserInteractionEnabled = false
        if #available(iOS 27.1, *) {
            let binding = $posture
            view.addInteraction(UIHingeInteraction { _, update in
                let next: HingePosture
                switch update.hinge?.status {
                case .partiallyOpen: next = .bent
                case .closed: next = .closed
                default: next = .flat
                }
                if binding.wrappedValue != next {
                    withAnimation(.smooth) { binding.wrappedValue = next }
                }
            })
        }
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {}
}
