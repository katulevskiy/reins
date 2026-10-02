import Foundation
import UIKit

/// Opens a model file. The seam between `OnnxModelRuntime` and ONNX Runtime, so tests can run without native code.
protocol InferenceEngine: Sendable {
    func open(_ path: String) throws -> InferenceSession
}

/// A loaded model. `run` may be called from several threads at once; `close` is called once, after the last run.
protocol InferenceSession: AnyObject, Sendable {
    func run(_ input: PackedInput) throws -> RawOutput
    func close()
}

/// Autopilot's forward pass (spec §6.3, the Android app's `OnnxModelRuntime`): the core tokenizes, calibrates and
/// decides; this only runs the network. The session is opened once and reused; runs share it, and loading or
/// unloading waits for them to finish.
///
/// On a memory warning the session is closed but the file is remembered, and the next run opens it again: the core
/// keeps believing the model is loaded, and a request after the warning is still judged (only more slowly).
final class OnnxModelRuntime: ModelRuntime, @unchecked Sendable {
    private let engine: InferenceEngine
    private let lock = ReadWriteLock()
    // Guarded by `lock`.
    private var session: InferenceSession?
    private var path: String?
    private var memoryObserver: NSObjectProtocol?

    init(engine: InferenceEngine = OrtEngine()) {
        self.engine = engine
    }

    deinit {
        if let memoryObserver { NotificationCenter.default.removeObserver(memoryObserver) }
        session?.close()
    }

    /// The runtime the app hands the core: ONNX Runtime, freeing the model when iOS runs short of memory.
    static func forApp() -> OnnxModelRuntime {
        let runtime = OnnxModelRuntime()
        runtime.memoryObserver = NotificationCenter.default.addObserver(
            forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: nil
        ) { [weak runtime] _ in
            // Off the main thread: a run in progress holds the session until it is done.
            DispatchQueue.global(qos: .utility).async { runtime?.relieveMemory() }
        }
        return runtime
    }

    /// The file the core asked for, if any (loaded now, or opened again on the next run).
    var loaded: String? { lock.read { path } }

    /// The session is open now (not closed by a memory warning).
    var isOpen: Bool { lock.read { session != nil } }

    func load(path: String) throws {
        try lock.write {
            if path == self.path, session != nil { return }
            closeSession()
            session = try failing("could not load the model") { try engine.open(path) }
            self.path = path
        }
    }

    func run(input: ModelInput) throws -> ModelOutput {
        let packed = try failing("the model could not run") { try ModelTensors.pack(input) }
        if let output = try lock.read({ try session.map { try forward($0, packed) } }) {
            return output
        }
        // Closed by a memory warning since the core loaded it: open the same file again.
        return try lock.write {
            guard let path else { throw ForeignError.Failed(reason: "no model is loaded") }
            if session == nil {
                session = try failing("could not load the model") { try engine.open(path) }
            }
            guard let session else { throw ForeignError.Failed(reason: "no model is loaded") }
            return try forward(session, packed)
        }
    }

    func unload() {
        lock.write {
            closeSession()
            path = nil
        }
    }

    /// Frees the session and its few hundred megabytes, keeping the file for the next run.
    func relieveMemory() {
        lock.write {
            session?.close()
            session = nil
        }
    }

    private func forward(_ session: InferenceSession, _ packed: PackedInput) throws -> ModelOutput {
        try failing("the model could not run") { try ModelTensors.unpack(packed, try session.run(packed)) }
    }

    private func closeSession() {
        session?.close()
        session = nil
        path = nil
    }

    /// Every failure crosses to the core as a `ForeignError`, which leaves the request to the user.
    private func failing<T>(_ what: String, _ block: () throws -> T) throws -> T {
        do {
            return try block()
        } catch let e as ForeignError {
            throw e
        } catch {
            throw ForeignError.Failed(reason: "\(what): \(String(describing: error))")
        }
    }
}

/// A many-readers, one-writer lock (`pthread_rwlock`), kept at a fixed address as pthread requires.
final class ReadWriteLock: @unchecked Sendable {
    private let handle: UnsafeMutablePointer<pthread_rwlock_t>

    init() {
        handle = .allocate(capacity: 1)
        handle.initialize(to: pthread_rwlock_t())
        pthread_rwlock_init(handle, nil)
    }

    deinit {
        pthread_rwlock_destroy(handle)
        handle.deinitialize(count: 1)
        handle.deallocate()
    }

    func read<T>(_ body: () throws -> T) rethrows -> T {
        pthread_rwlock_rdlock(handle)
        defer { pthread_rwlock_unlock(handle) }
        return try body()
    }

    func write<T>(_ body: () throws -> T) rethrows -> T {
        pthread_rwlock_wrlock(handle)
        defer { pthread_rwlock_unlock(handle) }
        return try body()
    }
}
