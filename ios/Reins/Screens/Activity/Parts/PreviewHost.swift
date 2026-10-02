import SwiftUI

#if DEBUG
/// Wraps a preview in a signed-in model over the `-demo` core, so screens show the sample data.
struct PreviewHost<Content: View>: View {
    @ViewBuilder var content: Content
    @State private var model = AppModel(core: DemoRewardenCore(), feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)

    var body: some View {
        content
            .environment(model)
            .environment(\.feedback, model.feedback)
            .task {
                model.autoPopup = false
                await model.refreshSession()
            }
    }
}
#endif
