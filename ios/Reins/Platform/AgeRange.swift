import UIKit
#if canImport(DeclaredAgeRange)
import DeclaredAgeRange
#endif

/// What the phone's family features say about who uses it (04 §4: a minor's mark is confirmed "through the platform's
/// family features where available"). Only a hint beside the ward's own acceptance; `.unavailable` never blocks.
enum AgeSignal: Equatable, Sendable {
    /// 18 or over, as the platform says. The core refuses a minor's mark with it.
    case adult
    /// Under 18; `upper` is the top of the range the platform gave (15 for 13–15), when it gave one.
    case minor(upper: Int?)
    /// No answer: declined, not shared, not supported, an error.
    case unavailable

    /// The age gates asked of the platform: 13, 16 and 18 (under 18 is a minor's).
    static let adultAge = 18

    /// A platform's range as a signal: an upper bound under 18 is a minor, a lower bound of 18 or more an adult; no
    /// range is no signal.
    static func of(lower: Int?, upper: Int?) -> AgeSignal {
        if let upper, upper < adultAge { return .minor(upper: upper) }
        if let lower, lower >= adultAge { return .adult }
        return .unavailable
    }

    /// What the core is handed with the ward's acceptance.
    var input: AgeSignalInput {
        switch self {
        case .adult: .adult
        case let .minor(upper): .minor(upper: upper.map { UInt8(clamping: $0) }, platform: .apple)
        case .unavailable: .unavailable
        }
    }
}

/// Asks the platform for an age signal. Never throws: anything that goes wrong is `.unavailable`.
protocol AgeSignals: Sendable {
    func check() async -> AgeSignal
}

/// A fixed answer (tests, the demo, and phones without the framework).
struct FixedAgeSignals: AgeSignals {
    var signal: AgeSignal = .unavailable

    func check() async -> AgeSignal { signal }
}

/// Apple's Declared Age Range (iOS 26): the range the person (or their family organiser) chose to share, at the gates
/// 13, 16 and 18. Declined, not available (no entitlement, an unsupported region, no account), an invalid request or
/// any other error is `.unavailable`, which never blocks.
struct DeclaredAgeSignals: AgeSignals {
    func check() async -> AgeSignal { await Self.ask() }

    @MainActor
    private static func ask() async -> AgeSignal {
        #if canImport(DeclaredAgeRange)
        guard #available(iOS 26.0, *), let presenter = presenter() else { return .unavailable }
        do {
            switch try await AgeRangeService.shared.requestAgeRange(ageGates: 13, 16, 18, in: presenter) {
            case .declinedSharing:
                return .unavailable
            case let .sharing(range):
                return AgeSignal.of(lower: range.lowerBound, upper: range.upperBound)
            @unknown default:
                return .unavailable
            }
        } catch {
            return .unavailable
        }
        #else
        return .unavailable
        #endif
    }

    /// The view controller in front, for the system's sheet.
    @MainActor
    private static func presenter() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let front = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first { $0.activationState == .foregroundInactive }
        var top = (front?.keyWindow ?? front?.windows.first)?.rootViewController
        while let next = top?.presentedViewController { top = next }
        return top
    }
}
