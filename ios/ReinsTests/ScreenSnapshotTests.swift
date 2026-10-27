import SwiftUI
import UIKit
import XCTest
@testable import Reins

/// Renders the main screens with the demo core to PNG files, light and dark, at the Next boards' phone size (390×844,
/// design/next/iphone/previews), for people and agents to look at: `REINS_SNAPSHOTS=<dir>` in the test's environment
/// (`TEST_RUNNER_REINS_SNAPSHOTS` through `xcodebuild test`; scripts/ios-job.sh sets it). Skipped otherwise, so a normal
/// run pays nothing. Nothing is compared with a reference image: these never fail on a pixel, only when a screen
/// cannot be rendered at all. Like Android's ScreenshotsTest, with no UI-test app launch: each screen is a SwiftUI
/// view in a window of the test host, drawn after its data has loaded. One method per screen, so one can be run alone
/// (`-only-testing:ReinsTests/ScreenSnapshotTests/testApprove`).
///
/// Not ImageRenderer: it draws a placeholder for every view UIKit backs (ScrollView, NavigationStack, List, text
/// fields), which is most of a screen. Not swift-snapshot-testing either: what it adds over the window drawn here is
/// the reference-image comparison these deliberately do without.
@MainActor
final class ScreenSnapshotTests: XCTestCase {
    private var directory: URL!
    private var model: AppModel!
    private var core: DemoReinsCore!

    override func setUp() async throws {
        let path = ProcessInfo.processInfo.environment["REINS_SNAPSHOTS"] ?? ""
        try XCTSkipIf(path.isEmpty, "set REINS_SNAPSHOTS to a directory to render the screens")
        directory = URL(fileURLWithPath: path, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    override func tearDown() async throws {
        Timers.frozenNow = nil
        model = nil
        core = nil
    }

    // MARK: Screens (the boards they match)

    /// Next-Activity 01: a request waits on top, the day below.
    func testActivityWaiting() async throws {
        try await start()
        try await shoot("activity-waiting", MainShell())
    }

    /// Next-Activity 02: nothing waits.
    func testActivityQuiet() async throws {
        try await start(quiet: true)
        try await shoot("activity-quiet", MainShell())
    }

    /// Next-Approve 01: a push to octo/app, full screen.
    func testApprove() async throws {
        try await start()
        try await shoot("approve", SheetContent(target: .approval("req20"), regular: false))
    }

    /// Next-Trust: Manual or Auto, the grants.
    func testTrust() async throws {
        try await start()
        model.select(.trust)
        try await shoot("trust", MainShell())
    }

    /// Next-Autopilot 02: Bypass running, the field on Trust.
    func testAutopilot() async throws {
        try await start()
        try await core.setAutopilotMode(connectionId: nil, mode: .bypass, minutes: 60)
        _ = await model.refreshAutopilot()
        model.select(.trust)
        try await shoot("autopilot-bypass", MainShell())
    }

    /// Next-Autopilot 05: a profile's numbers.
    func testAutopilotProfile() async throws {
        try await start()
        model.show(.autopilotProfile("personal"), in: .trust)
        try await shoot("autopilot-profile", MainShell())
    }

    /// Next-Autopilot 06: Try it.
    func testTryIt() async throws {
        try await start()
        model.show(.tryIt(nil), in: .trust)
        try await shoot("autopilot-tryit", MainShell())
    }

    /// Next-Connections: AIs, computers, services.
    func testConnections() async throws {
        try await start()
        model.select(.connections)
        try await shoot("connections", MainShell())
    }

    /// Next-Vault: the vault's list.
    func testVault() async throws {
        try await start()
        model.show(.vault, in: .connections)
        try await shoot("vault", MainShell())
    }

    /// Next-Settings: the root.
    func testSettings() async throws {
        try await start()
        model.select(.settings)
        try await shoot("settings", MainShell())
    }

    #if DEBUG
    /// Next-System: the kit, one board-sized page after another (`system-1`, `system-2`, ...). One tall image would pass
    /// the renderer's texture limit and come out blank.
    func testSystem() async throws {
        try await start()
        let visible = Board.size.height - Board.insets.top - Board.insets.bottom
        let fit = UIHostingController(rootView: SystemGalleryPage().environment(model))
            .sizeThatFits(in: CGSize(width: Board.size.width, height: 100_000))
        let pages = min(max(Int((fit.height / visible).rounded(.up)), 1), 12)
        for page in 0..<pages {
            let view = SystemGalleryPage()
                .fixedSize(horizontal: false, vertical: true)
                .offset(y: -CGFloat(page) * visible)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .clipped()
                .pageBackground()
            try await shoot("system-\(page + 1)", view)
        }
    }
    #endif

    // MARK: Rendering

    /// The demo account, signed in, with its data loaded and the clocks stopped at the moment it was seeded.
    private func start(quiet: Bool = false) async throws {
        core = DemoReinsCore(signedIn: true, syncCap: 0.3, quiet: quiet)
        Timers.frozenNow = TimeInterval(DemoReinsCore.now())
        // Recovery was set up long ago, as `-demo` assumes (DemoCore.make).
        AppGroup.defaults.set(true, forKey: RecoveryRecord.key(server: DemoData.server, code: DemoData.recoveryCode))
        model = AppModel(core: core, feedback: NoFeedback.shared, authenticator: TrustingAuthenticator(), demo: true)
        // A waiting request would otherwise open its own sheet over Activity.
        model.autoPopup = false
        await model.refreshSession()
        await model.refreshPending()
        await model.refreshConnections()
        _ = await model.refreshAutopilot()
    }

    /// Light and dark PNGs of `view`: `<name>-light.png`, `<name>-dark.png`.
    private func shoot(_ name: String, _ view: some View) async throws {
        for style in [UIUserInterfaceStyle.light, .dark] {
            let image = try await render(view.environment(model).environment(\.feedback, model.feedback), style: style)
            let file = directory.appendingPathComponent("\(name)-\(style == .dark ? "dark" : "light").png")
            XCTAssertFalse(Self.isBlank(image), "\(name) rendered as one flat colour: nothing was drawn")
            let data = try XCTUnwrap(image.pngData(), "\(name) did not encode")
            try data.write(to: file)
            print("snapshot \(file.path)")
        }
    }

    private func render(_ view: some View, style: UIUserInterfaceStyle) async throws -> UIImage {
        let scene = try XCTUnwrap(
            UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first,
            "the test host has no window scene"
        )
        let size = Board.size
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(origin: .zero, size: size)
        window.windowLevel = .alert + 1
        window.overrideUserInterfaceStyle = style
        let host = UIHostingController(rootView: AnyView(view.toggleStyle(SignalSwitchStyle()).tint(Palette.violet)))
        host.view.backgroundColor = .systemBackground
        window.rootViewController = host
        window.isHidden = false
        defer {
            window.isHidden = true
            window.rootViewController = nil
        }
        window.layoutIfNeeded()
        // The boards' phone: status bar and home indicator insets, whatever simulator runs the test.
        let current = window.safeAreaInsets
        host.additionalSafeAreaInsets = UIEdgeInsets(
            top: Board.insets.top - current.top, left: -current.left,
            bottom: Board.insets.bottom - current.bottom, right: -current.right
        )
        // `.task`s load their data, sheets and pushed pages land, first animations end.
        try await Task.sleep(for: .milliseconds(1_500))
        window.layoutIfNeeded()
        let format = UIGraphicsImageRendererFormat()
        format.scale = Board.scale
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: size, format: format).image { _ in
            _ = window.drawHierarchy(in: CGRect(origin: .zero, size: size), afterScreenUpdates: true)
        }
        return image
    }

    /// Every sampled pixel the same: the window drew nothing (a texture too large, a view that never appeared).
    private static func isBlank(_ image: UIImage) -> Bool {
        guard let cg = image.cgImage, let data = cg.dataProvider?.data, let bytes = CFDataGetBytePtr(data) else { return true }
        let pixel = max(cg.bitsPerPixel / 8, 1)
        let length = CFDataGetLength(data)
        var offset = pixel * 97
        while offset + pixel <= length {
            for byte in 0..<pixel where bytes[offset + byte] != bytes[byte] { return false }
            offset += pixel * 97
        }
        return true
    }

    private enum Board {
        /// An iPhone 13/14/16e: the size the Next boards draw.
        static let size = CGSize(width: 390, height: 844)
        static let insets = UIEdgeInsets(top: 47, left: 0, bottom: 34, right: 0)
        static let scale: CGFloat = 2
    }
}
