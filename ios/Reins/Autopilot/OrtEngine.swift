import Foundation
import OnnxRuntimeC

/// ONNX Runtime on this phone's CPU (the Android app's `OrtEngine`): every graph optimisation, up to four threads, one
/// session per model file, run sequentially inside. ONNX Runtime's telemetry is switched off, so nothing about the
/// model or its use leaves the phone.
///
/// This uses ONNX Runtime's C API: the Swift package's Objective-C wrapper has no boolean tensors, and the model's
/// `marker_mask` is one.
struct OrtEngine: InferenceEngine {
    static let maxThreads = 4

    func open(_ path: String) throws -> InferenceSession {
        let ort = try Ort.shared()
        var options: OpaquePointer?
        try ort.check(ort.api.CreateSessionOptions(&options))
        defer { ort.api.ReleaseSessionOptions(options) }
        try ort.check(ort.api.SetSessionGraphOptimizationLevel(options, ORT_ENABLE_ALL))
        let threads = min(max(ProcessInfo.processInfo.activeProcessorCount, 1), Self.maxThreads)
        try ort.check(ort.api.SetIntraOpNumThreads(options, Int32(threads)))
        try ort.check(ort.api.SetSessionExecutionMode(options, ORT_SEQUENTIAL))
        var session: OpaquePointer?
        try ort.check(ort.api.CreateSession(ort.env, path, options, &session))
        guard let session else { throw OrtFailure(message: "ONNX Runtime opened no session") }
        do {
            return OrtSession(ort: ort, session: session, inputs: try ort.inputNames(session))
        } catch {
            ort.api.ReleaseSession(session)
            throw error
        }
    }
}

/// What ONNX Runtime said went wrong.
struct OrtFailure: Error, CustomStringConvertible {
    var message: String
    var description: String { message }
}

/// The C API table, the process's one environment and the CPU memory description tensors are made with.
final class Ort: @unchecked Sendable {
    let api: OrtApi
    let env: OpaquePointer
    let cpu: OpaquePointer

    private static let lock = NSLock()
    nonisolated(unsafe) private static var instance: Ort?

    /// Made on first use and kept for the life of the process (ONNX Runtime wants one environment).
    static func shared() throws -> Ort {
        lock.lock()
        defer { lock.unlock() }
        if let instance { return instance }
        guard let base = OrtGetApiBase(), let table = base.pointee.GetApi(UInt32(ORT_API_VERSION)) else {
            throw OrtFailure(message: "this ONNX Runtime does not offer API version \(ORT_API_VERSION)")
        }
        let made = try Ort(api: table.pointee)
        instance = made
        return made
    }

    private init(api: OrtApi) throws {
        self.api = api
        var env: OpaquePointer?
        try Self.check(api, api.CreateEnv(ORT_LOGGING_LEVEL_WARNING, "reins", &env))
        guard let env else { throw OrtFailure(message: "ONNX Runtime made no environment") }
        _ = api.DisableTelemetryEvents(env).map { api.ReleaseStatus($0) }
        var cpu: OpaquePointer?
        try Self.check(api, api.CreateCpuMemoryInfo(OrtArenaAllocator, OrtMemTypeDefault, &cpu))
        guard let cpu else { throw OrtFailure(message: "ONNX Runtime has no CPU memory") }
        self.env = env
        self.cpu = cpu
    }

    func check(_ status: OpaquePointer?) throws {
        try Self.check(api, status)
    }

    /// A status is nil when all went well; otherwise its message is read and it is freed.
    private static func check(_ api: OrtApi, _ status: OpaquePointer?) throws {
        guard let status else { return }
        let message = api.GetErrorMessage(status).map { String(cString: $0) } ?? "unknown error"
        api.ReleaseStatus(status)
        throw OrtFailure(message: message)
    }

    func inputNames(_ session: OpaquePointer) throws -> Set<String> {
        var count = 0
        try check(api.SessionGetInputCount(session, &count))
        var allocator: UnsafeMutablePointer<OrtAllocator>?
        try check(api.GetAllocatorWithDefaultOptions(&allocator))
        var names = Set<String>()
        for i in 0..<count {
            var name: UnsafeMutablePointer<CChar>?
            try check(api.SessionGetInputName(session, i, allocator, &name))
            if let name {
                names.insert(String(cString: name))
                _ = api.AllocatorFree(allocator, name).map { api.ReleaseStatus($0) }
            }
        }
        return names
    }
}

/// One open model file. ONNX Runtime allows concurrent runs of a session.
final class OrtSession: InferenceSession, @unchecked Sendable {
    private let ort: Ort
    private var session: OpaquePointer?
    /// A model exported without an input (an older export without `qtype`, say) simply does not get it.
    let inputs: Set<String>

    init(ort: Ort, session: OpaquePointer, inputs: Set<String>) {
        self.ort = ort
        self.session = session
        self.inputs = inputs
    }

    deinit { close() }

    func close() {
        if let session { ort.api.ReleaseSession(session) }
        session = nil
    }

    func run(_ input: PackedInput) throws -> RawOutput {
        guard let session else { throw OrtFailure(message: "the session is closed") }
        let api = ort.api
        var tensors = Tensors(api: api)
        defer { tensors.release() }
        if inputs.contains(ModelTensors.inputIds) {
            try tensors.add(ModelTensors.inputIds, input.inputIds, input.tokenShape, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, ort)
        }
        if inputs.contains(ModelTensors.attentionMask) {
            try tensors.add(ModelTensors.attentionMask, input.attentionMask, input.tokenShape, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, ort)
        }
        if inputs.contains(ModelTensors.markerPos) {
            try tensors.add(ModelTensors.markerPos, input.markerPos, input.markerShape, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, ort)
        }
        if inputs.contains(ModelTensors.markerMask) {
            try tensors.add(ModelTensors.markerMask, input.markerMask, input.markerShape, ONNX_TENSOR_ELEMENT_DATA_TYPE_BOOL, ort)
        }
        if inputs.contains(ModelTensors.qtype) {
            try tensors.add(ModelTensors.qtype, input.qtype, input.batchShape, ONNX_TENSOR_ELEMENT_DATA_TYPE_INT64, ort)
        }

        let outputNames = CStrings(ModelTensors.outputs)
        defer { outputNames.free() }
        var outputs = [OpaquePointer?](repeating: nil, count: ModelTensors.outputs.count)
        defer { outputs.forEach { if let v = $0 { api.ReleaseValue(v) } } }
        let inputNames = CStrings(tensors.names)
        defer { inputNames.free() }
        let values = tensors.values.map { Optional($0) }
        try values.withUnsafeBufferPointer { valuePtr in
            try outputs.withUnsafeMutableBufferPointer { outPtr in
                try ort.check(api.Run(
                    session, nil,
                    inputNames.pointers, valuePtr.baseAddress, values.count,
                    outputNames.pointers, ModelTensors.outputs.count, outPtr.baseAddress
                ))
            }
        }
        func read(_ index: Int) throws -> RawTensor {
            guard let value = outputs[index] else { throw OrtFailure(message: "the model has no output \(ModelTensors.outputs[index])") }
            return try floats(value)
        }
        return RawOutput(logits: try read(0), act: try read(1), pooled: try read(2))
    }

    /// A float tensor's values and shape, copied out of ONNX Runtime's memory.
    private func floats(_ value: OpaquePointer) throws -> RawTensor {
        let api = ort.api
        var info: OpaquePointer?
        try ort.check(api.GetTensorTypeAndShape(value, &info))
        defer { if let info { api.ReleaseTensorTypeAndShapeInfo(info) } }
        var type = ONNX_TENSOR_ELEMENT_DATA_TYPE_UNDEFINED
        try ort.check(api.GetTensorElementType(info, &type))
        guard type == ONNX_TENSOR_ELEMENT_DATA_TYPE_FLOAT else { throw OrtFailure(message: "an output is not float32") }
        var rank = 0
        try ort.check(api.GetDimensionsCount(info, &rank))
        var shape = [Int64](repeating: 0, count: rank)
        try ort.check(api.GetDimensions(info, &shape, rank))
        var count = 0
        try ort.check(api.GetTensorShapeElementCount(info, &count))
        var data: UnsafeMutableRawPointer?
        try ort.check(api.GetTensorMutableData(value, &data))
        guard let data else { return RawTensor(values: [], shape: shape) }
        let floats = data.bindMemory(to: Float.self, capacity: count)
        return RawTensor(values: Array(UnsafeBufferPointer(start: floats, count: count)), shape: shape)
    }
}

/// The input tensors of one run, over buffers of their own that live until the run is over.
private struct Tensors {
    let api: OrtApi
    var names: [String] = []
    var values: [OpaquePointer] = []
    var buffers: [UnsafeMutableRawPointer] = []

    init(api: OrtApi) { self.api = api }

    mutating func add<T>(_ name: String, _ data: [T], _ shape: [Int64], _ type: ONNXTensorElementDataType, _ ort: Ort) throws {
        let bytes = max(data.count * MemoryLayout<T>.stride, 1)
        let buffer = UnsafeMutableRawPointer.allocate(byteCount: bytes, alignment: 16)
        buffers.append(buffer)
        data.withUnsafeBytes { buffer.copyMemory(from: $0.baseAddress ?? UnsafeRawPointer(buffer), byteCount: $0.count) }
        var value: OpaquePointer?
        try shape.withUnsafeBufferPointer { dims in
            try ort.check(api.CreateTensorWithDataAsOrtValue(
                ort.cpu, buffer, data.count * MemoryLayout<T>.stride, dims.baseAddress, shape.count, type, &value
            ))
        }
        guard let value else { throw OrtFailure(message: "ONNX Runtime made no tensor for \(name)") }
        names.append(name)
        values.append(value)
    }

    func release() {
        values.forEach { api.ReleaseValue($0) }
        buffers.forEach { $0.deallocate() }
    }
}

/// C strings for ONNX Runtime's name arrays, freed by hand.
private struct CStrings {
    let pointers: [UnsafePointer<CChar>?]

    init(_ strings: [String]) {
        pointers = strings.map { UnsafePointer(strdup($0)) }
    }

    func free() {
        pointers.forEach { Foundation.free(UnsafeMutablePointer(mutating: $0)) }
    }
}
