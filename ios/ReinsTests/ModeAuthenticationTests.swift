import AppIntents
import XCTest
@testable import Reins

/// Every control that loosens Autopilot from outside the app (Shortcuts, Control Center, Live Activity buttons) keeps
/// the system's owner check: the phone must be unlocked with Face ID, Touch ID or the passcode first.
final class ModeAuthenticationTests: XCTestCase {
    private func requiresAuthentication(_ policy: IntentAuthenticationPolicy) -> Bool {
        if case .requiresAuthentication = policy { return true }
        if case .requiresLocalDeviceAuthentication = policy { return true }
        return false
    }

    func testIntentsThatLeaveLockdownOrChangeTheModeNeedTheOwner() {
        XCTAssertTrue(requiresAuthentication(EndLockdownIntent.authenticationPolicy))
        XCTAssertTrue(requiresAuthentication(SetLockdownIntent.authenticationPolicy))
        XCTAssertTrue(requiresAuthentication(SetAutopilotModeIntent.authenticationPolicy))
        XCTAssertTrue(requiresAuthentication(ApproveQuickIntent.authenticationPolicy))
    }
}
