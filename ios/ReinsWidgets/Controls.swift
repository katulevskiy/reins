import AppIntents
import SwiftUI
import WidgetKit

// Control Center, Lock Screen and Action button controls. Their values come from the snapshot; the app reloads them
// with the widgets whenever it writes one. Each uses its field's colour only while its state is on.

/// Lockdown on or off. The intent runs in the app and needs an unlocked phone.
struct LockdownControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.lockdown"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: Provider()) { on in
            ControlWidgetToggle("Lockdown", isOn: on, action: SetLockdownIntent()) { isOn in
                Label(isOn ? "On" : "Off", systemImage: isOn ? "lock.fill" : "lock.open")
            }
            .tint(Palette.fieldWarn)
        }
        .displayName("Lockdown")
        .description("Denies every request at once while it is on.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Bool { false }

        func currentValue() async throws -> Bool {
            let s = SnapshotCache.current()
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
            .tint(count > 0 ? Palette.fieldPush : .gray)
        }
        .displayName("Waiting request")
        .description("Opens the newest request that waits for you.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Int { 1 }

        func currentValue() async throws -> Int {
            Glance.waiting(SnapshotCache.current(), now: Glance.now()).count
        }
    }
}

/// Ends every running bypass: red while one runs.
struct StopBypassControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.stopBypass"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: Provider()) { running in
            ControlWidgetButton(action: StopBypassIntent()) {
                Label(running ? "Stop bypass" : "No bypass", systemImage: "bolt.slash.fill")
            }
            .tint(running ? Palette.fieldDelete : .gray)
        }
        .displayName("Stop bypass")
        .description("Ends every bypass now; requests wait for you again.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Bool { true }

        func currentValue() async throws -> Bool {
            let s = Snapshot.load()
            return s.signedIn && Glance.bypassEnd(s, now: Glance.now()) != nil
        }
    }
}

/// Autopilot's mode in force ("Mode · Auto"), opening Trust where it is changed. In its colour unless Manual.
struct ModeControl: ControlWidget {
    static let kind = "com.reins2fa.app.control.mode"

    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: Self.kind, provider: Provider()) { mode in
            ControlWidgetButton(action: ShowAutopilotIntent()) {
                Label(mode.map { $0.name } ?? "Sign in", systemImage: mode?.symbol ?? "person.crop.circle")
            }
            .tint(mode.map { $0 == .manual ? Color.gray : GlanceStyle.tint($0) } ?? .gray)
        }
        .displayName("Autopilot mode")
        .description("Shows Autopilot's mode and opens Reins where it is changed.")
    }

    struct Provider: ControlValueProvider {
        var previewValue: Glance.Mode? { .auto }

        func currentValue() async throws -> Glance.Mode? {
            let s = Snapshot.load()
            return s.signedIn ? Glance.mode(s, now: Glance.now()) : nil
        }
    }
}
