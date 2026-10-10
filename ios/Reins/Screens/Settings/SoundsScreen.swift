import SwiftUI

/// Settings > Sounds & haptics (the Android app's `SoundsScreen`): one master above everything (notifications
/// included), the sounds by kind with a volume, haptics with a strength, and a list that plays the real thing. The
/// switches here are the only ones Reins listens to; the ring/silent switch and the volume buttons still shape what is
/// heard (see `SoundPlayer`).
struct SoundsScreen: View {
    @Environment(\.feedback) private var feedback

    private var engine: FeedbackPreviewing? { feedback as? FeedbackPreviewing }
    private var store: FeedbackStore { engine?.store ?? .shared }

    var body: some View {
        let s = store.settings
        List {
            Section {
                SwitchRow(title: "Sounds & haptics", symbol: "speaker.wave.2", isOn: s.master, id: "soundsMaster") { on in
                    store.update { $0.master = on }
                }
            } footer: {
                FooterText("Off silences every sound and vibration in Reins, its notifications included.")
            }

            Section {
                SwitchRow(title: "Sounds", subtitle: "Every sound in Reins", symbol: "speaker.wave.2", isOn: s.sounds,
                          enabled: s.master, id: "sounds") { on in store.update { $0.sounds = on } }
                SwitchRow(title: "Interface", subtitle: nil,
                          symbol: "square.grid.2x2", isOn: s.interfaceSounds, enabled: s.soundsOn, id: "interfaceSounds") { on in
                    store.update { $0.interfaceSounds = on }
                }
                SwitchRow(title: "Requests", subtitle: nil, symbol: "bell",
                          isOn: s.requestSounds, enabled: s.soundsOn, id: "requestSounds") { on in store.update { $0.requestSounds = on } }
                SwitchRow(title: "Alerts", subtitle: nil,
                          symbol: "exclamationmark.triangle", isOn: s.alertSounds, enabled: s.soundsOn, id: "alertSounds") { on in
                    store.update { $0.alertSounds = on }
                }
                SwitchRow(title: "Autopilot", subtitle: nil, symbol: "sparkles",
                          isOn: s.autopilotSounds, enabled: s.soundsOn, id: "autopilotSounds") { on in store.update { $0.autopilotSounds = on } }
                VolumeRow(store: store, engine: engine)
            } header: {
                HeaderText("Sounds")
            } footer: {
                FooterText("While Reins is open. In the background the same chimes come with its notifications.")
            }

            Section {
                SwitchRow(title: "Haptics", subtitle: engine?.hapticTier, symbol: "iphone.radiowaves.left.and.right", isOn: s.haptics,
                          enabled: s.master, id: "haptics") { on in store.update { $0.haptics = on } }
                StrengthRow(store: store)
            } header: {
                HeaderText("Haptics")
            }

            Section {
                PreviewRows(engine: engine, enabled: s.soundsOn || s.hapticsOn)
            } header: {
                HeaderText("Try them")
            } footer: {
                FooterText("Each plays its sound and haptic, even when that kind of sound is off.")
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .pageBackground()
        .navigationTitle("Sounds & haptics")
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: Rows

private struct HeaderText: View {
    var text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text).font(RFont.sans(13, .medium)).foregroundStyle(Palette.secondary)
    }
}

private struct FooterText: View {
    var text: String
    init(_ text: String) { self.text = text }

    var body: some View { GroupFooter(text) }
}

/// An icon, a title with an explanation, and a switch. Greyed out like the switch when a switch above turns it off.
private struct SwitchRow: View {
    var title: String
    var subtitle: String?
    var symbol: String
    var isOn: Bool
    var enabled = true
    var id: String
    var onChange: (Bool) -> Void

    @Environment(\.feedback) private var feedback

    var body: some View {
        Toggle(isOn: Binding(get: { isOn }, set: { on in
            onChange(on)
            // After the change: turning the master off is silent, turning it on answers.
            feedback.play(.toggle(on))
        })) {
            RowLabel(title: title, subtitle: subtitle, symbol: symbol, enabled: enabled)
        }
        .tint(Palette.accent)
        .disabled(!enabled)
        .listRowBackground(Palette.elevated)
        .accessibilityIdentifier(id)
    }
}

private struct RowLabel: View {
    var title: String
    var subtitle: String?
    var symbol: String
    var tint: Color = Palette.secondary
    var enabled = true

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: symbol)
                .font(.system(size: 18, weight: .regular))
                .foregroundStyle(tint)
                .frame(width: 24)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(RFont.sans(16, .medium))
                    .foregroundStyle(enabled ? Palette.text : Palette.tertiary)
                if let subtitle {
                    Text(subtitle)
                        .font(RFont.sans(13))
                        .foregroundStyle(Palette.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .opacity(enabled ? 1 : 0.6)
        .padding(.vertical, 2)
    }
}

/// Ten steps of volume. The detents climb the scale as the thumb moves, heard at the level they set; letting go plays
/// the selection sound once more at the final level.
private struct VolumeRow: View {
    static let steps = 10

    var store: FeedbackStore
    var engine: FeedbackPreviewing?
    @State private var last: Int?

    var body: some View {
        let s = store.settings
        let enabled = s.soundsOn
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .center, spacing: 10) {
                RowLabel(title: "Volume", subtitle: nil,
                         symbol: "speaker.wave.2", enabled: enabled)
                Spacer(minLength: 0)
                Text("\(Int((s.volume * 100).rounded()))%")
                    .font(RFont.mono(14, .medium))
                    .foregroundStyle(Palette.secondary)
                    .monospacedDigit()
                    .accessibilityIdentifier("volumeValue")
            }
            Slider(
                value: Binding(get: { Double(store.settings.volume) }, set: { change(Float($0)) }),
                in: 0...1,
                step: 1 / Double(Self.steps),
                onEditingChanged: { editing in
                    if !editing { engine?.preview(haptic: nil, cue: FeedbackEvent.selection.cue, step: 0) }
                }
            )
            .tint(Palette.accent)
            .accessibilityLabel("Volume")
            .accessibilityValue("\(Int((s.volume * 100).rounded())) percent")
            .accessibilityIdentifier("volume")
        }
        .disabled(!enabled)
        .padding(.vertical, 4)
        .listRowBackground(Palette.elevated)
    }

    private func change(_ value: Float) {
        let previous = last ?? Int((store.settings.volume * Float(Self.steps)).rounded())
        store.update { $0.volume = value }
        let step = Int((value * Float(Self.steps)).rounded())
        last = step
        if step != previous { engine?.preview(.detent, step: step) }
    }
}

/// How firmly haptics are felt. The choice is felt at the strength it picks: its haptic plays after the change.
private struct StrengthRow: View {
    var store: FeedbackStore
    @Environment(\.feedback) private var feedback

    var body: some View {
        let s = store.settings
        VStack(alignment: .leading, spacing: 12) {
            RowLabel(title: "Strength", subtitle: "How firmly taps and alerts are felt", symbol: "dial.medium", enabled: s.hapticsOn)
            Picker("Strength", selection: Binding(get: { store.settings.strength }, set: { level in
                store.update { $0.strength = level }
                feedback.play(.selection)
            })) {
                ForEach(HapticStrength.allCases, id: \.self) { level in
                    Text(level.label).tag(level)
                }
            }
            .pickerStyle(.segmented)
            .accessibilityIdentifier("strength")
        }
        .disabled(!s.hapticsOn)
        .padding(.vertical, 4)
        .listRowBackground(Palette.elevated)
    }
}

/// A moment to audition: its title, when it plays, and the event (or a little sequence of them).
private struct Sample: Identifiable {
    let title: String
    let supporting: String
    let events: [FeedbackEvent]
    var gapMs = 0
    var steps: [Int]?

    var id: String { title }

    static let all: [Sample] = [
        Sample(title: "Request", supporting: "Something waits for your approval", events: [.requestArrived]),
        Sample(title: "Approve", supporting: "Approved once", events: [.approved]),
        Sample(title: "Deny", supporting: "Denied", events: [.denied]),
        Sample(title: "Allow for a while", supporting: "A permission that stays", events: [.grantCreated]),
        Sample(title: "Connected", supporting: "An AI, a server or an account", events: [.connected]),
        Sample(title: "File approved", supporting: "An upload can be downloaded", events: [.uploadApproved]),
        Sample(title: "Revoke", supporting: "A grant or connection removed", events: [.revoked]),
        Sample(title: "Switch", supporting: "On, then off", events: [.toggleOn, .toggleOff], gapMs: 450),
        Sample(title: "Slider", supporting: "Detents climbing the scale", events: [.detent], gapMs: 110, steps: [0, 1, 2, 3, 4]),
        Sample(title: "Error", supporting: "Something went wrong", events: [.error]),
        Sample(title: "Alert", supporting: "A grant ends soon", events: [.alert]),
        Sample(title: "Autopilot approved", supporting: "Approved for you, quietly", events: [.autoApproved]),
        Sample(title: "Autopilot denied", supporting: "Denied for you", events: [.autoDenied]),
        Sample(title: "More autonomy", supporting: "From Assisted to Auto", events: [.autopilotOn]),
        Sample(title: "Less autonomy", supporting: "Back to Manual", events: [.autopilotOff]),
        Sample(title: "Bypass", supporting: "On, then off", events: [.bypassOn, .bypassOff], gapMs: 900),
        Sample(title: "Lockdown", supporting: "Everything denied", events: [.lockdownOn]),
    ]
}

private struct PreviewRows: View {
    var engine: FeedbackPreviewing?
    var enabled: Bool
    @State private var running = false

    var body: some View {
        ForEach(Sample.all) { sample in
            Button {
                play(sample)
            } label: {
                RowLabel(title: sample.title, subtitle: sample.supporting, symbol: "play.fill",
                         tint: enabled ? Palette.accent : Palette.tertiary, enabled: enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .listRowBackground(Palette.elevated)
            .accessibilityHint("Plays its sound and haptic")
            .accessibilityIdentifier("try:\(sample.title)")
        }
    }

    private func play(_ sample: Sample) {
        guard enabled, !running, let engine else { return }
        running = true
        Task { @MainActor in
            for event in sample.events {
                for step in sample.steps ?? [0] {
                    engine.preview(event, step: step)
                    try? await Task.sleep(for: .milliseconds(sample.gapMs))
                }
            }
            running = false
        }
    }
}
