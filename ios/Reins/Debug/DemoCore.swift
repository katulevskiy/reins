import Foundation

/// The `-demo` launch argument's in-memory core (screenshots, UI tests, a look around without a server).
/// Placeholder until the fake core lands: nil means the real core is used.
enum DemoCore {
    static func make() -> (any RewardenCoreProtocol)? { nil }
}
