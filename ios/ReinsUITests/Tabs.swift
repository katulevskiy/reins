import XCTest

extension XCUIApplication {
    /// One of the four places: its tab in the floating tab bar on a phone, its sidebar row on a regular width.
    /// `name` is "activity", "trust", "connections" or "settings".
    func tab(_ name: String) -> XCUIElement { descendants(matching: .any)["tab:\(name)"].firstMatch }

    /// Opens a place; false when its tab never showed. The tab bar shows at a place's root only, so a pushed page is
    /// left by its back button first, as a person would.
    @discardableResult
    func openTab(_ name: String, timeout: TimeInterval = 10) -> Bool {
        let tab = tab(name)
        var backs = 0
        while !tab.waitForExistence(timeout: backs == 0 ? 2 : 1) && backs < 4 {
            let back = navigationBars.buttons.firstMatch
            guard back.exists else { break }
            back.tap()
            backs += 1
        }
        guard tab.waitForExistence(timeout: timeout) else { return false }
        tab.tap()
        return true
    }
}
