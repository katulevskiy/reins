import SwiftUI
import UIKit
import XCTest
@testable import Reins

/// The frosted glass over the wallpaper: it is a copy of the art drawn now, not of the first art the app drew.
@MainActor
final class FrostedGlassTests: XCTestCase {
    private func solid(_ color: UIColor, width: Int = 78, height: Int = 100) throws -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: width, height: height), format: format).image { ctx in
            color.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: width, height: height))
        }
        return try XCTUnwrap(image.cgImage.map { UIImage(cgImage: $0) })
    }

    private func pixel(_ image: UIImage, _ x: Int, _ y: Int) throws -> (r: Int, g: Int, b: Int) {
        let cg = try XCTUnwrap(image.cgImage)
        let px = try XCTUnwrap(Pixels(fill: cg, width: cg.width, height: cg.height))
        let i = px.index(x, y)
        return (Int(px.bytes[i]), Int(px.bytes[i + 1]), Int(px.bytes[i + 2]))
    }

    /// The art as the wallpaper layer draws it: a fixed frame whatever the art, so a new art never moves it.
    private struct Probe: View {
        let key: String
        let image: UIImage
        let backdrop: Backdrop

        var body: some View {
            Image(uiImage: image)
                .resizable()
                .frame(width: 156, height: 200)
                .backdropArt(key: key, image: image, dark: true, backdrop: backdrop)
        }
    }

    private func settle(_ host: UIViewController) {
        for _ in 0..<5 {
            host.view.setNeedsLayout()
            host.view.layoutIfNeeded()
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
    }

    func testANewArtInTheSamePlaceGetsANewFrostedCopy() throws {
        let backdrop = Backdrop()
        let red = try solid(.red)
        let blue = try solid(.blue)
        let host = UIHostingController(rootView: Probe(key: "dark_halo_original_f0", image: red, backdrop: backdrop))
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 480))
        window.rootViewController = host
        window.makeKeyAndVisible()
        defer { window.isHidden = true }
        settle(host)

        let first = try XCTUnwrap(backdrop.blurred, "the art registers when it appears")
        let frame = backdrop.frame
        let reddish = try pixel(first, Int(first.size.width) / 2, 2)
        XCTAssertGreaterThan(reddish.r, reddish.b)

        // Another choice drawn in the same frame (as a new image, effect, fade or photo is).
        host.rootView = Probe(key: "dark_crescent_dither_f30", image: blue, backdrop: backdrop)
        settle(host)

        let second = try XCTUnwrap(backdrop.blurred)
        XCTAssertFalse(first === second, "the glass copies the art drawn now")
        XCTAssertEqual(backdrop.frame, frame, "nothing moved")
        let bluish = try pixel(second, Int(second.size.width) / 2, 2)
        XCTAssertGreaterThan(bluish.b, bluish.r)
    }
}
