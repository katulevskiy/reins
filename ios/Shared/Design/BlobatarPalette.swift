import Foundation

/// The colours and shapes a connection's blobatar keeps by index (the core's `avatar::COLOURS` and `SHAPES`), in the
/// order of design/signal/tokens/blobatar-palette.json, which `BlobatarPaletteTests` checks this copy against.
enum BlobatarPalette {
    struct Entry: Equatable, Sendable {
        /// The fixture's name ("red"); `title` is what the app says.
        let name: String
        let colour: BlobatarArt.Colour

        var title: String { name.prefix(1).uppercased() + name.dropFirst() }
    }

    static let colours: [Entry] = [
        Entry(name: "red", colour: .init(l: 0.63, c: 0.21, h: 27.0)),
        Entry(name: "orange", colour: .init(l: 0.73, c: 0.17, h: 58.0)),
        Entry(name: "yellow", colour: .init(l: 0.85, c: 0.165, h: 95.0)),
        Entry(name: "lime", colour: .init(l: 0.80, c: 0.19, h: 130.0)),
        Entry(name: "green", colour: .init(l: 0.64, c: 0.16, h: 150.0)),
        Entry(name: "teal", colour: .init(l: 0.71, c: 0.12, h: 186.0)),
        Entry(name: "sky", colour: .init(l: 0.74, c: 0.13, h: 236.0)),
        Entry(name: "blue", colour: .init(l: 0.57, c: 0.19, h: 262.0)),
        Entry(name: "purple", colour: .init(l: 0.52, c: 0.22, h: 295.0)),
        Entry(name: "violet", colour: .init(l: 0.70, c: 0.17, h: 318.0)),
        Entry(name: "pink", colour: .init(l: 0.69, c: 0.20, h: 355.0)),
        Entry(name: "slate", colour: .init(l: 0.56, c: 0.035, h: 255.0)),
    ]

    /// The library's shapes, in its band order (`BlobatarArt.shapeNames`).
    static let shapes = ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"]

    /// Colour `index`, if there is one.
    static func colour(_ index: Int?) -> BlobatarArt.Colour? {
        guard let index, colours.indices.contains(index) else { return nil }
        return colours[index].colour
    }

    /// Shape `index`, if there is one.
    static func shape(_ index: Int?) -> Int? {
        guard let index, shapes.indices.contains(index) else { return nil }
        return index
    }
}

/// A connection's kept blobatar as widgets and the app draw it: the seed for the figure's details, its shape and its
/// colour (indexes into `BlobatarPalette`). Codable, so the widget snapshot carries it.
struct BlobatarLook: Codable, Hashable, Sendable {
    var seed: String
    var shape: Int
    var colour: Int
}
