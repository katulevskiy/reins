package dev.reins.android.autopilot

import dev.reins.core.ModelInput
import dev.reins.core.ModelOutput

/** One batch for the model as flat arrays, checked against the shapes of spec §6.2 (row-major). */
class PackedInput(
    val batch: Int,
    val seqLen: Int,
    val k: Int,
    /** int64 [batch, seqLen] */
    val inputIds: LongArray,
    /** int64 [batch, seqLen] */
    val attentionMask: LongArray,
    /** int64 [batch, k] */
    val markerPos: LongArray,
    /** bool [batch, k], one byte each (0 or 1) */
    val markerMask: ByteArray,
    /** int64 [batch] */
    val qtype: LongArray,
) {
    val tokenShape: LongArray get() = longArrayOf(batch.toLong(), seqLen.toLong())
    val markerShape: LongArray get() = longArrayOf(batch.toLong(), k.toLong())
    val batchShape: LongArray get() = longArrayOf(batch.toLong())
}

/** A float tensor the model returned: its values and its shape. */
class RawTensor(val values: FloatArray, val shape: LongArray)

/** The three outputs of a forward pass, as the runtime handed them back. */
class RawOutput(val logits: RawTensor, val act: RawTensor, val pooled: RawTensor)

/** Turns the core's records into tensors and the model's tensors back into a record. Pure, so it is tested on the JVM. */
object ModelTensors {
    const val INPUT_IDS = "input_ids"
    const val ATTENTION_MASK = "attention_mask"
    const val MARKER_POS = "marker_pos"
    const val MARKER_MASK = "marker_mask"
    const val QTYPE = "qtype"
    const val LOGITS = "logits"
    const val ACT = "act"
    const val POOLED = "pooled"

    /** Checks every array has the size its shape says; a mismatch is the core's bug, not something to guess around. */
    fun pack(input: ModelInput): PackedInput {
        val batch = input.batch.toInt()
        val seqLen = input.seqLen.toInt()
        val k = input.k.toInt()
        require(batch > 0 && seqLen > 0 && k > 0) { "empty batch ($batch × $seqLen, k = $k)" }
        fun check(name: String, size: Int, expected: Int) = require(size == expected) { "$name has $size values, expected $expected" }
        check(INPUT_IDS, input.inputIds.size, batch * seqLen)
        check(ATTENTION_MASK, input.attentionMask.size, batch * seqLen)
        check(MARKER_POS, input.markerPos.size, batch * k)
        check(MARKER_MASK, input.markerMask.size, batch * k)
        check(QTYPE, input.qtype.size, batch)
        return PackedInput(
            batch, seqLen, k,
            input.inputIds.toLongArray(),
            input.attentionMask.toLongArray(),
            input.markerPos.toLongArray(),
            input.markerMask.copyOf(),
            input.qtype.toLongArray(),
        )
    }

    /** `logits` [B,K], `act` [B,2], `pooled` [B,H]; the hidden size comes from the pooled tensor's last dimension. */
    fun unpack(input: PackedInput, raw: RawOutput): ModelOutput {
        val b = input.batch
        require(raw.logits.values.size == b * input.k) { "$LOGITS has ${raw.logits.values.size} values, expected ${b * input.k}" }
        require(raw.act.values.size == b * 2) { "$ACT has ${raw.act.values.size} values, expected ${b * 2}" }
        val hidden = raw.pooled.shape.lastOrNull()?.toInt() ?: 0
        require(hidden > 0 && raw.pooled.values.size == b * hidden) {
            "$POOLED has ${raw.pooled.values.size} values in shape ${raw.pooled.shape.toList()}, expected $b × hidden"
        }
        return ModelOutput(raw.logits.values.toList(), raw.act.values.toList(), raw.pooled.values.toList(), hidden.toUInt())
    }
}
