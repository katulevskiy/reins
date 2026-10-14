# SPDX-License-Identifier: Apache-2.0
"""Fine-tune a Laya checkpoint on the Autopilot approval dataset.

LoRA (peft) on the encoder's attention and MLP projections (Wqkv, Wo, Wi; r=16 by default) plus full
training of the decision head (2 transformer layers), type embedding, scorer and act head; or `--full`
for a full fine-tune. Objective: Laya's strictly proper score (log + 0.5 spherical) on the 3 options,
one item per situation variant (S_facts with label_facts, S_full with label_full). An auxiliary linear
probe on the pooled [CLS] state (label + class key) shapes the embedding the phone uses for kNN; the
probe itself is not exported.

The val split (gen_data.py) mixes in-distribution records with held-out families and held-out adversarial
phrasings. Every --eval-every steps the model is scored on it: a temperature is fitted (NLL) and the state with the
lowest calibrated val NLL (mean over the val parts) is kept (early stopping on unfamiliar situations, not on the training distribution).
After training: the best state is restored, LoRA merged, the temperature fitted on val (NLL) and then raised, if
needed, to the smallest value with no unsafe automatic approve on val (`--safe-temperature`); a checkpoint in
Laya's layout is written (rl_agent_config.json, encoder/, tokenizer/, model.safetensors), loadable by laya.Agent
and by export_onnx.py.

With `--risk-data` (gen_risk.py, docs/autopilot-risk.md) the model also gets a risk head (a LayerNorm and a linear
layer on `pooled`, 4 severity classes) trained jointly: the risk records join the approval records (both heads learn
from them), v7 records carry no risk label (masked). Model selection adds the risk val NLL; after training the risk
temperature is fitted on the risk val split and the Overnight threshold on P(catastrophic) is the largest value that
still holds `--risk-recall` of val's catastrophic records under the two-pass rule (lcommon.risk_combine). Both go
into the checkpoint's config (`risk`), with the metrics on every risk split and the golden set.

    python finetune.py --base ml --data ~/.cache/reins-laya/data/v3 --out ~/.cache/reins-laya/runs/ml-v3
    python finetune.py --base ~/.cache/reins-laya/runs/ml-v7-s0 --data ~/.cache/reins-laya/data/v7 \
        --risk-data ~/.cache/reins-laya/data/risk-v1 --out ~/.cache/reins-laya/runs/ml-v8-risk --lr 5e-5 --epochs 1
"""
import argparse
import json
import math
import os
import random
import shutil
import sys
import time

os.environ.setdefault("TORCHDYNAMO_DISABLE", "1")
os.environ.setdefault("CUDA_DEVICE_ORDER", "PCI_BUS_ID")

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import lcommon as lc  # noqa: E402


def token_batches(items, max_tokens, max_seqs, rng, shuffle=True):
    order = list(range(len(items)))
    if shuffle:
        rng.shuffle(order)
    batches = []
    chunk = 4096
    for s in range(0, len(order), chunk):
        part = sorted(order[s:s + chunk], key=lambda i: len(items[i]["ids"]))
        cur, cur_max = [], 0
        for i in part:
            ln = len(items[i]["ids"])
            nm = max(cur_max, ln)
            if cur and (nm * (len(cur) + 1) > max_tokens or len(cur) + 1 > max_seqs):
                batches.append(cur)
                cur, nm = [], ln
            cur.append(i)
            cur_max = nm
        if cur:
            batches.append(cur)
    if shuffle:
        rng.shuffle(batches)
    return batches


def collate(items, idx, pad_id):
    sel = [items[i] for i in idx]
    n, L = len(sel), max(len(it["ids"]) for it in sel)
    ids = torch.full((n, L), pad_id, dtype=torch.long)
    att = torch.zeros((n, L), dtype=torch.long)
    for r, it in enumerate(sel):
        ids[r, :len(it["ids"])] = torch.tensor(it["ids"])
        att[r, :len(it["ids"])] = 1
    mpos = torch.tensor([it["markers"] for it in sel], dtype=torch.long)
    mmask = torch.ones(mpos.shape, dtype=torch.bool)
    return {"input_ids": ids, "attention_mask": att, "marker_pos": mpos, "marker_mask": mmask,
            "qtype": torch.zeros(n, dtype=torch.long), "label": torch.tensor([it["label"] for it in sel]),
            "risk": torch.tensor([it.get("risk", -1) for it in sel]),
            "aux_class": torch.tensor([it.get("aux_class", 0) for it in sel])}


class Wrapped(nn.Module):
    """DecisionModel + returns pooled (h[:,0] after the head) for the auxiliary probe."""

    def __init__(self, model, n_class_keys):
        super().__init__()
        self.model = model
        d = model.encoder.config.hidden_size
        self.aux_label = nn.Linear(d, 3)
        self.aux_class = nn.Linear(d, max(1, n_class_keys))
        self._pooled = None
        model.act_head.register_forward_hook(self._grab)

    def _grab(self, mod, inp, out):
        x = inp[0]
        self._pooled = x[:, :-4]

    def forward(self, b):
        logits, act = self.model(b["input_ids"], b["attention_mask"], b["marker_pos"], b["marker_mask"], b["qtype"])
        head = getattr(self.model, "risk_head", None)
        risk = head(self._pooled.float()) if head is not None else None
        return logits, act, self._pooled, risk


def proper_loss(logits, target, w_sph=0.5, weight=None):
    """Laya's strictly proper score: log score + w_sph * spherical score (negated, mean over the batch; weighted per
    item when `weight` is given)."""
    q = torch.softmax(logits.float(), -1)
    logq = torch.log(q.clamp_min(1e-12)).clamp_min(-9.21)
    log_score = (target * logq).sum(-1)
    sph = (target * q).sum(-1) / q.norm(dim=-1).clamp_min(1e-9)
    score = log_score + w_sph * sph
    if weight is None:
        return -score.mean()
    return -(score * weight).sum() / weight.sum().clamp_min(1e-9)


@torch.no_grad()
def predict(model, items, pad_id, device, max_tokens=32768, dtype=torch.bfloat16, risk=False):
    """Approval logits [N, 3]; with `risk`, (approval logits, risk logits [N, 4])."""
    model.eval()
    em = lc.ExportModel(model)
    out_logits = np.zeros((len(items), 3), np.float32)
    out_risk = np.zeros((len(items), len(lc.RISK_LABELS)), np.float32)
    for idx in token_batches(items, max_tokens, 512, None, shuffle=False):
        b = collate(items, idx, pad_id)
        b = {k: v.to(device) for k, v in b.items()}
        with torch.autocast("cuda", dtype=dtype, enabled=device.type == "cuda"):
            out = em(b["input_ids"], b["attention_mask"], b["marker_pos"], b["marker_mask"], b["qtype"])
        out_logits[idx] = out[0].float().cpu().numpy()
        if risk:
            out_risk[idx] = out[3].float().cpu().numpy()
    model.train()
    return (out_logits, out_risk) if risk else out_logits


def risk_views(recs, items, rlog):
    """Item risk logits -> (facts view [R, 4], full view [R, 4]) per record."""
    rf = np.zeros((len(recs), rlog.shape[1]), np.float32)
    ru = np.zeros_like(rf)
    for it, z in zip(items, rlog):
        (rf if it["variant"] == "s_facts" else ru)[it["rec"]] = z
    return rf, ru


def risk_report(recs, items, rlog, T, theta):
    """Risk metrics for the whole split and its val parts."""
    rf, ru = risk_views(recs, items, rlog)
    out = {"all": lc.risk_metrics(recs, rf, ru, T, theta)}
    parts = np.array([r.get("part", "") for r in recs])
    for p in sorted(set(parts) - {""}):
        sel = np.where(parts == p)[0]
        out[p] = lc.risk_metrics([recs[i] for i in sel], rf[sel], ru[sel], T, theta)
    return out


def golden_risk_records(path=None):
    """golden.json["risk"] cases as records (approval labels are placeholders: the golden set is about risk)."""
    path = path or os.path.join(os.path.dirname(os.path.abspath(__file__)), "golden.json")
    with open(path) as f:
        cases = json.load(f).get("risk", {}).get("cases", [])
    return [dict(c, label_facts="ask", label_full="ask") for c in cases]


def split_scores(recs, items, logits, temperature, name="", sel=None):
    """Item accuracy / NLL and the spec's per-record decision (section 3) on `recs` (optionally a subset `sel`)."""
    labels = np.array([it["label"] for it in items])
    p = lc.softmax(logits, temperature)
    pf = np.zeros((len(recs), 3))
    pu = np.zeros((len(recs), 3))
    for it, pr in zip(items, p):
        (pf if it["variant"] == "s_facts" else pu)[it["rec"]] = pr
    rsel = np.ones(len(recs), bool) if sel is None else sel
    isel = rsel[np.array([it["rec"] for it in items])]
    acc = float((p[isel].argmax(-1) == labels[isel]).mean())
    nll = float(-np.log(np.clip(p[isel][np.arange(isel.sum()), labels[isel]], 1e-12, 1)).mean())
    dec = lc.autopilot_decide(pf[rsel], pu[rsel])
    gold = np.array([lc.LABEL_ID[r["label_full"]] for r in recs])[rsel]
    auto = dec != 2
    return {"split": name, "n": int(rsel.sum()), "item_acc": round(acc, 4), "nll": round(nll, 4),
            "coverage": round(float(auto.mean()), 4),
            "auto_err": round(float((dec[auto] != gold[auto]).mean()) if auto.any() else 0.0, 4),
            "unsafe_approve": int(((dec == 0) & (gold != 0)).sum()), "wrong_deny": int(((dec == 1) & (gold == 0)).sum())}


def evaluate_split(model, tok, recs, items, pad_id, device, temperature=1.0, name=""):
    logits = predict(model, items, pad_id, device)
    return split_scores(recs, items, logits, temperature, name), logits


def val_report(recs, items, logits, T):
    """Whole val + its parts (iid / heldout_family / heldout_phrasing)."""
    out = {"all": split_scores(recs, items, logits, T, "val")}
    parts = np.array([r.get("part", "iid") for r in recs])
    for p in sorted(set(parts)):
        out[p] = split_scores(recs, items, logits, T, "val." + p, parts == p)
    return out


def safe_temperature(recs, items, logits, t0, t_max=8.0):
    """Smallest T >= t0 (grid, x1.05 steps) with no unsafe automatic approve on val; t_max if none."""
    t = t0
    while t < t_max:
        if split_scores(recs, items, logits, t)["unsafe_approve"] == 0:
            return t
        t *= 1.05
    return t_max


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="ml", help="en | en-typed | ml | path to a laya checkpoint")
    ap.add_argument("--data", default=os.path.join(lc.CACHE, "data", "v1"))
    ap.add_argument("--out", required=True)
    ap.add_argument("--full", action="store_true", help="full fine-tune instead of LoRA")
    ap.add_argument("--r", type=int, default=16)
    ap.add_argument("--alpha", type=int, default=32)
    ap.add_argument("--lr", type=float, default=2e-4, help="LoRA / encoder lr")
    ap.add_argument("--head-lr", type=float, default=1e-4)
    ap.add_argument("--epochs", type=float, default=2.0)
    ap.add_argument("--max-tokens", type=int, default=12288)
    ap.add_argument("--max-seqs", type=int, default=64)
    ap.add_argument("--max-len", type=int, default=lc.MAX_LEN)
    ap.add_argument("--aux", type=float, default=0.2, help="weight of the pooled-embedding probe losses")
    ap.add_argument("--grad-ckpt", action="store_true")
    ap.add_argument("--device", default="cuda:0")
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--limit", type=int, default=0, help="debug: use only N training records")
    ap.add_argument("--eval-every", type=int, default=300, help="val evaluation + best-state tracking (0 = off)")
    ap.add_argument("--smooth", type=float, default=0.0, help="label smoothing of the target distribution")
    ap.add_argument("--safe-temperature", type=int, default=1, help="raise T until val has no unsafe approve (1/0)")
    ap.add_argument("--risk-data", help="gen_risk.py splits: adds and trains the risk head (docs/autopilot-risk.md)")
    ap.add_argument("--risk-weight", type=float, default=1.0, help="weight of the risk cross-entropy")
    ap.add_argument("--risk-approval-weight", type=float, default=0.3,
                    help="weight of the approval loss on risk records (v7 records weigh 1): they teach the approval head "
                         "the context line without pulling it away from v7's behaviour")
    ap.add_argument("--risk-lr", type=float, default=1e-3, help="lr of the risk head")
    ap.add_argument("--risk-recall", type=float, default=0.995,
                    help="catastrophic recall on risk val the Overnight threshold is set for")
    ap.add_argument("--risk-limit", type=int, default=0, help="debug: use only N risk training records")
    args = ap.parse_args()

    random.seed(args.seed)
    np.random.seed(args.seed)
    torch.manual_seed(args.seed)
    device = torch.device(args.device)
    ckpt = lc.base_dir(args.base)
    tok = lc.load_tokenizer(ckpt)
    model, cfg = lc.load_model(ckpt, device="cpu")
    hml = lc.head_max_len(cfg)
    pad_id = tok.pad_token_id

    splits = {}
    for s in ("train", "val", "test_iid", "test_ood", "adv_test"):
        p = os.path.join(args.data, s + ".jsonl")
        if os.path.exists(p):
            splits[s] = lc.read_jsonl(p)
    train_recs = splits["train"]
    if args.limit:
        random.Random(args.seed).shuffle(train_recs)
        train_recs = train_recs[:args.limit]
    risk_splits = {}
    if args.risk_data:
        for s in ("train", "val", "test_iid", "test_ood", "adv_test"):
            p = os.path.join(args.risk_data, s + ".jsonl")
            if os.path.exists(p):
                risk_splits[s] = lc.read_jsonl(p)
        if args.risk_limit:
            random.Random(args.seed + 1).shuffle(risk_splits["train"])
            risk_splits["train"] = risk_splits["train"][:args.risk_limit]
        train_recs = train_recs + risk_splits["train"]
    class_keys = sorted({r["class_key"] for r in train_recs})
    ck_id = {k: i for i, k in enumerate(class_keys)}
    t0 = time.time()
    train = lc.items_from_records(tok, train_recs, args.max_len, hml)
    for it in train:
        it["aux_class"] = ck_id[train_recs[it["rec"]]["class_key"]]
    evals = {s: (splits[s], lc.items_from_records(tok, splits[s], args.max_len, hml)) for s in splits if s != "train"}
    risk_evals = {s: (risk_splits[s], lc.items_from_records(tok, risk_splits[s], args.max_len, hml)) for s in risk_splits if s != "train"}
    if args.risk_data:
        golden = golden_risk_records()
        risk_evals["golden"] = (golden, lc.items_from_records(tok, golden, args.max_len, hml))
        if getattr(model, "risk_head", None) is None:
            model.risk_head = lc.make_risk_head(model.encoder.config.hidden_size)
    lens = [len(it["ids"]) for it in train]
    print("encoded %d train items in %.0fs; len mean %.0f p95 %.0f max %d; truncated %d" % (
        len(train), time.time() - t0, np.mean(lens), np.percentile(lens, 95), max(lens),
        sum(l >= args.max_len for l in lens)), flush=True)

    # ---- parameters to train
    if args.full:
        for p in model.parameters():
            p.requires_grad_(True)
        enc_params = list(model.encoder.parameters())
    else:
        from peft import LoraConfig, get_peft_model
        for p in model.encoder.parameters():
            p.requires_grad_(False)
        lcfg = LoraConfig(r=args.r, lora_alpha=args.alpha, lora_dropout=0.05,
                          target_modules=["Wqkv", "Wo", "Wi"], bias="none")
        model.encoder = get_peft_model(model.encoder, lcfg)
        model.encoder.print_trainable_parameters()
        enc_params = [p for p in model.encoder.parameters() if p.requires_grad]
    if args.grad_ckpt:
        enc = model.encoder.base_model.model if not args.full else model.encoder
        enc.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    head_params = [p for n, p in model.named_parameters() if not n.startswith("encoder.") and not n.startswith("risk_head.")]
    risk_params = [p for n, p in model.named_parameters() if n.startswith("risk_head.")]
    for p in head_params + risk_params:
        p.requires_grad_(True)
    wrapped = Wrapped(model, len(class_keys)).to(device)
    aux_params = list(wrapped.aux_label.parameters()) + list(wrapped.aux_class.parameters())
    groups = [{"params": enc_params, "lr": args.lr, "weight_decay": 0.01},
              {"params": head_params + aux_params, "lr": args.head_lr, "weight_decay": 0.01}]
    if risk_params:
        groups.append({"params": risk_params, "lr": args.risk_lr, "weight_decay": 0.01})
    opt = torch.optim.AdamW(groups, betas=(0.9, 0.98), eps=1e-6)
    rng = random.Random(args.seed)
    n_epochs_int = math.ceil(args.epochs)
    per_epoch = len(token_batches(train, args.max_tokens, args.max_seqs, random.Random(0)))
    total = int(per_epoch * args.epochs)
    warm = max(10, int(0.05 * total))
    sched = torch.optim.lr_scheduler.LambdaLR(
        opt, lambda s: min(1.0, (s + 1) / warm) * max(0.0, 0.5 * (1 + math.cos(math.pi * min(1.0, s / total)))))
    print("steps: %d (%d per epoch), warmup %d" % (total, per_epoch, warm), flush=True)

    trainable = {n: p for n, p in model.named_parameters() if p.requires_grad}
    best = {"nll": float("inf"), "step": 0, "state": None}

    def track(step):
        vrecs, vitems = evals["val"]
        vlog = predict(model, vitems, pad_id, device)
        vlab = np.array([it["label"] for it in vitems])
        T = lc.fit_temperature(vlog, vlab)
        rep = val_report(vrecs, vitems, vlog, T)
        # selection: mean NLL over the val parts, so held-out families and phrasings weigh as much as iid
        nll = float(np.mean([v["nll"] for k, v in rep.items() if k != "all"]))
        print("  val@%d T=%.3f " % (step, T) + " | ".join("%s acc %.4f nll %.4f cov %.3f unsafe %d wdeny %d" % (
            k, v["item_acc"], v["nll"], v["coverage"], v["unsafe_approve"], v["wrong_deny"]) for k, v in rep.items()), flush=True)
        if risk_evals:
            rrecs, ritems = risk_evals["val"]
            _, rlog = predict(model, ritems, pad_id, device, risk=True)
            Tr = lc.fit_temperature(rlog, np.array([it["risk"] for it in ritems]))
            rf, ru = risk_views(rrecs, ritems, rlog)
            p_cat = lc.risk_combine(lc.softmax(rf, Tr), lc.softmax(ru, Tr))[:, lc.CATASTROPHIC]
            theta = lc.risk_threshold(p_cat, np.array([r["risk_full"] for r in rrecs]) == lc.CATASTROPHIC, args.risk_recall)
            rrep = risk_report(rrecs, ritems, rlog, Tr, theta)
            rnll = float(np.mean([v["nll"] for k, v in rrep.items() if k != "all"]))
            print("  risk@%d T=%.3f theta=%.4f " % (step, Tr, theta) + " | ".join(
                "%s acc %.4f nll %.4f ece %.3f catR %.4f fh-benign %.4f fh-noncat %.4f" % (
                    k, v["acc_combined"], v["nll"], v["ece"], v["cat_recall"], v["false_holds_benign"], v["false_holds_noncat"])
                for k, v in rrep.items()), flush=True)
            nll += args.risk_weight * rnll
        if nll < best["nll"]:
            best.update(nll=nll, step=step, state={n: p.detach().to("cpu", copy=True) for n, p in trainable.items()})

    step, t0, run_loss = 0, time.time(), []
    wrapped.train()
    done = False
    for ep in range(n_epochs_int):
        for idx in token_batches(train, args.max_tokens, args.max_seqs, rng):
            b = {k: v.to(device) for k, v in collate(train, idx, pad_id).items()}
            with torch.autocast("cuda", dtype=torch.bfloat16):
                logits, act, pooled, risk = wrapped(b)
                tgt = F.one_hot(b["label"], 3).float() * (1 - args.smooth) + args.smooth / 3
                has_risk = b["risk"] >= 0
                loss = proper_loss(logits, tgt, weight=torch.where(has_risk, args.risk_approval_weight, 1.0).float())
                if risk is not None and has_risk.any():
                    loss = loss + args.risk_weight * F.cross_entropy(risk.float()[has_risk], b["risk"][has_risk])
                # act head: act (0) when the answer is right, escalate (1) otherwise
                right = (logits.detach().argmax(-1) == b["label"]).long()
                loss = loss + 0.1 * F.cross_entropy(act.float(), 1 - right)
                if args.aux > 0:
                    # cosine geometry, like the phone's kNN; the raw [CLS] norm is large and would blow up CE
                    pf = F.normalize(pooled.float(), dim=-1) * 16.0
                    loss = loss + args.aux * (F.cross_entropy(wrapped.aux_label(pf), b["label"])
                                              + F.cross_entropy(wrapped.aux_class(pf), b["aux_class"]))
            opt.zero_grad(set_to_none=True)
            loss.backward()
            torch.nn.utils.clip_grad_norm_([p for g in opt.param_groups for p in g["params"]], 1.0)
            opt.step()
            sched.step()
            step += 1
            run_loss.append(loss.item())
            if step % 50 == 0:
                el = time.time() - t0
                print("ep %d step %d/%d loss %.4f lr %.2e | %.1f steps/s ETA %.1fm | mem %.1fG" % (
                    ep, step, total, np.mean(run_loss[-50:]), sched.get_last_lr()[0], step / el,
                    (total - step) / max(step / el, 1e-9) / 60, torch.cuda.max_memory_allocated(device) / 2**30),
                    flush=True)
            if args.eval_every and step % args.eval_every == 0 and "val" in evals:
                track(step)
            if step >= total:
                done = True
                break
        if done:
            break

    # ---- best state, merge LoRA, fit temperature, evaluate, save
    if args.eval_every and "val" in evals:
        track(step)
        print("best val NLL %.4f at step %d of %d" % (best["nll"], best["step"], step), flush=True)
        with torch.no_grad():
            for n, p in trainable.items():
                p.copy_(best["state"][n].to(p.device))
    if not args.full:
        model.encoder = model.encoder.merge_and_unload()
    model.eval()
    val_recs, val_items = evals["val"]
    vlog = predict(model, val_items, pad_id, device)
    vlab = np.array([it["label"] for it in val_items])
    T_nll = lc.fit_temperature(vlog, vlab)
    print("temperature %.4f (val NLL %.4f -> %.4f)" % (T_nll, -np.log(lc.softmax(vlog)[np.arange(len(vlab)), vlab] + 1e-12).mean(),
                                                    -np.log(lc.softmax(vlog, T_nll)[np.arange(len(vlab)), vlab] + 1e-12).mean()))
    T = safe_temperature(val_recs, val_items, vlog, T_nll) if args.safe_temperature else T_nll
    if args.safe_temperature and risk_evals:
        # the risk records carry approval labels too: no unsafe automatic approve on their val either
        rv_recs, rv_items = risk_evals["val"]
        rvlog = predict(model, rv_items, pad_id, device)
        T = safe_temperature(rv_recs, rv_items, rvlog, T)
    print("temperature used %.4f (safety on val: %s)" % (T, "raised" if T > T_nll else "unchanged"), flush=True)
    for k, v in val_report(val_recs, val_items, vlog, T).items():
        print("  val.%s" % k, v, flush=True)
    results = {}
    for s, (recs, items) in evals.items():
        r, _ = evaluate_split(model, tok, recs, items, pad_id, device, T, name=s)
        results[s] = r
        print(r, flush=True)
    risk_cfg = None
    if risk_evals:
        rrecs, ritems = risk_evals["val"]
        _, rlog = predict(model, ritems, pad_id, device, risk=True)
        T_r = lc.fit_temperature(rlog, np.array([it["risk"] for it in ritems]))
        rf, ru = risk_views(rrecs, ritems, rlog)
        p_cat = lc.risk_combine(lc.softmax(rf, T_r), lc.softmax(ru, T_r))[:, lc.CATASTROPHIC]
        theta = lc.risk_threshold(p_cat, np.array([r["risk_full"] for r in rrecs]) == lc.CATASTROPHIC, args.risk_recall)
        print("risk temperature %.4f, Overnight threshold P(catastrophic) >= %.4f (val recall target %.3f)" % (
            T_r, theta, args.risk_recall), flush=True)
        risk_results = {}
        for s, (recs, items) in risk_evals.items():
            # approval on the risk records too (their own labels), then the risk metrics
            if s != "golden":
                ra, _ = evaluate_split(model, tok, recs, items, pad_id, device, T, name="risk." + s)
                print(ra, flush=True)
                results["risk." + s] = ra
            _, rl = predict(model, items, pad_id, device, risk=True)
            risk_results[s] = risk_report(recs, items, rl, T_r, theta)
            for k, v in risk_results[s].items():
                print("  risk.%s.%s %s" % (s, k, json.dumps({a: (round(b, 4) if isinstance(b, float) else b) for a, b in v.items()})), flush=True)
        risk_cfg = {"labels": lc.RISK_LABELS, "temperature": T_r, "thresholds": {"overnight": theta},
                    "recall_target": args.risk_recall, "situation": "context", "data": args.risk_data,
                    "approval_weight": args.risk_approval_weight, "results": risk_results}

    os.makedirs(args.out, exist_ok=True)
    from safetensors.torch import save_file
    sd = {k: v.detach().cpu().contiguous() for k, v in model.state_dict().items()}
    sd["temperature"] = torch.tensor([T, 1.0, 1.0])
    save_file(sd, os.path.join(args.out, "model.safetensors"))
    shutil.copytree(os.path.join(ckpt, "tokenizer"), os.path.join(args.out, "tokenizer"), dirs_exist_ok=True)
    shutil.copytree(os.path.join(ckpt, "encoder"), os.path.join(args.out, "encoder"), dirs_exist_ok=True)
    new_cfg = dict(cfg)
    if risk_cfg:
        new_cfg["risk"] = risk_cfg
    new_cfg.update({"model_name": "laya-approvals", "max_len": args.max_len, "temperature": [T, 1.0, 1.0],
                    "temperature_by_options": {"choice:3-5": T}, "fine_tuned": True,
                    "autopilot": {"base": args.base, "base_dir": ckpt, "data": args.data, "full": args.full,
                                  "lora_r": None if args.full else args.r, "epochs": args.epochs, "lr": args.lr,
                                  "head_lr": args.head_lr, "aux": args.aux, "steps": step, "best_step": best["step"],
                                  "smooth": args.smooth, "temperature_nll": T_nll,
                                  "minutes": round((time.time() - t0) / 60, 1), "results": results}})
    with open(os.path.join(args.out, "rl_agent_config.json"), "w") as f:
        json.dump(new_cfg, f, indent=2)
    print("saved", args.out)


if __name__ == "__main__":
    main()
