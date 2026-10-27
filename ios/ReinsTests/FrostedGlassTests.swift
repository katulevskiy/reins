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

    /// The shipped art's size: black, with a white stripe a quarter of its width down the middle.
    private func stripe() throws -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let image = UIGraphicsImageRenderer(size: CGSize(width: 780, height: 1000), format: format).image { ctx in
            UIColor.black.setFill()
            ctx.fill(CGRect(x: 0, y: 0, width: 780, height: 1000))
            UIColor.white.setFill()
            ctx.fill(CGRect(x: 292, y: 0, width: 196, height: 1000))
        }
        return try XCTUnwrap(image.cgImage.map { UIImage(cgImage: $0) })
    }

    func testTheBlurIsAndroidsShareOfTheArtsWidth() {
        // Android: σ ≈ 2.04 px on its 1/12 copy of the 780-px art (see `Backdrop.blurShare`).
        XCTAssertEqual(Backdrop.blurShare * 780, 2.04 * 12, accuracy: 0.2)
        XCTAssertEqual(Backdrop.blurShare * 195, 6.1, accuracy: 0.1, "the quarter copy's sigma")
    }

    func testShapesStayRecognisableThroughTheGlass() throws {
        let copy = try XCTUnwrap(Backdrop.blur(try stripe(), dark: true))
        XCTAssertEqual(copy.cgImage?.width, 195)
        XCTAssertEqual(copy.cgImage?.height, 250)
        let y = 100 // above the fade
        // The stripe is 49 px of the copy, its edges at 73 and 122: its middle keeps all of its light (white at 0.82),
        // 12 px (2 σ) outside it is nearly dark again. At the old σ 20 the middle was a quarter dimmer and the light
        // spread past its edges.
        XCTAssertGreaterThanOrEqual(try pixel(copy, 97, y).r, 195)
        XCTAssertLessThanOrEqual(try pixel(copy, 61, y).r, 30)
        XCTAssertLessThanOrEqual(try pixel(copy, 134, y).r, 30)
        XCTAssertLessThanOrEqual(try pixel(copy, 5, y).r, 4)
    }

    func testTheCopyFadesIntoThePaperAtItsFoot() throws {
        for dark in [true, false] {
            let copy = try XCTUnwrap(Backdrop.blur(try stripe(), dark: dark))
            let paper = WallpaperRender.paper(dark: dark)
            let height = try XCTUnwrap(copy.cgImage?.height)
            for x in [0, 61, 97, 134, 194] {
                let foot = try pixel(copy, x, height - 1)
                XCTAssertEqual(foot.r, Int(paper.r), "dark \(dark), x \(x): the last row is the page's paper")
                XCTAssertEqual(foot.g, Int(paper.g))
                XCTAssertEqual(foot.b, Int(paper.b))
            }
            // Above the fade the art shows as it is (the side unlike the paper: the stripe in dark, the black in light);
            // halfway down the fade it is between the two.
            let x = dark ? 97 : 5
            let above = try pixel(copy, x, Int(Double(height) * 0.6)).r - Int(paper.r)
            let half = try pixel(copy, x, Int(Double(height) * 0.85)).r - Int(paper.r)
            XCTAssertGreaterThan(abs(above), 150, "dark \(dark)")
            XCTAssertLessThan(abs(half), abs(above) * 3 / 4)
            XCTAssertGreaterThan(abs(half), abs(above) / 4)
        }
    }
}
