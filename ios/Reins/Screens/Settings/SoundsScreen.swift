import SwiftUI

/// Settings > Sounds (the Settings board, 03; the Android app's `SoundsScreen`), quiet: one master above everything
/// (notifications included), the sounds by kind with a volume, haptics with a strength, and chips that play the real
/// thing. The switches here are the only ones Reins listens to; the ring/silent switch and the volume buttons still
/// shape what is heard (see `SoundPlayer`).
struct SoundsScreen: View {
    @Environment(\.feedback) private var feedback

    private var engine: FeedbackPreviewing? { feedback as? FeedbackPreviewing }
    private var store: FeedbackStore { engine?.store ?? .shared }

    var body: some View {
        let s = store.settings
        QuietPage("Sounds") {
            FlatGroup {
                SwitchTileRow(title: "Sounds & haptics", glyph: .speaker, isOn: s.master, id: "soundsMaster") { on in
                    store.update { $0.master = on }
                }
            }
            .padding(.top, 14)

            QuietHelpHeader(title: "Sounds", help: "The first switch silences every sound and vibration in Reins, its notifications included. "
                + "These play while Reins is open; in the background the same chimes come with its notifications. "
                + "When open: a new request chimes while Reins is on screen too; the ring/silent switch mutes it.")
            FlatGroup {
                SwitchTileRow(title: "Chimes", glyph: .bell, isOn: s.sounds, enabled: s.master, id: "sounds") { on in
                    store.update { $0.sounds = on }
                }
                SwitchTileRow(title: "Interface", glyph: .appearance, isOn: s.interfaceSounds, enabled: s.soundsOn, id: "interfaceSounds") { on in
                    store.update { $0.interfaceSounds = on }
                }
                SwitchTileRow(title: "Requests", glyph: .tray, isOn: s.requestSounds, enabled: s.soundsOn, id: "requestSounds") { on in
                    store.update { $0.requestSounds = on }
                }
                SwitchTileRow(title: "When open", glyph: .phone, isOn: s.soundWhenOpen, enabled: s.allows(.requests), id: "soundWhenOpen") { on in
                    store.update { $0.soundWhenOpen = on }
                }
                SwitchTileRow(title: "Alerts", glyph: .warning, isOn: s.alertSounds, enabled: s.soundsOn, id: "alertSounds") { on in
                    store.update { $0.alertSounds = on }
                }
                SwitchTileRow(title: "Autopilot", glyph: .autopilot, isOn: s.autopilotSounds, enabled: s.soundsOn, id: "autopilotSounds") { on in
                    store.update { $0.autopilotSounds = on }
                }
                VolumeRow(store: store, engine: engine)
            }

            SectionHeader("Haptics", quiet: true)
            FlatGroup {
                SwitchTileRow(title: "Haptics", glyph: .haptics, isOn: s.haptics, enabled: s.master, id: "haptics") { on in
                    store.update { $0.haptics = on }
                }
            }
            StrengthRow(store: store)
                .padding(.horizontal, 20)
                .padding(.top, 12)

            SectionHeader("Feel them", quiet: true)
            PreviewChips(engine: engine, enabled: s.soundsOn || s.hapticsOn)
                .padding(.horizontal, 20)
        }
    }
}

// MARK: Rows

/// Ten steps of volume, between a small and a big speaker. The detents climb the scale as the thumb moves, heard at
/// the level they set; letting go plays the selection sound once more at the final level.
private struct VolumeRow: View {
    static let steps = 10

    var store: FeedbackStore
    var engine: FeedbackPreviewing?
    @State private var last: Int?

    var body: some View {
        let s = store.settings
        let enabled = s.soundsOn
        let percent = Int((s.volume * 100).rounded())
        HStack(spacing: 12) {
            GlyphIcon(.speaker, size: 16).foregroundStyle(Palette.inkMeta).accessibilityHidden(true)
            Slider(
                value: Binding(get: { Double(store.settings.volume) }, set: { change(Float($0)) }),
                in: 0...1,
                step: 1 / Double(Self.steps),
                onEditingChanged: { editing in
                    if !editing { engine?.preview(haptic: nil, cue: FeedbackEvent.selection.cue, step: 0) }
                }
            )
            .tint(Palette.violet)
            .accessibilityLabel("Volume")
            .accessibilityValue("\(percent) percent")
            .accessibilityIdentifier("volume")
            GlyphIcon(.speaker, size: 22).foregroundStyle(Palette.ink2).accessibilityHidden(true)
        }
        .frame(minHeight: 56)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.4)
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
        Chooser(HapticStrength.allCases.map { level in
            Chooser.Segment(id: "strength:\(level.label)", title: level.label, selected: s.strength == level) {
                store.update { $0.strength = level }
                feedback.play(.selection)
            }
        })
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Strength")
        .accessibilityIdentifier("strength")
        .disabled(!s.hapticsOn)
        .opacity(s.hapticsOn ? 1 : 0.5)
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
        Sample(title: "More autonomy", supporting: "From Manual to Auto", events: [.autopilotOn]),
        Sample(title: "Less autonomy", supporting: "Back to Manual", events: [.autopilotOff]),
        Sample(title: "Bypass", supporting: "On, then off", events: [.bypassOn, .bypassOff], gapMs: 900),
        Sample(title: "Lockdown", supporting: "Everything denied", events: [.lockdownOn]),
    ]
}

/// "Feel them": a grid of chips, each its name over a little score of its haptic, that plays it when tapped.
private struct PreviewChips: View {
    var engine: FeedbackPreviewing?
    var enabled: Bool
    @State private var running = false

    var body: some View {
        // Not lazy: every chip is there for VoiceOver and UI tests, scrolled to or not.
        Grid(horizontalSpacing: 8, verticalSpacing: 8) {
            ForEach(Array(stride(from: 0, to: Sample.all.count, by: 3)), id: \.self) { start in
                GridRow {
                    ForEach(Sample.all[start..<min(start + 3, Sample.all.count)]) { sample in chip(sample) }
                    ForEach(0..<(3 - min(3, Sample.all.count - start)), id: \.self) { _ in Color.clear.gridCellUnsizedAxes([.horizontal, .vertical]) }
                }
            }
        }
    }

    private func chip(_ sample: Sample) -> some View {
        Button {
            play(sample)
        } label: {
            VStack(alignment: .leading, spacing: 10) {
                Score(sample: sample)
                Text(sample.title)
                    .font(RFont.sans(13.5, .medium))
                    .foregroundStyle(enabled ? Palette.ink : Palette.inkMeta)
                    .lineLimit(2)
                    .minimumScaleFactor(0.85)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, minHeight: 72, alignment: .topLeading)
            .padding(12)
            .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(sample.title)
        .accessibilityHint(sample.supporting + ". Plays its sound and haptic")
        .accessibilityIdentifier("try:\(sample.title)")
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

/// A haptic drawn as a score: one tick per tap it plays, rising for detents climbing the scale.
private struct Score: View {
    var sample: Sample

    var body: some View {
        let steps = sample.steps ?? [0]
        let ticks = sample.events.flatMap { _ in steps }
        HStack(alignment: .bottom, spacing: 3) {
            ForEach(Array(ticks.enumerated()), id: \.offset) { _, step in
                Capsule()
                    .fill(Palette.violet)
                    .frame(width: 3, height: CGFloat(6 + step * 3))
            }
        }
        .frame(height: 18, alignment: .bottom)
        .accessibilityHidden(true)
    }
}
