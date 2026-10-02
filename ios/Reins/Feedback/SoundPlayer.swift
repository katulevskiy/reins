import AVFoundation
import os

/// Every cue preloaded as PCM into memory and played through one `AVAudioEngine`: a small pool of player voices, each
/// through a varispeed unit (the detent ladder resamples one tick, like the Android SoundPool's rate), into the main
/// mixer. No decoding on press and no thread hop the caller waits for: everything runs on one serial queue.
///
/// Audio session: `.ambient`. The user's music keeps playing (ambient mixes with others and never takes focus), and
/// the ring/silent switch mutes Reins's sounds. The Android app ignores the phone's own touch-sound settings once its
/// master is on, but on iOS the silent switch is the one control every user expects to silence an app's interface
/// sounds, and `.ambient` is what every well-behaved iOS app's sound effects use. The in-app switches still decide
/// everything above it; the volume buttons set the level, the in-app slider trims it.
///
/// Latency: the engine starts lazily (the first touch or cue) and then stays running while the user is touching the
/// app (`WarmPolicy`), so the tap that sounds finds the output awake; after a quiet spell it stops and costs nothing.
/// Interruptions, route and configuration changes stop the engine; the next cue starts it again.
final class SoundPlayer: @unchecked Sendable {
    private let queue = DispatchQueue(label: "com.reins2fa.app.sounds", qos: .userInteractive)
    private let log = Logger(subsystem: "com.reins2fa.app", category: "feedback")
    private let clock: () -> Int64
    private let warm: WarmPolicy

    private var buffers: [String: AVAudioPCMBuffer] = [:]
    private var format: AVAudioFormat?
    private var engine: AVAudioEngine?
    private var players: [AVAudioPlayerNode] = []
    private var speeds: [AVAudioUnitVarispeed] = []
    private var pool = VoicePool(count: FeedbackGate.maxVoices)
    private var sessionReady = false
    private var idleCheck: DispatchWorkItem?
    private var observers: [NSObjectProtocol] = []

    init(clock: @escaping () -> Int64 = feedbackClock) {
        self.clock = clock
        warm = WarmPolicy(clock: clock)
        queue.async { self.load() }
        observe()
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    /// Starts `cue` at `volume` (0...1) and `rate`; returns at once. `done` reports, on the sound queue, whether it
    /// sounded (false while the files are still loading or the engine cannot start).
    func play(_ cue: Cue, volume: Float, rate: Float, priority: Int, done: (@Sendable (Bool) -> Void)? = nil) {
        queue.async {
            let ok = self.start(cue, volume: volume, rate: rate, priority: priority)
            done?(ok)
        }
    }

    /// A finger went down somewhere in the app: wake the output now so it is running by the time the tap sounds.
    func touchDown() {
        warm.touch()
        queue.async {
            guard self.engine?.isRunning != true else { return }
            _ = self.ensureRunning()
        }
    }

    /// The app left the foreground: let the output go at once.
    func suspend() {
        warm.stop()
        queue.async { self.stopEngine() }
    }

    // MARK: Loading

    private func load() {
        for name in CueTable.files {
            guard let url = Bundle.main.url(forResource: name, withExtension: "wav")
                ?? Bundle.main.url(forResource: name, withExtension: "wav", subdirectory: "Sounds")
            else {
                log.error("no sound \(name)")
                continue
            }
            do {
                let file = try AVAudioFile(forReading: url)
                guard let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)) else { continue }
                try file.read(into: buffer)
                // One format runs through the graph; every asset is mastered alike (mono, 48 kHz).
                if format == nil { format = buffer.format }
                guard buffer.format == format else {
                    log.error("sound \(name) is not in the set's format")
                    continue
                }
                buffers[name] = buffer
            } catch {
                log.error("sound \(name) did not load: \(error.localizedDescription)")
            }
        }
    }

    // MARK: Engine

    private func configureSession() {
        guard !sessionReady else { return }
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.ambient, mode: .default, options: [])
            // The shortest buffer the hardware allows: a tap should be heard within a few milliseconds.
            try? session.setPreferredIOBufferDuration(0.005)
            try session.setActive(true)
            sessionReady = true
        } catch {
            log.error("audio session: \(error.localizedDescription)")
        }
    }

    private func buildGraph() -> AVAudioEngine? {
        if let engine { return engine }
        guard let format else { return nil }
        let engine = AVAudioEngine()
        for _ in 0..<FeedbackGate.maxVoices {
            let player = AVAudioPlayerNode()
            let speed = AVAudioUnitVarispeed()
            engine.attach(player)
            engine.attach(speed)
            engine.connect(player, to: speed, format: format)
            engine.connect(speed, to: engine.mainMixerNode, format: format)
            players.append(player)
            speeds.append(speed)
        }
        self.engine = engine
        return engine
    }

    /// Whether the engine runs after the call.
    private func ensureRunning() -> Bool {
        configureSession()
        guard let engine = buildGraph() else { return false }
        if !engine.isRunning {
            do {
                engine.prepare()
                try engine.start()
                pool.reset()
            } catch {
                log.error("audio engine did not start: \(error.localizedDescription)")
                return false
            }
        }
        // A player plays as soon as it has something scheduled.
        for player in players where !player.isPlaying { player.play() }
        scheduleIdleCheck()
        return true
    }

    private func start(_ cue: Cue, volume: Float, rate: Float, priority: Int) -> Bool {
        let spec = CueTable.spec(cue)
        guard let buffer = buffers[spec.file] else { return false }
        warm.touch()
        guard ensureRunning() else { return false }
        let lengthMs = Int64(Double(buffer.frameLength) / buffer.format.sampleRate / Double(max(rate, 0.01)) * 1000)
        guard let voice = pool.claim(now: clock(), priority: priority, lengthMs: lengthMs) else { return false }
        speeds[voice].rate = rate
        players[voice].volume = volume
        // `.interrupts`: a stolen voice drops what it was playing and starts this at once.
        players[voice].scheduleBuffer(buffer, at: nil, options: .interrupts)
        if !players[voice].isPlaying { players[voice].play() }
        return true
    }

    private func stopEngine() {
        idleCheck?.cancel()
        idleCheck = nil
        guard let engine, engine.isRunning else { return }
        players.forEach { $0.stop() }
        engine.stop()
        pool.reset()
    }

    /// Stops the engine once nothing has wanted sound for `WarmPolicy.idleMs`.
    private func scheduleIdleCheck() {
        guard idleCheck == nil else { return }
        let item = DispatchWorkItem { [weak self] in
            guard let self else { return }
            self.idleCheck = nil
            if self.warm.wanted {
                self.scheduleIdleCheck()
            } else {
                self.stopEngine()
            }
        }
        idleCheck = item
        queue.asyncAfter(deadline: .now() + .milliseconds(Int(warm.remainingMs) + 50), execute: item)
    }

    // MARK: Interruptions

    private func observe() {
        let center = NotificationCenter.default
        // A call, Siri, another app's audio: the engine was stopped for us; the next cue starts it again.
        observers.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) { [weak self] note in
            guard let self else { return }
            let began = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt).flatMap(AVAudioSession.InterruptionType.init(rawValue:)) == .began
            self.queue.async {
                if began { self.pool.reset() }
                // An interrupted session must be activated again.
                self.sessionReady = false
            }
        })
        // Headphones in or out, a Bluetooth speaker: the output's format may have changed and the engine stopped.
        observers.append(center.addObserver(forName: .AVAudioEngineConfigurationChange, object: nil, queue: nil) { [weak self] _ in
            guard let self else { return }
            self.queue.async { self.pool.reset() }
        })
        // The media server restarted: every object made before is invalid.
        observers.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: nil) { [weak self] _ in
            guard let self else { return }
            self.queue.async {
                self.engine = nil
                self.players = []
                self.speeds = []
                self.pool.reset()
                self.sessionReady = false
            }
        })
    }
}
