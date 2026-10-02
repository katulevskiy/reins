# SPDX-License-Identifier: Apache-2.0
"""Export a (fine-tuned) Laya checkpoint to the phone package of spec section 6.

ONNX I/O (section 6.2):
  inputs   input_ids int64 [B,L], attention_mask int64 [B,L], marker_pos int64 [B,K], marker_mask bool [B,K],
           qtype int64 [B]
  outputs  logits f32 [B,K] (uncalibrated), act f32 [B,2] (raw logits), pooled f32 [B,H] (the [CLS] state
           after the decision head = h[:,0] after the head layers; the kNN embedding)

Steps: fp32 export (legacy TorchScript exporter, dynamic batch/seq/k) -> int8 dynamic quantization
(MatMul, optionally Gather for the embedding table) -> parity check against PyTorch -> package
`<pkg_root>/<id>/{model.onnx, tokenizer.json, laya_config.json}` + SHA256SUMS.

    python export_onnx.py --ckpt ~/.cache/rewarden-laya/runs/ml-v1 --id laya-approvals-ml-v1
"""
import argparse
import hashlib
import json
import os
import shutil
import sys
import time

os.environ.setdefault("TORCHDYNAMO_DISABLE", "1")
os.environ.setdefault("CUDA_VISIBLE_DEVICES", "")

import numpy as np
import torch
import torch.nn as nn

# nn.TransformerEncoderLayer's fused inference fast path (aten::_transformer_encoder_layer_fwd) has no ONNX
# symbolic; the regular path computes the same thing.
torch.backends.mha.set_fastpath_enabled(False)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lcommon as lc  # noqa: E402
from lcommon import ExportModel  # noqa: E402
import sequence as sq  # noqa: E402

INPUTS = ["input_ids", "attention_mask", "marker_pos", "marker_mask", "qtype"]
OUTPUTS = ["logits", "act", "pooled"]


def sample_inputs(tok, cfg, texts, max_len):
    seqs = lc.encode_texts(tok, texts, max_len, lc.head_max_len(cfg))
    n, L = len(seqs), max(len(s[0]) for s in seqs)
    ids = np.full((n, L), tok.pad_token_id, np.int64)
    att = np.zeros((n, L), np.int64)
    for i, (s, _) in enumerate(seqs):
        ids[i, :len(s)] = s
        att[i, :len(s)] = 1
    mpos = np.array([m for _, m in seqs], np.int64)
    return {"input_ids": ids, "attention_mask": att, "marker_pos": mpos, "marker_mask": np.ones(mpos.shape, bool),
            "qtype": np.zeros(n, np.int64)}


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def export_fp32(model, feeds, path, opset):
    em = ExportModel(model).eval()
    args = tuple(torch.from_numpy(feeds[k]) for k in INPUTS)
    dyn = {"input_ids": {0: "batch", 1: "seq"}, "attention_mask": {0: "batch", 1: "seq"},
           "marker_pos": {0: "batch", 1: "k"}, "marker_mask": {0: "batch", 1: "k"}, "qtype": {0: "batch"},
           "logits": {0: "batch", 1: "k"}, "act": {0: "batch"}, "pooled": {0: "batch"}}
    with torch.no_grad():
        torch.onnx.export(em, args, path, input_names=INPUTS, output_names=OUTPUTS, dynamic_axes=dyn,
                          opset_version=opset, do_constant_folding=True, dynamo=False)


@torch.no_grad()
def smooth_fold(model, tok, cfg, texts, max_len, alpha=0.5, bs=16):
    """SmoothQuant with exact folding (no graph change): per input channel j of a quantized MatMul, divide the
    activation by s_j and multiply the weight column by s_j, s_j = max|x_j|^alpha / max|W_j|^(1-alpha), folding
    1/s_j into the op that produces the activation. ModernBERT/mmBERT have no biases, so each fold is exact:
      mlp.Wo   <- input act(a) * g is linear in g      -> divide the gate rows of mlp.Wi
      attn.Wo  <- attention output is linear in V      -> divide the V rows of attn.Wqkv
      mlp.Wi   <- input is mlp_norm(x)                 -> divide the mlp_norm weight
      attn.Wqkv<- input is attn_norm(x) (not layer 0)  -> divide the attn_norm weight
    Removes the activation outliers that wreck per-tensor dynamic int8 activations. Returns the number of folds."""
    enc = model.encoder
    layers = enc.layers
    stats = {}

    def hook(key):
        def f(mod, inp, out):
            x = inp[0].detach().float().abs().reshape(-1, inp[0].shape[-1]).amax(0)
            stats[key] = torch.maximum(stats[key], x) if key in stats else x
        return f

    hs = []
    for i, l in enumerate(layers):
        hs += [l.mlp.Wo.register_forward_hook(hook((i, "mlp_wo"))), l.attn.Wo.register_forward_hook(hook((i, "attn_wo"))),
               l.mlp.Wi.register_forward_hook(hook((i, "mlp_wi"))), l.attn.Wqkv.register_forward_hook(hook((i, "wqkv")))]
    torch_batches(model, tok, cfg, texts, max_len, bs)
    for h in hs:
        h.remove()

    def scale(xmax, w):  # w: [out, in]
        wmax = w.abs().amax(0).float().clamp_min(1e-5)
        s = (xmax.clamp_min(1e-5) ** alpha) / (wmax ** (1 - alpha))
        return s.clamp(1e-3, 1e3)

    n = 0
    for i, l in enumerate(layers):
        H = l.attn.Wo.weight.shape[1]
        # mlp.Wo <- gate half of Wi
        s = scale(stats[(i, "mlp_wo")], l.mlp.Wo.weight)
        l.mlp.Wo.weight.mul_(s[None, :].to(l.mlp.Wo.weight.dtype))
        I = l.mlp.Wo.weight.shape[1]
        l.mlp.Wi.weight[I:].div_(s[:, None].to(l.mlp.Wi.weight.dtype))
        # attn.Wo <- V rows of Wqkv
        s = scale(stats[(i, "attn_wo")], l.attn.Wo.weight)
        l.attn.Wo.weight.mul_(s[None, :])
        l.attn.Wqkv.weight[2 * H:].div_(s[:, None])
        n += 2
        # Wi <- mlp_norm weight (the Wi gate rows were just rescaled; their input stats are unchanged)
        if isinstance(l.mlp_norm, nn.LayerNorm) and l.mlp_norm.weight is not None:
            s = scale(stats[(i, "mlp_wi")], l.mlp.Wi.weight)
            l.mlp.Wi.weight.mul_(s[None, :])
            l.mlp_norm.weight.div_(s)
            n += 1
        if isinstance(l.attn_norm, nn.LayerNorm) and l.attn_norm.weight is not None:
            s = scale(stats[(i, "wqkv")], l.attn.Wqkv.weight)
            l.attn.Wqkv.weight.mul_(s[None, :])
            l.attn_norm.weight.div_(s)
            n += 1
    return n


QUANT_MODES = ("smooth-hybrid", "smooth-dynamic", "hybrid", "dynamic", "dynamic-fp32-wo", "weight-only")


def quantize(src, dst, mode: str = "smooth-hybrid", gather: bool = True):
    """int8 quantization. ModernBERT/mmBERT carry large activation outliers into the attention and MLP output
    projections (`attn/Wo`, `mlp/Wo`): per-tensor dynamic activation quantization there wrecks the logits
    (argmax agreement ~73% with plain quantize_dynamic). Modes (the smooth-* ones expect a model that went through
    smooth_fold() before the fp32 export, main() does that):

      smooth-hybrid    SmoothQuant-folded model; dynamic int8 (per-channel weights, dynamic uint8 activations)
                       for every weight MatMul except `mlp/Wo`, which gets 8-bit weight-only MatMulNBits (block 32,
                       fp32 activations); embedding Gather int8. ~99% argmax agreement, 4x smaller; the default.
      smooth-dynamic   SmoothQuant-folded model, plain per-channel dynamic int8 everywhere (~98%, fastest).
      hybrid           like smooth-hybrid without the fold, both output projections weight-only (~98.6%).
      dynamic-fp32-wo  like hybrid but the output projections stay fp32 (standard ONNX ops only, ~55 MB larger).
      dynamic          plain quantize_dynamic on every MatMul + Gather (inaccurate, kept for comparison).
      weight-only      8-bit MatMulNBits everywhere + int8 Gather (most accurate, slower than fp32 on x86 CPUs).
    """
    import onnx
    from onnxruntime.quantization import QuantType, quantize_dynamic
    from onnxruntime.quantization.matmul_nbits_quantizer import DefaultWeightOnlyQuantConfig, MatMulNBitsQuantizer
    m = onnx.load(src)
    inits = {i.name for i in m.graph.initializer}
    mm = [n for n in m.graph.node if n.op_type in ("MatMul", "Gemm")]
    act_mm = [n.name for n in mm if n.input[1] not in inits]  # attention scores, RoPE: activation x activation
    out_proj = [n.name for n in mm if "/mlp/Wo/" in n.name or "/attn/Wo/" in n.name]
    if mode == "smooth-hybrid":
        out_proj = [n.name for n in mm if "/mlp/Wo/" in n.name]
    elif mode == "smooth-dynamic":
        out_proj = []
    gops = ["Gather"] if gather else []
    if mode == "dynamic":
        quantize_dynamic(src, dst, weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul", "Gemm"] + gops)
        return
    if mode == "smooth-dynamic":
        quantize_dynamic(src, dst, weight_type=QuantType.QInt8, op_types_to_quantize=["MatMul", "Gemm"] + gops,
                         per_channel=True, nodes_to_exclude=act_mm)
        return
    stage1 = dst + ".stage1.onnx"
    if mode == "weight-only":
        quantize_dynamic(src, stage1, weight_type=QuantType.QInt8, op_types_to_quantize=gops or ["Gather"], per_channel=True,
                         nodes_to_exclude=[] if gather else [n.name for n in m.graph.node])
        keep = None
    else:
        quantize_dynamic(src, stage1 if mode in ("hybrid", "smooth-hybrid") else dst, weight_type=QuantType.QInt8,
                         op_types_to_quantize=["MatMul", "Gemm"] + gops, per_channel=True, nodes_to_exclude=act_mm + out_proj)
        if mode == "dynamic-fp32-wo":
            return
        keep = out_proj
    qc = DefaultWeightOnlyQuantConfig(block_size=32, is_symmetric=True, bits=8, op_types_to_quantize=("MatMul",))
    q = MatMulNBitsQuantizer(onnx.load(stage1), algo_config=qc, nodes_to_include=keep)
    q.process()
    q.model.save_model_to_file(dst, use_external_data_format=False)
    os.remove(stage1)


def ort_session(path, threads=4):
    import onnxruntime as ort
    so = ort.SessionOptions()
    so.intra_op_num_threads = threads
    so.inter_op_num_threads = 1
    so.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL
    return ort.InferenceSession(path, so, providers=["CPUExecutionProvider"])


def run_batches(sess, tok, cfg, texts, max_len, bs=16):
    outs = {k: [] for k in OUTPUTS}
    order = np.argsort([len(t) for t in texts])
    for s in range(0, len(texts), bs):
        sel = order[s:s + bs]
        feeds = sample_inputs(tok, cfg, [texts[i] for i in sel], max_len)
        res = sess.run(OUTPUTS, feeds)
        for k, v in zip(OUTPUTS, res):
            outs[k].append((sel, v))
    final = {}
    for k in OUTPUTS:
        dim = outs[k][0][1].shape[1]
        arr = np.zeros((len(texts), dim), np.float32)
        for sel, v in outs[k]:
            arr[sel] = v
        final[k] = arr
    return final


def torch_batches(model, tok, cfg, texts, max_len, bs=16):
    em = ExportModel(model).eval()
    outs = {k: np.zeros((len(texts), 0), np.float32) for k in OUTPUTS}
    res = {k: [None] * len(texts) for k in OUTPUTS}
    order = np.argsort([len(t) for t in texts])
    with torch.no_grad():
        for s in range(0, len(texts), bs):
            sel = order[s:s + bs]
            feeds = sample_inputs(tok, cfg, [texts[i] for i in sel], max_len)
            o = em(*[torch.from_numpy(feeds[k]) for k in INPUTS])
            for k, v in zip(OUTPUTS, o):
                for r, i in enumerate(sel):
                    res[k][i] = v[r].numpy()
    return {k: np.stack(v) for k, v in res.items()}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ckpt", required=True)
    ap.add_argument("--id", required=True, help="package id, e.g. laya-approvals-ml-v1")
    ap.add_argument("--pkg-root", default=os.path.join(lc.CACHE, "pkg"))
    ap.add_argument("--work", default=os.path.join(lc.CACHE, "onnx"))
    ap.add_argument("--max-len", type=int, default=lc.MAX_LEN)
    ap.add_argument("--opset", type=int, default=17)
    ap.add_argument("--attn", default="sdpa", help="sdpa | eager")
    ap.add_argument("--quant", default="smooth-hybrid", choices=QUANT_MODES)
    ap.add_argument("--smooth-alpha", type=float, default=0.8, help="SmoothQuant migration strength (smooth-* modes)")
    ap.add_argument("--smooth-n", type=int, default=200, help="val records (x2 views) for the SmoothQuant statistics")
    ap.add_argument("--no-gather", action="store_true", help="do not quantize the embedding Gather")
    ap.add_argument("--parity-data", default=os.path.join(lc.CACHE, "data", "v4"),
                    help="dataset dir: parity runs on S_facts and S_full of test_iid, test_ood and adv_test records")
    ap.add_argument("--parity-n", type=int, default=300, help="records per split")
    ap.add_argument("--reuse-fp32", action="store_true", help="skip the export if the fp32 file exists")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--no-recalibrate", action="store_true",
                    help="keep the checkpoint's temperature (default: raise it to the int8 model's safety minimum on val)")
    args = ap.parse_args()
    torch.set_num_threads(args.threads)

    tok = lc.load_tokenizer(args.ckpt)
    model, cfg = lc.load_model(args.ckpt, "cpu", attn=args.attn)
    model.eval()
    os.makedirs(args.work, exist_ok=True)
    fp32 = os.path.join(args.work, args.id + ".fp32.onnx")
    int8 = os.path.join(args.work, args.id + ".int8.onnx")

    texts = [c["s_full"] for c in sq.golden_inputs()]
    feeds = sample_inputs(tok, cfg, texts[:3], args.max_len)

    # parity texts: golden texts + S_facts and S_full of test_iid, test_ood and adv_test records
    ptexts = list(texts)
    recs = []
    for split in ("test_iid", "test_ood", "adv_test"):
        path = os.path.join(args.parity_data, split + ".jsonl")
        if os.path.exists(path):
            rs = lc.read_jsonl(path)
            recs += [rs[i] for i in np.random.RandomState(0).permutation(len(rs))[:args.parity_n]]
    n0, nr = len(texts), len(recs)
    ptexts += [r["s_facts"] for r in recs] + [r["s_full"] for r in recs]
    labels = np.array([lc.LABEL_ID[r["label_full"]] for r in recs]) if recs else None
    T = float(cfg.get("temperature_by_options", {}).get("choice:3-5", cfg.get("temperature", [1.0])[0]))
    t0 = time.time()
    ref = torch_batches(model, tok, cfg, ptexts, args.max_len)  # reference: the unmodified PyTorch model
    print("PyTorch reference on %d texts %.0fs" % (len(ptexts), time.time() - t0), flush=True)

    if args.quant.startswith("smooth"):
        vr = lc.read_jsonl(os.path.join(args.parity_data, "val.jsonl"))
        vr = [vr[i] for i in np.random.RandomState(5).permutation(len(vr))[:args.smooth_n]]
        t0 = time.time()
        n = smooth_fold(model, tok, cfg, [r["s_facts"] for r in vr] + [r["s_full"] for r in vr], args.max_len, args.smooth_alpha)
        print("SmoothQuant fold (alpha %.2f, %d folds, val statistics) %.0fs" % (args.smooth_alpha, n, time.time() - t0), flush=True)
    t0 = time.time()
    if not (args.reuse_fp32 and os.path.exists(fp32)):
        export_fp32(model, feeds, fp32, args.opset)
    print("fp32 export %.0fs, %.1f MB" % (time.time() - t0, os.path.getsize(fp32) / 1e6), flush=True)
    t0 = time.time()
    quantize(fp32, int8, args.quant, gather=not args.no_gather)
    print("int8 quantize %.0fs, %.1f MB" % (time.time() - t0, os.path.getsize(int8) / 1e6), flush=True)

    parity = {}
    for name, path in (("fp32", fp32), ("int8", int8)):
        out = run_batches(ort_session(path, args.threads), tok, cfg, ptexts, args.max_len)
        pr, po = lc.softmax(ref["logits"], T), lc.softmax(out["logits"], T)
        cos = (ref["pooled"] * out["pooled"]).sum(-1) / (np.linalg.norm(ref["pooled"], axis=-1) * np.linalg.norm(out["pooled"], axis=-1))
        d = {"max_abs_prob_diff": float(np.abs(pr - po).max()), "mean_abs_prob_diff": float(np.abs(pr - po).mean()),
             "argmax_agree": float((pr.argmax(-1) == po.argmax(-1)).mean()), "n": len(ptexts),
             "max_abs_logit_diff": float(np.abs(ref["logits"] - out["logits"]).max()),
             "pooled_cos_min": float(cos.min()), "pooled_cos_mean": float(cos.mean())}
        if labels is not None:
            # the decision the phone takes (spec section 3 rule at 0.95 / 0.90), PyTorch vs ONNX
            dr = lc.autopilot_decide(pr[n0:n0 + nr], pr[n0 + nr:])
            do = lc.autopilot_decide(po[n0:n0 + nr], po[n0 + nr:])
            d["decision_agree"] = float((dr == do).mean())
            d["acc_full_torch"] = float((pr[n0 + nr:].argmax(-1) == labels).mean())
            d["acc_full_onnx"] = float((po[n0 + nr:].argmax(-1) == labels).mean())
            d["unsafe_approve_onnx"] = int(((do == 0) & (labels != 0)).sum())
        parity[name] = d
        print(name, json.dumps(d), flush=True)

    # ---- calibrate what ships: the int8 model's own safety temperature on val (quantization noise can move a
    # probability across the approve threshold). The package temperature is max(checkpoint T, int8 T_safe).
    T_pkg, calib = T, {"checkpoint": T}
    if not args.no_recalibrate and os.path.exists(os.path.join(args.parity_data, "val.jsonl")):
        import eval as ev
        vr = lc.read_jsonl(os.path.join(args.parity_data, "val.jsonl"))
        sess = ort_session(int8, args.threads)
        t0 = time.time()
        vo = run_batches(sess, tok, cfg, [r["s_facts"] for r in vr] + [r["s_full"] for r in vr], args.max_len, bs=32)
        lf, lu = vo["logits"][:len(vr)], vo["logits"][len(vr):]
        t_nll, t_safe = ev.calibrate(vr, lf, lu, 0.95, 0.90, t_max=12.0)
        # start at max(checkpoint T, int8 T_safe) and step up until val has no unsafe automatic approve
        yu = np.array([lc.LABEL_ID[r["label_full"]] for r in vr])
        T_pkg = max(T, t_safe)
        while T_pkg < 12.0:
            dec = lc.autopilot_decide(lc.softmax(lf, T_pkg), lc.softmax(lu, T_pkg))
            if not ((dec == 0) & (yu != 0)).any():
                break
            T_pkg *= 1.02
        calib.update({"int8_val_t_nll": t_nll, "int8_val_t_safe": t_safe, "package": T_pkg})
        print("int8 calibration on val (%d records, %.0fs): T_nll %.4f T_safe %.4f -> package T %.4f (checkpoint %.4f)" % (
            len(vr), time.time() - t0, t_nll, t_safe, T_pkg, T), flush=True)

    # ---- package
    pdir = os.path.join(args.pkg_root, args.id)
    os.makedirs(pdir, exist_ok=True)
    shutil.copyfile(int8, os.path.join(pdir, "model.onnx"))
    shutil.copyfile(os.path.join(args.ckpt, "tokenizer", "tokenizer.json"), os.path.join(pdir, "tokenizer.json"))
    sp = sq.special_ids(tok)
    lcfg = {"id": args.id, "base": cfg.get("encoder"), "max_len": args.max_len, "head_max_len": lc.head_max_len(cfg),
            "hidden": model.encoder.config.hidden_size,
            "temperature": [T_pkg] + list(cfg.get("temperature", [T, 1.0, 1.0])[1:3]),
            "temperature_by_options": dict(cfg.get("temperature_by_options", {}), **{"choice:3-5": T_pkg}),
            "cls_id": sp.cls_id, "sep_id": sp.sep_id, "mask_id": sp.mask_id, "pad_id": sp.pad_id,
            "mask_token": sp.mask_token, "options": sq.OPTIONS, "qtype": sq.QTYPE_CHOICE, "quantization": args.quant,
            "smooth_alpha": args.smooth_alpha if args.quant.startswith("smooth") else None,
            "question": sq.QUESTION}
    with open(os.path.join(pdir, "laya_config.json"), "w") as f:
        json.dump(lcfg, f, indent=2, ensure_ascii=False)
        f.write("\n")
    files = ["model.onnx", "tokenizer.json", "laya_config.json"]
    with open(os.path.join(pdir, "SHA256SUMS"), "w") as f:
        for fn in files:
            f.write("%s  %s\n" % (sha256(os.path.join(pdir, fn)), fn))
    with open(os.path.join(args.work, args.id + ".parity.json"), "w") as f:
        json.dump({"quant": args.quant, "calibration": calib, "parity": parity, "fp32_mb": os.path.getsize(fp32) / 1e6, "int8_mb": os.path.getsize(int8) / 1e6}, f, indent=2)
    print("package", pdir)
    for fn in files:
        print("  %-16s %8.1f MB" % (fn, os.path.getsize(os.path.join(pdir, fn)) / 1e6))
    print(open(os.path.join(pdir, "SHA256SUMS")).read())


if __name__ == "__main__":
    main()
