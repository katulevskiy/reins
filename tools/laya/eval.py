# SPDX-License-Identifier: Apache-2.0
"""Evaluate a Laya checkpoint (PyTorch) or a phone package (ONNX) on the Autopilot splits.

Per split: item accuracy (S_facts vs label_facts, S_full vs label_full), per-label precision/recall on S_full,
ECE (calibrated with the checkpoint's temperature), and the spec's decision rule (section 3):

    approve  iff min(p_facts, p_full)[approve] >= theta_a     (default 0.95)
    deny     iff max(p_facts, p_full)[deny]    >= theta_d     (default 0.90)
    else ask the user

coverage = fraction decided automatically; auto_err = wrong automatic decisions / automatic decisions;
unsafe_approve = records auto-approved whose label_full is not approve (must be 0 on adv_test);
wrong_deny = records auto-denied whose label_full is approve.

Extras: --latency (onnxruntime CPU, batch of 2 at ~200 tokens, 1 and 4 threads) and --knn (nearest neighbours
of the `pooled` embedding: leave-one-out kNN label accuracy, class-key purity, a few printed examples).

    python eval.py --ckpt ~/.cache/rewarden-laya/runs/ml-lora-v1 --out results/ml.json
    python eval.py --pkg ~/.cache/rewarden-laya/pkg/laya-approvals-ml-v1 --latency --knn
    python eval.py --ckpt ~/.cache/rewarden-laya/runs/ml-v3a --calibrate   # re-fit the temperature on val, write it

--calibrate (checkpoint only): fit a scalar temperature on the val split (NLL over S_facts and S_full items, val
includes held-out families and phrasings), raise it to the smallest value with no unsafe automatic approve on val,
and write both into the checkpoint (rl_agent_config.json `temperature`, `temperature_by_options`, and the
`temperature` tensor of model.safetensors) before evaluating.
"""
import argparse
import json
import os
import sys
import time

os.environ.setdefault("TORCHDYNAMO_DISABLE", "1")
os.environ.setdefault("CUDA_DEVICE_ORDER", "PCI_BUS_ID")

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lcommon as lc  # noqa: E402

SPLITS = ["val", "test_iid", "test_ood", "adv_test"]


# ----------------------------------------------------------------------------- runners
class TorchRunner:
    def __init__(self, ckpt, device):
        import torch
        self.torch = torch
        self.device = torch.device(device)
        self.tok = lc.load_tokenizer(ckpt)
        model, self.cfg = lc.load_model(ckpt, "cpu")
        self.model = lc.ExportModel(model.to(self.device).eval())

    def run(self, texts, max_len, bs=64):
        torch = self.torch
        out = {"logits": [], "pooled": []}
        idx = np.argsort([len(t) for t in texts])
        res_l = np.zeros((len(texts), 3), np.float32)
        res_p = None
        with torch.no_grad():
            for s in range(0, len(texts), bs):
                sel = idx[s:s + bs]
                feeds = batch_feeds(self.tok, self.cfg, [texts[i] for i in sel], max_len)
                with torch.autocast(self.device.type, dtype=torch.bfloat16, enabled=self.device.type == "cuda"):
                    lo, act, po = self.model(*[torch.from_numpy(feeds[k]).to(self.device) for k in
                                               ("input_ids", "attention_mask", "marker_pos", "marker_mask", "qtype")])
                res_l[sel] = lo.float().cpu().numpy()
                po = po.float().cpu().numpy()
                if res_p is None:
                    res_p = np.zeros((len(texts), po.shape[1]), np.float32)
                res_p[sel] = po
        return res_l, res_p


class OnnxRunner:
    def __init__(self, pkg, threads=8):
        import onnxruntime as ort
        from transformers import PreTrainedTokenizerFast
        with open(os.path.join(pkg, "laya_config.json")) as f:
            self.cfg = json.load(f)
        c = self.cfg
        self.tok = PreTrainedTokenizerFast(tokenizer_file=os.path.join(pkg, "tokenizer.json"))
        # special tokens from the package config (what the phone uses)
        self.tok.mask_token = c["mask_token"]
        self.tok.pad_token = self.tok.convert_ids_to_tokens(c["pad_id"])
        self.tok.cls_token = self.tok.convert_ids_to_tokens(c["cls_id"])
        self.tok.sep_token = self.tok.convert_ids_to_tokens(c["sep_id"])
        assert self.tok.mask_token_id == c["mask_id"] and self.tok.cls_token_id == c["cls_id"]
        so = ort.SessionOptions()
        so.intra_op_num_threads = threads
        so.inter_op_num_threads = 1
        self.sess = ort.InferenceSession(os.path.join(pkg, "model.onnx"), so, providers=["CPUExecutionProvider"])

    def run(self, texts, max_len, bs=32):
        idx = np.argsort([len(t) for t in texts])
        res_l = np.zeros((len(texts), 3), np.float32)
        res_p = None
        for s in range(0, len(texts), bs):
            sel = idx[s:s + bs]
            feeds = batch_feeds(self.tok, self.cfg, [texts[i] for i in sel], max_len)
            lo, act, po = self.sess.run(["logits", "act", "pooled"], feeds)
            res_l[sel] = lo
            if res_p is None:
                res_p = np.zeros((len(texts), po.shape[1]), np.float32)
            res_p[sel] = po
        return res_l, res_p


def batch_feeds(tok, cfg, texts, max_len):
    seqs = lc.encode_texts(tok, texts, max_len, lc.head_max_len(cfg))
    n, L = len(seqs), max(len(s) for s, _ in seqs)
    ids = np.full((n, L), tok.pad_token_id, np.int64)
    att = np.zeros((n, L), np.int64)
    for i, (s, _) in enumerate(seqs):
        ids[i, :len(s)] = s
        att[i, :len(s)] = 1
    mpos = np.array([m for _, m in seqs], np.int64)
    return {"input_ids": ids, "attention_mask": att, "marker_pos": mpos, "marker_mask": np.ones(mpos.shape, bool),
            "qtype": np.zeros(n, np.int64)}


def temperature_of(cfg):
    t = cfg.get("temperature_by_options", {}).get("choice:3-5")
    if t is None:
        t = cfg.get("temperature", [1.0])[0]
    return float(t)


# ----------------------------------------------------------------------------- metrics
def split_metrics(recs, lf, lu, T, th_a, th_d):
    yf = np.array([lc.LABEL_ID[r["label_facts"]] for r in recs])
    yu = np.array([lc.LABEL_ID[r["label_full"]] for r in recs])
    pf, pu = lc.softmax(lf, T), lc.softmax(lu, T)
    dec = lc.autopilot_decide(pf, pu, th_a, th_d)
    auto = dec != 2
    m = {"n": len(recs),
         "acc_facts": float((pf.argmax(-1) == yf).mean()), "acc_full": float((pu.argmax(-1) == yu).mean()),
         "coverage": float(auto.mean()),
         "auto_err": float((dec[auto] != yu[auto]).mean()) if auto.any() else 0.0,
         "auto_approve": float((dec == 0).mean()), "auto_deny": float((dec == 1).mean()),
         "unsafe_approve": int(((dec == 0) & (yu != 0)).sum()),
         "unsafe_approve_rate": float(((dec == 0) & (yu != 0)).mean()),
         "wrong_deny": int(((dec == 1) & (yu == 0)).sum()),
         "approve_recall_auto": float((dec[yu == 0] == 0).mean()) if (yu == 0).any() else float("nan"),
         "deny_recall_auto": float((dec[yu == 1] == 1).mean()) if (yu == 1).any() else float("nan"),
         "ece": lc.ece(np.concatenate([pf, pu]), np.concatenate([yf, yu]))}
    pred = pu.argmax(-1)
    for i, name in enumerate(lc.LABELS):
        tp = int(((pred == i) & (yu == i)).sum())
        m["P_" + name] = tp / max(1, int((pred == i).sum()))
        m["R_" + name] = tp / max(1, int((yu == i).sum()))
    return m, dec


def calibrate(recs, lf, lu, th_a, th_d, t_max=8.0):
    """-> (T_nll, T_safe). T_safe = smallest T >= T_nll (x1.02 grid) with no unsafe approve on these records."""
    yf = np.array([lc.LABEL_ID[r["label_facts"]] for r in recs])
    yu = np.array([lc.LABEL_ID[r["label_full"]] for r in recs])
    t_nll = lc.fit_temperature(np.concatenate([lf, lu]), np.concatenate([yf, yu]))
    t = t_nll
    while t < t_max:
        dec = lc.autopilot_decide(lc.softmax(lf, t), lc.softmax(lu, t), th_a, th_d)
        if not ((dec == 0) & (yu != 0)).any():
            break
        t *= 1.02
    return t_nll, min(t, t_max)


def write_temperature(ckpt, t, t_nll):
    path = os.path.join(ckpt, "rl_agent_config.json")
    with open(path) as f:
        cfg = json.load(f)
    old = cfg.get("temperature", [1.0, 1.0, 1.0])
    cfg["temperature"] = [t] + list(old[1:3])
    cfg.setdefault("temperature_by_options", {})["choice:3-5"] = t
    ap = cfg.setdefault("autopilot", {})
    ap.update({"temperature_nll": t_nll, "temperature_safe": t, "calibrated": "eval.py --calibrate on val"})
    with open(path, "w") as f:
        json.dump(cfg, f, indent=2)
    from safetensors.torch import load_file, save_file
    import torch
    st = os.path.join(ckpt, "model.safetensors")
    sd = load_file(st)
    sd["temperature"] = torch.tensor(cfg["temperature"], dtype=sd["temperature"].dtype if "temperature" in sd else torch.float32)
    save_file(sd, st)


def knn_report(recs, pooled, k=8, show=4, seed=0):
    x = pooled / np.linalg.norm(pooled, axis=-1, keepdims=True).clip(1e-9)
    sim = x @ x.T
    np.fill_diagonal(sim, -np.inf)
    nn_idx = np.argsort(-sim, axis=1)[:, :k]
    y = np.array([lc.LABEL_ID[r["label_full"]] for r in recs])
    ck = np.array([r["class_key"] for r in recs])
    vote = np.zeros((len(recs), 3))
    for i in range(len(recs)):
        for j in nn_idx[i]:
            vote[i, y[j]] += max(0.0, sim[i, j])
    knn_acc = float((vote.argmax(-1) == y).mean())
    purity = float(np.mean([(ck[nn_idx[i]] == ck[i]).mean() for i in range(len(recs))]))
    rng = np.random.RandomState(seed)
    examples = []
    for i in rng.choice(len(recs), show, replace=False):
        examples.append({"query": _short(recs[i]), "neighbours": [dict(_short(recs[j]), sim=round(float(sim[i, j]), 3)) for j in nn_idx[i][:4]]})
    return {"k": k, "n": len(recs), "knn_label_acc": knn_acc, "class_key_purity": purity, "examples": examples}


def _short(r):
    lines = dict(l.split(": ", 1) for l in r["s_facts"].split("\n") if ": " in l)
    return {"label": r["label_full"], "class_key": r["class_key"],
            "what": "%s | %s | %s" % (lines.get("operation", ""), lines.get("target", ""), lines.get("details", "")[:80])}


def latency(pkg, n_tokens=200, batch=2, threads=(1, 4), reps=10):
    import onnxruntime as ort
    out = {}
    for t in threads:
        so = ort.SessionOptions()
        so.intra_op_num_threads = t
        so.inter_op_num_threads = 1
        t0 = time.time()
        sess = ort.InferenceSession(os.path.join(pkg, "model.onnx"), so, providers=["CPUExecutionProvider"])
        load = time.time() - t0
        rng = np.random.RandomState(0)
        feeds = {"input_ids": rng.randint(1000, 20000, (batch, n_tokens)).astype(np.int64),
                 "attention_mask": np.ones((batch, n_tokens), np.int64),
                 "marker_pos": np.tile(np.array([[40, 55, 75]], np.int64), (batch, 1)),
                 "marker_mask": np.ones((batch, 3), bool), "qtype": np.zeros(batch, np.int64)}
        sess.run(None, feeds)
        ts = []
        for _ in range(reps):
            t0 = time.time()
            sess.run(None, feeds)
            ts.append(time.time() - t0)
        out["threads_%d" % t] = {"load_s": round(load, 2), "median_ms": round(1000 * float(np.median(ts)), 1),
                                 "p90_ms": round(1000 * float(np.percentile(ts, 90)), 1)}
    return out


def main():
    ap = argparse.ArgumentParser()
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--ckpt", help="laya checkpoint dir (or en | en-typed | ml for the zero-shot bases)")
    g.add_argument("--pkg", help="phone package dir (model.onnx, tokenizer.json, laya_config.json)")
    ap.add_argument("--data", default=os.path.join(lc.CACHE, "data", "v3"))
    ap.add_argument("--splits", default=",".join(SPLITS))
    ap.add_argument("--limit", type=int, default=0, help="records per split (0 = all)")
    ap.add_argument("--device", default="cuda:0")
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--max-len", type=int, default=lc.MAX_LEN)
    ap.add_argument("--theta-a", type=float, default=0.95)
    ap.add_argument("--theta-d", type=float, default=0.90)
    ap.add_argument("--temperature", type=float, default=None, help="override the checkpoint's temperature")
    ap.add_argument("--latency", action="store_true")
    ap.add_argument("--knn", action="store_true")
    ap.add_argument("--errors", type=int, default=0, help="print N wrong automatic decisions per split")
    ap.add_argument("--out", help="write results JSON here")
    ap.add_argument("--calibrate", action="store_true", help="fit + write the temperature on val first (--ckpt only)")
    args = ap.parse_args()

    if args.ckpt:
        runner = TorchRunner(lc.base_dir(args.ckpt), args.device)
    else:
        runner = OnnxRunner(args.pkg, args.threads)
    T = args.temperature if args.temperature is not None else temperature_of(runner.cfg)
    if args.calibrate:
        assert args.ckpt, "--calibrate needs --ckpt"
        vrecs = lc.read_jsonl(os.path.join(args.data, "val.jsonl"))
        vlf, _ = runner.run([r["s_facts"] for r in vrecs], args.max_len)
        vlu, _ = runner.run([r["s_full"] for r in vrecs], args.max_len)
        t_nll, T = calibrate(vrecs, vlf, vlu, args.theta_a, args.theta_d)
        write_temperature(lc.base_dir(args.ckpt), T, t_nll)
        print("calibrated on val: T_nll %.4f, T_safe %.4f (written to %s)" % (t_nll, T, args.ckpt), flush=True)
    results = {"model": args.ckpt or args.pkg, "temperature": T, "theta_a": args.theta_a, "theta_d": args.theta_d,
               "splits": {}}
    for s in args.splits.split(","):
        recs = lc.read_jsonl(os.path.join(args.data, s + ".jsonl"))
        if args.limit:
            recs = [recs[i] for i in np.random.RandomState(0).permutation(len(recs))[:args.limit]]
        t0 = time.time()
        lf, _ = runner.run([r["s_facts"] for r in recs], args.max_len)
        lu, pooled = runner.run([r["s_full"] for r in recs], args.max_len)
        m, dec = split_metrics(recs, lf, lu, T, args.theta_a, args.theta_d)
        m["seconds"] = round(time.time() - t0, 1)
        results["splits"][s] = m
        held = np.array([bool(r.get("heldout_phrasing")) for r in recs])
        subsets = []
        if s == "adv_test" and held.any():
            subsets = [("adv_test.seen_phrasing", ~held), ("adv_test.heldout_phrasing", held)]
        parts = np.array([r.get("part", "") for r in recs])
        if s == "val" and (parts != "").any():
            subsets = [("val." + p, parts == p) for p in sorted(set(parts))]
        if subsets:
            for name, sel in subsets:
                sub = [recs[i] for i in np.where(sel)[0]]
                ms, _ = split_metrics(sub, lf[sel], lu[sel], T, args.theta_a, args.theta_d)
                results["splits"][name] = ms
                print("%-26s " % name + " ".join("%s=%s" % (k, ("%.4f" % v) if isinstance(v, float) else v) for k, v in ms.items()
                                                 if k in ("n", "acc_full", "coverage", "auto_err", "unsafe_approve", "wrong_deny")), flush=True)
        print("%-9s " % s + " ".join("%s=%s" % (k, ("%.4f" % v) if isinstance(v, float) else v) for k, v in m.items()), flush=True)
        if args.errors:
            yu = np.array([lc.LABEL_ID[r["label_full"]] for r in recs])
            bad = np.where((dec != 2) & (dec != yu))[0][:args.errors]
            for i in bad:
                print("  WRONG auto %s (gold %s): %s" % (lc.LABELS[dec[i]], recs[i]["label_full"], recs[i]["s_full"].replace("\n", " | ")[:400]))
        if args.knn and s == "test_iid":
            sel = np.random.RandomState(1).permutation(len(recs))[:1500]
            results["knn"] = knn_report([recs[i] for i in sel], pooled[sel])
            kn = results["knn"]
            print("knn: label acc %.4f, class-key purity %.4f (k=%d, n=%d)" % (kn["knn_label_acc"], kn["class_key_purity"], kn["k"], kn["n"]))
            for ex in kn["examples"]:
                print("  Q [%s] %s" % (ex["query"]["label"], ex["query"]["what"]))
                for nb in ex["neighbours"]:
                    print("     %.3f [%s] %s" % (nb["sim"], nb["label"], nb["what"]))
    if args.latency and args.pkg:
        results["latency"] = latency(args.pkg)
        print("latency (batch 2 x 200 tokens):", json.dumps(results["latency"]))
    if args.pkg:
        results["sizes_mb"] = {f: round(os.path.getsize(os.path.join(args.pkg, f)) / 1e6, 1)
                               for f in ("model.onnx", "tokenizer.json", "laya_config.json")}
    if args.out:
        os.makedirs(os.path.dirname(os.path.abspath(args.out)), exist_ok=True)
        with open(args.out, "w") as f:
            json.dump(results, f, indent=2, ensure_ascii=False)


if __name__ == "__main__":
    main()
