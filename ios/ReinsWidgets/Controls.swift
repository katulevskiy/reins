import AppIntents
import SwiftUI
import WidgetKit

// Control Center, Lock Screen and Action button controls. Their values come from the snapshot; the app reloads them
// with the widgets whenever it writes one.

/// Lockdown on or off. The intent runs in the app and needs an unlocked phone.
struct LockdownControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.lockdown"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: Provider()) { on in
            ControlWidgetToggle("Lockdown", isOn: on, action: SetLockdownIntent()) { isOn in
                Label(isOn ? "On" : "Off", systemImage: isOn ? "lock.fill" : "lock.open")
            }
            .tint(Palette.warning)
        }
        .displayName("Lockdown")
        .description("Denies every request at once while it is on.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Bool { false }

        func currentValue() async throws -> Bool {
            let s = Snapshot.load()
            return s.signedIn && Glance.mode(s, now: Glance.now()) == .lockdown
        }
    }
}

/// Opens the newest waiting request (the app's home when nothing waits), with the count.
struct WaitingControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.waiting"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: Provider()) { count in
            ControlWidgetButton(action: ShowWaitingIntent()) {
                Label(Glance.countLine(count), systemImage: count == 0 ? "tray" : "tray.full.fill")
            }
            .tint(Palette.accent)
        }
        .displayName("Waiting request")
        .description("Opens the newest request that waits for you.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Int { 1 }

        func currentValue() async throws -> Int {
            Glance.waiting(Snapshot.load(), now: Glance.now()).count
        }
    }
}

/// Ends every running bypass.
struct StopBypassControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.stopBypass"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind) {
            ControlWidgetButton(action: StopBypassIntent()) {
                Label("Stop bypass", systemImage: "bolt.slash.fill")
            }
            .tint(Palette.danger)
        }
        .displayName("Stop bypass")
        .description("Ends every bypass now; requests wait for you again.")
    }
}
