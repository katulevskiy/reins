import XCTest

extension XCUIApplication {
    /// One of the four places: its tab in the floating tab bar on a phone, its sidebar row on a regular width.
    /// `name` is "activity", "trust", "connections" or "settings".
    func tab(_ name: String) -> XCUIElement { descendants(matching: .any)["tab:\(name)"].firstMatch }

    /// Opens a place; false when its tab never showed.
    @discardableResult
    func openTab(_ name: String, timeout: TimeInterval = 10) -> Bool {
        let tab = tab(name)
        guard tab.waitForExistence(timeout: timeout) else { return false }
        tab.tap()
        return true
    }
}
