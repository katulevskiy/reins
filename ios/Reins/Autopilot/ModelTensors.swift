import Foundation

/// One batch for the model as flat arrays, checked against the shapes of spec §6.2 (row-major).
struct PackedInput: Equatable {
    var batch: Int
    var seqLen: Int
    var k: Int
    /// int64 [batch, seqLen]
    var inputIds: [Int64]
    /// int64 [batch, seqLen]
    var attentionMask: [Int64]
    /// int64 [batch, k]
    var markerPos: [Int64]
    /// bool [batch, k], one byte each (0 or 1)
    var markerMask: [UInt8]
    /// int64 [batch]
    var qtype: [Int64]

    var tokenShape: [Int64] { [Int64(batch), Int64(seqLen)] }
    var markerShape: [Int64] { [Int64(batch), Int64(k)] }
    var batchShape: [Int64] { [Int64(batch)] }
}

/// A float tensor the model returned: its values and its shape.
struct RawTensor: Equatable {
    var values: [Float]
    var shape: [Int64]
}

/// The three outputs of a forward pass, as the runtime handed them back.
struct RawOutput: Equatable {
    var logits: RawTensor
    var act: RawTensor
    var pooled: RawTensor
}

/// Turns the core's records into tensors and the model's tensors back into a record (the Android app's
/// `ModelTensors`). Pure, so it is tested without ONNX Runtime.
enum ModelTensors {
    static let inputIds = "input_ids"
    static let attentionMask = "attention_mask"
    static let markerPos = "marker_pos"
    static let markerMask = "marker_mask"
    static let qtype = "qtype"
    static let logits = "logits"
    static let act = "act"
    static let pooled = "pooled"

    /// The outputs every run asks for, in this order.
    static let outputs = [logits, act, pooled]

    /// A batch or an output whose sizes do not add up; the message names the tensor.
    struct Mismatch: Error, Equatable, CustomStringConvertible {
        var description: String
    }

    /// Checks every array has the size its shape says; a mismatch is the core's bug, not something to guess around.
    static func pack(_ input: ModelInput) throws -> PackedInput {
        let batch = Int(input.batch)
        let seqLen = Int(input.seqLen)
        let k = Int(input.k)
        guard batch > 0, seqLen > 0, k > 0 else {
            throw Mismatch(description: "empty batch (\(batch) × \(seqLen), k = \(k))")
        }
        func check(_ name: String, _ size: Int, _ expected: Int) throws {
            guard size == expected else { throw Mismatch(description: "\(name) has \(size) values, expected \(expected)") }
        }
        try check(inputIds, input.inputIds.count, batch * seqLen)
        try check(attentionMask, input.attentionMask.count, batch * seqLen)
        try check(markerPos, input.markerPos.count, batch * k)
        try check(markerMask, input.markerMask.count, batch * k)
        try check(qtype, input.qtype.count, batch)
        return PackedInput(
            batch: batch, seqLen: seqLen, k: k,
            inputIds: input.inputIds,
            attentionMask: input.attentionMask,
            markerPos: input.markerPos,
            // ONNX booleans are one byte, 0 or 1; anything else the core sent counts as true.
            markerMask: input.markerMask.map { $0 == 0 ? 0 : 1 },
            qtype: input.qtype
        )
    }

    /// `logits` [B,K], `act` [B,2], `pooled` [B,H]; the hidden size comes from the pooled tensor's last dimension.
    static func unpack(_ input: PackedInput, _ raw: RawOutput) throws -> ModelOutput {
        let b = input.batch
        guard raw.logits.values.count == b * input.k else {
            throw Mismatch(description: "\(logits) has \(raw.logits.values.count) values, expected \(b * input.k)")
        }
        guard raw.act.values.count == b * 2 else {
            throw Mismatch(description: "\(act) has \(raw.act.values.count) values, expected \(b * 2)")
        }
        let hidden = Int(raw.pooled.shape.last ?? 0)
        guard hidden > 0, raw.pooled.values.count == b * hidden else {
            throw Mismatch(description: "\(pooled) has \(raw.pooled.values.count) values in shape \(raw.pooled.shape), expected \(b) × hidden")
        }
        return ModelOutput(logits: raw.logits.values, act: raw.act.values, pooled: raw.pooled.values, hidden: UInt32(hidden))
    }
}
