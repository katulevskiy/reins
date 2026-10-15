import SwiftUI

// The fold (Motion.png): after an answer, the field folds into its own kind tile and lands at the top of the river, so
// you see where your decision went. An approximation drawn over Activity: no real view travels from the approval's full
// screen to the list. A grained rounded rectangle in the field's colour starts at the screen's size and shrinks onto
// the kind tile of the entry that decision wrote (360 ms, ease out), then fades into it; the row then glows faintly in
// its kind colour for 4 s. If no entry shows up within 1.5 s, or it is not on screen, the field only fades. Reduce
// Motion: a 150 ms cross-fade. Silent: the decision itself played its feedback. It never takes a touch.

/// A decision, as the fold needs it.
struct FoldRequest: Equatable {
    var field: FieldKind
    var connectionId: String
    var action: String
    /// The newest Activity entry when it was decided: the decision's entry comes after it.
    var after: Int64
    /// Decided on the approval's full screen (the fold starts at the screen's size), else on a card (it only glows).
    var fromSheet: Bool
    var at = Date()
}

@Observable
@MainActor
final class FoldState {
    /// A decision waiting for its entry.
    private(set) var request: FoldRequest?
    /// The fold in the air: the field and the entry whose tile it lands on (nil: it fades where it is).
    private(set) var flight: (field: FieldKind, target: Int64?, id: UUID)?
    /// The row that glows, and until when.
    private(set) var glow: (id: Int64, until: Date)?

    /// A decision was taken. A burst answers several at once; one fold is enough.
    func decided(_ request: FoldRequest) {
        if let current = self.request, request.at.timeIntervalSince(current.at) < 0.5 { return }
        self.request = request
    }

    /// The entry the decision wrote: the newest one after it with the same connection and action, else any newer one
    /// of that connection.
    static func target(_ request: FoldRequest, in entries: [ActivityEntry]) -> ActivityEntry? {
        let newer = entries.filter { $0.id > request.after && $0.connectionId == request.connectionId }
        return newer.first { $0.action == request.action } ?? newer.first
    }

    /// Waits for the decision's entry (at most 1.5 s) and the approval's screen to go, then folds.
    func run(_ model: AppModel) async {
        guard let request else { return }
        var target: ActivityEntry?
        let deadline = request.at.addingTimeInterval(1.5)
        while Date() < deadline {
            target = Self.target(request, in: model.activity)
            if target != nil && model.sheet == nil { break }
            try? await Task.sleep(for: .milliseconds(100))
            if Task.isCancelled { return }
        }
        while model.sheet != nil && Date() < request.at.addingTimeInterval(4) {
            try? await Task.sleep(for: .milliseconds(100))
        }
        self.request = nil
        if request.fromSheet { flight = (request.field, target?.id, UUID()) }
        if let target { glow = (target.id, Date().addingTimeInterval(4)) }
    }

    func landed() { flight = nil }

    func glowing(_ id: Int64, now: Date = Date()) -> Bool {
        guard let glow else { return false }
        return glow.id == id && now < glow.until
    }
}

/// Where each river row's kind tile is, for the fold to land on.
struct FoldTileAnchors: PreferenceKey {
    static var defaultValue: [Int64: Anchor<CGRect>] { [:] }

    static func reduce(value: inout [Int64: Anchor<CGRect>], nextValue: () -> [Int64: Anchor<CGRect>]) {
        value.merge(nextValue()) { $1 }
    }
}

extension View {
    /// Marks the kind tile of entry `id` as a place the fold can land.
    func foldTile(_ id: Int64) -> some View {
        anchorPreference(key: FoldTileAnchors.self, value: .bounds) { [id: $0] }
    }

    /// Draws the fold over this view (Activity) while one is in the air.
    func foldOverlay(_ fold: FoldState) -> some View {
        overlayPreferenceValue(FoldTileAnchors.self) { anchors in
            GeometryReader { geo in
                if let flight = fold.flight {
                    FoldLayer(
                        field: flight.field,
                        start: CGRect(origin: .zero, size: geo.size).insetBy(dx: 0, dy: -geo.safeAreaInsets.top),
                        target: flight.target.flatMap { anchors[$0] }.map { geo[$0] },
                        onLanded: fold.landed
                    )
                    .id(flight.id)
                }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

/// The field in the air: from the screen to the tile, then gone.
private struct FoldLayer: View {
    var field: FieldKind
    var start: CGRect
    var target: CGRect?
    var onLanded: () -> Void
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var folded = false
    @State private var gone = false

    var body: some View {
        let end = target ?? start
        let rect = folded && !reduceMotion ? end : start
        let radius: CGFloat = folded && !reduceMotion ? (target == nil ? 30 : 11) : 30
        FieldSurface(kind: field)
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
            .frame(width: rect.width, height: rect.height)
            .position(x: rect.midX, y: rect.midY)
            .opacity(gone ? 0 : (reduceMotion || target == nil ? (folded ? 0 : 1) : 1))
            .task {
                if reduceMotion {
                    withAnimation(.easeInOut(duration: Motion.fade)) { folded = true }
                    try? await Task.sleep(for: .seconds(Motion.fade))
                } else {
                    withAnimation(Motion.easeOut(Motion.fold)) { folded = true }
                    try? await Task.sleep(for: .seconds(Motion.fold))
                    withAnimation(.easeOut(duration: Motion.fade)) { gone = true }
                    try? await Task.sleep(for: .seconds(Motion.fade))
                }
                onLanded()
            }
    }
}
