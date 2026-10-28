import Foundation
import Observation

/// Auto's setup (01 §9.1; the Android app's `AutoSetupViewModel`): your world sorted into zones, the matrix with your
/// names in it, then a hold that turns Auto on. Shown when Auto is picked for every AI before the setup was done
/// (`AutopilotModel.guidesToAutoSetup`). Every loosening (toward Mine, a cell toward Auto, turning on) is the owner's:
/// the core asks for the check itself; tightening needs nothing. Nets come later (P3).
@Observable
@MainActor
final class AutoSetupModel {
    enum Step: Int, CaseIterable {
        case world, matrix, turnOn
    }

    private(set) var setup: AutoSetup?
    private(set) var step: Step = .world
    private(set) var busy = false
    var error: String?
    /// Auto is on: the setup closes.
    private(set) var finished = false
    /// A cell about to get looser, with what it would have done on this phone's last 30 days (02 §4).
    private(set) var loosen: LoosenPreview?

    struct LoosenPreview: Equatable {
        var zone: ZoneChoice
        var level: Int
        var line: String
    }

    @ObservationIgnored private weak var app: AppModel?

    init(app: AppModel? = nil) {
        self.app = app
    }

    /// The screen makes its model before the environment is there; it hands the app model over on appear.
    func bind(_ app: AppModel) {
        if self.app !== app { self.app = app }
    }

    func load() async {
        guard let app else { return }
        do {
            setup = try await app.core.autoSetup()
        } catch {
            self.error = AutopilotModel.sentence(error.userMessage)
        }
    }

    // MARK: Steps

    func next() {
        guard let after = Step(rawValue: step.rawValue + 1) else { return }
        app?.feedback.play(.tap)
        error = nil
        step = after
    }

    /// One step back; false on the first (the screen closes then).
    @discardableResult
    func back() -> Bool {
        guard let before = Step(rawValue: step.rawValue - 1) else { return false }
        app?.feedback.play(.tap)
        error = nil
        step = before
        return true
    }

    /// Leaves without turning anything on (what was changed stays changed).
    func close() {
        app?.autoSetupOpen = false
    }

    // MARK: Your world

    /// GitHub, Email, Messages, Computers, each with what is in it (the empty ones left out).
    var sections: [AutoSetupText.Section] { AutoSetupText.sections(setup?.world ?? []) }

    /// A thing's zone. Toward Mine the core asks the owner first; a refusal changes nothing.
    func setZone(_ item: WorldItem, _ zone: ZoneChoice) async {
        guard let app, item.zone != zone || !item.confirmed else { return }
        await run {
            try await app.core.setWorldZone(kind: item.kind, key: item.key, zone: zone, owner: app.ownerCheck)
            app.feedback.play(.selection)
        }
        await app.refreshAutoQuestions()
    }

    // MARK: The matrix

    /// A tap on a cell: Auto, then Ask first, then Ask, then Auto again. "Can't be undone" always asks. Toward Auto the
    /// core asks the owner first.
    func cycle(_ zone: ZoneChoice, level: Int) async {
        guard let app, AutoSetupText.editable(level: level),
              let row = setup?.matrix.first(where: { $0.zone == zone }), level < row.cells.count else { return }
        let next = AutoSetupText.nextCell(row.cells[level])
        await run {
            try await app.core.setMatrixCell(zone: zone, level: UInt8(level), cell: next, owner: app.ownerCheck)
            app.feedback.play(.selection)
        }
    }

    /// A tap on a cell from the screen: a looser cell shows its preview first ([`loosen`]), then the owner is asked on
    /// Continue ([`confirmLoosen`]); a stricter one changes at once.
    func tap(_ zone: ZoneChoice, level: Int) async {
        guard let app, AutoSetupText.editable(level: level),
              let row = setup?.matrix.first(where: { $0.zone == zone }), level < row.cells.count else { return }
        let old = row.cells[level]
        let next = AutoSetupText.nextCell(old)
        guard AutoSetupText.strictness(next) < AutoSetupText.strictness(old) else {
            await cycle(zone, level: level)
            return
        }
        let line = await PolicyPreview.line(app.core, .setCell(zone: zone, level: UInt8(level), cell: next))
        loosen = LoosenPreview(zone: zone, level: level, line: line)
    }

    func confirmLoosen() async {
        guard let l = loosen else { return }
        loosen = nil
        await cycle(l.zone, level: l.level)
    }

    func cancelLoosen() {
        loosen = nil
    }

    // MARK: Turn on

    /// After the hold: the core asks the owner, keeps every proposal as shown and turns Auto on.
    func finish() async {
        guard let app, !busy else { return }
        let done = await run {
            try await app.core.finishAutoSetup(owner: app.ownerCheck)
        }
        await app.refreshAutopilot()
        guard done else { return }
        app.feedback.play(.autopilotOn)
        finished = true
        app.autoSetupOpen = false
        await app.refreshAutoQuestions()
    }

    // MARK: Running

    /// Runs `block`, then reads the setup again (also after a refusal, so nothing shows that did not happen). True when
    /// `block` went through.
    @discardableResult
    private func run(_ block: () async throws -> Void) async -> Bool {
        busy = true
        error = nil
        defer { busy = false }
        var ok = true
        do {
            try await block()
        } catch {
            ok = false
            app?.feedback.play(.error)
            self.error = AutopilotModel.sentence(error.userMessage)
        }
        await load()
        return ok
    }
}

/// Auto's setup in words (pure, unit-tested).
enum AutoSetupText {
    struct Section: Identifiable, Equatable {
        /// "github" | "email" | "messages" | "computers".
        var service: String
        var items: [WorldItem]
        var id: String { service }
        var title: String { AutoSetupText.serviceTitle(service) }
    }

    static let services = ["github", "email", "messages", "computers"]

    static func sections(_ world: [WorldItem]) -> [Section] {
        services.compactMap { service in
            let items = world.filter { $0.service == service }
                .sorted { $0.count != $1.count ? $0.count > $1.count : $0.label < $1.label }
            return items.isEmpty ? nil : Section(service: service, items: items)
        }
    }

    static func serviceTitle(_ service: String) -> String {
        switch service {
        case "github": "GitHub"
        case "email": "Email"
        case "messages": "Messages"
        case "computers": "Computers"
        default: service.capitalized
        }
    }

    /// An SF Symbol for a service's section.
    static func serviceSymbol(_ service: String) -> String {
        switch service {
        case "github": "chevron.left.forwardslash.chevron.right"
        case "email": "envelope"
        case "messages": "bubble.left.and.bubble.right"
        case "computers": "desktopcomputer"
        default: "circle.grid.2x2"
        }
    }

    /// An SF Symbol for what a thing is.
    static func kindSymbol(_ kind: FactChoice) -> String {
        switch kind {
        case .owner: "person.2"
        case .repo: "folder"
        case .domain: "at"
        case .person: "person"
        case .host: "desktopcomputer"
        case .chat: "bubble.left"
        }
    }

    static let zones: [ZoneChoice] = [.mine, .friends, .work, .outside]

    static func zoneName(_ zone: ZoneChoice) -> String {
        switch zone {
        case .mine: "Mine"
        case .friends: "Friends"
        case .work: "Work"
        case .outside: "Outside"
        }
    }

    static func zoneId(_ zone: ZoneChoice) -> String { zoneName(zone).lowercased() }

    /// The matrix's columns, L0 to L3.
    static let levels = ["Undoable", "Recoverable", "Reaches people", "Can't be undone"]

    /// Only "Can't be undone" (L3) is fixed: it always asks.
    static func editable(level: Int) -> Bool { (0..<3).contains(level) }

    /// Auto, then Ask first, then Ask, then Auto again.
    /// Higher is stricter: Auto, Ask first, Ask.
    static func strictness(_ cell: CellChoice) -> Int {
        switch cell {
        case .auto: 0
        case .askFirst: 1
        case .ask: 2
        }
    }

    static func nextCell(_ cell: CellChoice) -> CellChoice {
        switch cell {
        case .auto: .askFirst
        case .askFirst: .ask
        case .ask: .auto
        }
    }

    static func cellName(_ cell: CellChoice) -> String {
        switch cell {
        case .auto: "Auto"
        case .askFirst: "Ask first"
        case .ask: "Ask"
        }
    }

    /// A thing's name as the screen shows it: from outside, so cleaned and kept short.
    static func label(_ item: WorldItem) -> String { untrustedName(item.label.isEmpty ? item.key : item.label, limit: 40) }

    /// "dkat, acme": the names that make a row real, quiet under its zone.
    static func names(_ row: MatrixRowView) -> String {
        row.names.prefix(3).map { untrustedName($0, limit: 16) }.joined(separator: ", ")
    }

    /// The step's title.
    static func title(_ step: AutoSetupModel.Step) -> String {
        switch step {
        case .world: "Your world"
        case .matrix: "What Auto does"
        case .turnOn: "Turn on Auto"
        }
    }

    /// The (?) behind the matrix: one or two short lines.
    static let matrixHelp = "Tap a cell to change it. Giving Auto more asks it's you.\nWhat can't be undone always asks."
}
