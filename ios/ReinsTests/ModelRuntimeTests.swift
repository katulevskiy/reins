import XCTest
@testable import Reins

/// The model runtime (the Android app's `ModelRuntimeTest`): the tensors it builds and reads, how it holds its
/// session, and ONNX Runtime itself on the few-kilobyte model of `crates/reins-laya/testdata` (its numbers mean
/// nothing; its inputs, outputs, shapes and types are the real model's).
final class ModelRuntimeTests: XCTestCase {
    private func input(batch: Int = 2, seqLen: Int = 4, k: Int = 3) -> ModelInput {
        ModelInput(
            batch: UInt32(batch), seqLen: UInt32(seqLen), k: UInt32(k),
            inputIds: (0..<(batch * seqLen)).map(Int64.init),
            attentionMask: Array(repeating: 1, count: batch * seqLen),
            markerPos: (0..<(batch * k)).map { Int64($0 % seqLen) },
            markerMask: Data(repeating: 1, count: batch * k),
            qtype: Array(repeating: 0, count: batch)
        )
    }

    /// Answers with tensors of the right shapes for the batch it gets, numbered so their order can be checked.
    private final class FakeSession: InferenceSession, @unchecked Sendable {
        let hidden: Int
        private let lock = NSLock()
        private var _runs: [PackedInput] = []
        private(set) var closed = false
        var runs: [PackedInput] { lock.withLock { _runs } }

        init(hidden: Int = 5) { self.hidden = hidden }

        func run(_ input: PackedInput) throws -> RawOutput {
            lock.withLock { _runs.append(input) }
            let b = Int64(input.batch)
            return RawOutput(
                logits: RawTensor(values: (0..<(input.batch * input.k)).map(Float.init), shape: [b, Int64(input.k)]),
                act: RawTensor(values: (0..<(input.batch * 2)).map { -Float($0) }, shape: [b, 2]),
                pooled: RawTensor(values: (0..<(input.batch * hidden)).map { Float($0) / 10 }, shape: [b, Int64(hidden)])
            )
        }

        func close() { closed = true }
    }

    private final class FakeEngine: InferenceEngine, @unchecked Sendable {
        private let lock = NSLock()
        var opened: [String] = []
        var sessions: [FakeSession] = []
        var failure: Error?

        func open(_ path: String) throws -> InferenceSession {
            try lock.withLock {
                if let failure { throw failure }
                opened.append(path)
                let s = FakeSession()
                sessions.append(s)
                return s
            }
        }
    }

    private struct Broken: Error, CustomStringConvertible { var description = "bad file" }

    // MARK: Tensors

    func testPackingKeepsTheRowMajorLayoutAndTheShapes() throws {
        let packed = try ModelTensors.pack(input())
        XCTAssertEqual(packed.tokenShape, [2, 4])
        XCTAssertEqual(packed.markerShape, [2, 3])
        XCTAssertEqual(packed.batchShape, [2])
        XCTAssertEqual(packed.inputIds, (0..<8).map(Int64.init))
        XCTAssertEqual(packed.markerMask.count, 6)
    }

    func testMarkerMaskBytesBecomeOnnxBooleans() throws {
        var i = input()
        i.markerMask = Data([1, 0, 7, 0, 1, 255])
        XCTAssertEqual(try ModelTensors.pack(i).markerMask, [1, 0, 1, 0, 1, 1])
    }

    func testAnArrayOfTheWrongSizeIsRefusedNamingIt() {
        var bad = input()
        bad.markerPos = [1, 2]
        XCTAssertThrowsError(try ModelTensors.pack(bad)) { XCTAssertTrue(String(describing: $0).contains("marker_pos")) }
        var empty = input()
        empty.batch = 0
        XCTAssertThrowsError(try ModelTensors.pack(empty))
    }

    func testUnpackingReadsTheHiddenSizeFromThePooledTensor() throws {
        let packed = try ModelTensors.pack(input())
        let out = try ModelTensors.unpack(packed, try FakeSession(hidden: 7).run(packed))
        XCTAssertEqual(out.hidden, 7)
        XCTAssertEqual(out.logits.count, 6)
        XCTAssertEqual(out.act, [0, -1, -2, -3])
        XCTAssertEqual(out.pooled.count, 14)
    }

    func testOutputsThatDoNotFitTheBatchAreRefused() throws {
        let packed = try ModelTensors.pack(input())
        let raw = RawOutput(
            logits: RawTensor(values: Array(repeating: 0, count: 5), shape: [1, 5]),
            act: RawTensor(values: Array(repeating: 0, count: 4), shape: [2, 2]),
            pooled: RawTensor(values: Array(repeating: 0, count: 10), shape: [2, 5])
        )
        XCTAssertThrowsError(try ModelTensors.unpack(packed, raw)) { XCTAssertTrue(String(describing: $0).contains("logits")) }
    }

    // MARK: The session

    func testTheSessionIsOpenedOnceAndReusedAndANewFileReplacesIt() throws {
        let engine = FakeEngine()
        let runtime = OnnxModelRuntime(engine: engine)
        try runtime.load(path: "/models/a/model.onnx")
        try runtime.load(path: "/models/a/model.onnx")
        XCTAssertEqual(engine.opened.count, 1)
        _ = try runtime.run(input: input())
        _ = try runtime.run(input: input())
        XCTAssertEqual(engine.sessions.first?.runs.count, 2)
        try runtime.load(path: "/models/b/model.onnx")
        XCTAssertTrue(engine.sessions[0].closed, "the old session is freed")
        XCTAssertEqual(runtime.loaded, "/models/b/model.onnx")
    }

    func testRunningWithoutAModelOrAModelThatFailsToLoadIsAForeignError() {
        let engine = FakeEngine()
        let runtime = OnnxModelRuntime(engine: engine)
        XCTAssertThrowsError(try runtime.run(input: input())) { e in
            guard case let ForeignError.Failed(reason) = e else { return XCTFail("\(e)") }
            XCTAssertTrue(reason.contains("no model"))
        }
        engine.failure = Broken()
        XCTAssertThrowsError(try runtime.load(path: "/x.onnx")) { e in
            guard case let ForeignError.Failed(reason) = e else { return XCTFail("\(e)") }
            XCTAssertTrue(reason.contains("bad file"))
        }
        XCTAssertNil(runtime.loaded)
    }

    func testAMalformedBatchFailsAsAForeignErrorNotACrash() throws {
        let runtime = OnnxModelRuntime(engine: FakeEngine())
        try runtime.load(path: "/a.onnx")
        var bad = input()
        bad.qtype = []
        XCTAssertThrowsError(try runtime.run(input: bad)) { e in
            guard case let ForeignError.Failed(reason) = e else { return XCTFail("\(e)") }
            XCTAssertTrue(reason.contains("qtype"))
        }
    }

    func testUnloadingFreesTheSession() throws {
        let engine = FakeEngine()
        let runtime = OnnxModelRuntime(engine: engine)
        try runtime.load(path: "/a.onnx")
        runtime.unload()
        XCTAssertTrue(engine.sessions[0].closed)
        XCTAssertNil(runtime.loaded)
    }

    func testAMemoryWarningFreesTheSessionAndTheNextRunOpensItAgain() throws {
        let engine = FakeEngine()
        let runtime = OnnxModelRuntime(engine: engine)
        try runtime.load(path: "/a.onnx")
        runtime.relieveMemory()
        XCTAssertTrue(engine.sessions[0].closed)
        XCTAssertFalse(runtime.isOpen)
        XCTAssertEqual(runtime.loaded, "/a.onnx", "the core still counts on it")
        _ = try runtime.run(input: input())
        XCTAssertEqual(engine.opened, ["/a.onnx", "/a.onnx"])
        XCTAssertTrue(runtime.isOpen)
    }

    func testRunsFromSeveralThreadsShareTheSession() throws {
        let engine = FakeEngine()
        let runtime = OnnxModelRuntime(engine: engine)
        try runtime.load(path: "/a.onnx")
        let one = input()
        DispatchQueue.concurrentPerform(iterations: 40) { _ in _ = try? runtime.run(input: one) }
        XCTAssertEqual(engine.sessions.count, 1)
        XCTAssertEqual(engine.sessions[0].runs.count, 40)
    }

    // MARK: ONNX Runtime

    private var tinyDir: URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("crates/reins-laya/testdata")
    }

    private struct Expected: Decodable {
        var input_ids: [Int64]
        var attention_mask: [Int64]
        var marker_pos: [Int64]
        var marker_mask: [UInt8]
        var qtype: [Int64]
        var batch: UInt32
        var seq_len: UInt32
        var k: UInt32
        var hidden: UInt32
        var logits: [Float]
        var act: [Float]
        var pooled: [Float]
    }

    private func tiny() throws -> (OnnxModelRuntime, Expected, ModelInput) {
        let model = tinyDir.appendingPathComponent("tiny.onnx")
        try XCTSkipUnless(FileManager.default.fileExists(atPath: model.path), "no tiny.onnx at \(model.path)")
        let e = try JSONDecoder().decode(Expected.self, from: Data(contentsOf: tinyDir.appendingPathComponent("tiny_expected.json")))
        let runtime = OnnxModelRuntime()
        try runtime.load(path: model.path)
        let input = ModelInput(
            batch: e.batch, seqLen: e.seq_len, k: e.k, inputIds: e.input_ids, attentionMask: e.attention_mask,
            markerPos: e.marker_pos, markerMask: Data(e.marker_mask), qtype: e.qtype
        )
        return (runtime, e, input)
    }

    private func assertClose(_ a: [Float], _ b: [Float], _ name: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(a.count, b.count, name, file: file, line: line)
        for (x, y) in zip(a, b) { XCTAssertEqual(x, y, accuracy: 1e-4, name, file: file, line: line) }
    }

    func testOnnxRuntimeGivesWhatPythonsOnnxRuntimeGave() throws {
        let (runtime, e, input) = try tiny()
        let out = try runtime.run(input: input)
        XCTAssertEqual(out.hidden, e.hidden)
        assertClose(out.logits, e.logits, "logits")
        assertClose(out.act, e.act, "act")
        assertClose(out.pooled, e.pooled, "pooled")
        runtime.unload()
    }

    func testTheMarkerMaskReachesTheModelAsBooleans() throws {
        let (runtime, e, input) = try tiny()
        var masked = input
        masked.markerMask = Data([1, 1, 0, 1, 0, 1])
        let out = try runtime.run(input: masked)
        XCTAssertEqual(out.logits[2], -1e4, accuracy: 1, "a masked marker scores -1e4")
        XCTAssertEqual(out.logits[4], -1e4, accuracy: 1)
        XCTAssertEqual(out.logits[0], e.logits[0], accuracy: 1e-4)
    }

    func testOnnxRuntimeRunsFromSeveralThreadsAtOnce() throws {
        let (runtime, e, input) = try tiny()
        let failures = NSLock()
        var wrong = 0
        DispatchQueue.concurrentPerform(iterations: 16) { _ in
            let ok = (try? runtime.run(input: input)).map { zip($0.act, e.act).allSatisfy { abs($0 - $1) < 1e-4 } } ?? false
            if !ok { failures.withLock { wrong += 1 } }
        }
        XCTAssertEqual(wrong, 0)
    }

    func testAFileThatIsNotAModelIsAForeignError() throws {
        let junk = FileManager.default.temporaryDirectory.appendingPathComponent("not-a-model.onnx")
        try Data("hello".utf8).write(to: junk)
        XCTAssertThrowsError(try OnnxModelRuntime().load(path: junk.path)) { e in
            guard case ForeignError.Failed = e else { return XCTFail("\(e)") }
        }
    }
}
