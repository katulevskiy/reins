# SPDX-License-Identifier: Apache-2.0
"""Shared helpers for the Autopilot Laya tooling: paths, checkpoint loading, dataset encoding, metrics."""
import json
import math
import os
from typing import Dict, List, Optional, Tuple

import numpy as np
import torch
import torch.nn as nn

import sequence as sq

CACHE = os.path.expanduser(os.environ.get("REINS_LAYA_CACHE", "~/.cache/reins-laya"))
LAYA = os.path.join(CACHE, "laya")
BASES = {  # name -> checkpoint dir (laya layout: rl_agent_config.json, encoder/, tokenizer/, model.safetensors)
    "en": os.path.join(LAYA),
    "en-typed": os.path.join(LAYA, "typed-decisions"),
    "ml": os.path.join(LAYA, "multilingual"),
}
LABELS = sq.OPTIONS  # approve, deny, ask
LABEL_ID = {k: i for i, k in enumerate(LABELS)}
MAX_LEN = 512  # phone sequence length
# Severity classes of the risk head (docs/autopilot-risk.md section 2), in output order.
RISK_LABELS = ["benign", "risky", "severe", "catastrophic"]
CATASTROPHIC = 3
RISK_PREFIX = "risk_head."


def make_risk_head(hidden: int) -> nn.Module:
    """The risk head: a LayerNorm and one linear layer on `pooled` (the [CLS] state after the decision head)."""
    return nn.Sequential(nn.LayerNorm(hidden), nn.Linear(hidden, len(RISK_LABELS)))


def base_dir(name_or_dir: str) -> str:
    return BASES.get(name_or_dir, name_or_dir)


def load_cfg(ckpt: str) -> Dict:
    with open(os.path.join(ckpt, "rl_agent_config.json")) as f:
        return json.load(f)


def load_tokenizer(ckpt: str):
    from transformers import AutoTokenizer
    return AutoTokenizer.from_pretrained(os.path.join(ckpt, "tokenizer"))


def load_model(ckpt: str, device="cpu", attn: str = "sdpa"):
    """Laya DecisionModel with the checkpoint's weights (laya.common.build_model + strict load)."""
    import torch
    from safetensors.torch import load_file
    from transformers import AutoConfig, AutoModel
    from laya.common import DecisionModel, _apply_rope_config, _no_init_weights
    cfg = load_cfg(ckpt)
    ecfg = AutoConfig.from_pretrained(os.path.join(ckpt, "encoder"))
    _apply_rope_config(ecfg)
    ecfg.reference_compile = False
    with _no_init_weights():
        enc = AutoModel.from_config(ecfg, attn_implementation=attn)
    model = DecisionModel(enc, cfg.get("head_layers", 2), len(cfg.get("act_costs", {})) + 1, no_init=True)
    sd = load_file(os.path.join(ckpt, "model.safetensors"))
    # The risk head (when the checkpoint has one) is ours, not Laya's: attached next to the decision model.
    risk = {k[len(RISK_PREFIX):]: sd.pop(k) for k in list(sd) if k.startswith(RISK_PREFIX)}
    model.load_state_dict(sd, strict=True)
    if risk:
        model.risk_head = make_risk_head(enc.config.hidden_size)
        model.risk_head.load_state_dict(risk, strict=True)
    try:
        model.encoder.config.reference_compile = False
    except Exception:
        pass
    return model.to(device), cfg


def head_max_len(cfg: Dict) -> int:
    return int(cfg.get("head_max_len", 192))


def read_jsonl(path: str) -> List[Dict]:
    with open(path) as f:
        return [json.loads(l) for l in f if l.strip()]


def encode_texts(tok, texts: List[str], max_len: int, hml: int) -> List[Tuple[List[int], List[int]]]:
    """Batch-tokenize states, then assemble sequences exactly like sequence.build_sequence."""
    sp = sq.special_ids(tok)
    prefix_ids, markers = sq.build_sequence(sq.hf_encoder(tok), sp, "", max_len, hml)
    prefix = prefix_ids[:-1]  # drop the closing [SEP] of the empty state
    room = max(0, max_len - len(prefix) - 1)
    clean = [t.replace(sp.mask_token, " ") for t in texts]
    enc = tok(clean, add_special_tokens=False)["input_ids"]
    out = []
    for e in enc:
        ids = (prefix + e[:room] + [sp.sep_id])[:max_len]
        out.append((ids, markers))
    return out


def items_from_records(tok, recs: List[Dict], max_len: int, hml: int, variants=("s_facts", "s_full")) -> List[Dict]:
    """Each record -> one item per variant, label from `label_facts` / `label_full`, risk from `risk_facts` /
    `risk_full` (-1 for records without a risk label, such as v7's)."""
    texts, meta = [], []
    for ri, r in enumerate(recs):
        for v in variants:
            texts.append(r[v])
            lab = r["label_facts"] if v == "s_facts" else r["label_full"]
            risk = r.get("risk_facts" if v == "s_facts" else "risk_full", -1)
            meta.append((ri, v, LABEL_ID[lab], -1 if risk is None else int(risk)))
    seqs = encode_texts(tok, texts, max_len, hml)
    items = []
    for (ids, markers), (ri, v, y, risk) in zip(seqs, meta):
        items.append({"ids": ids, "markers": markers, "qtype": sq.QTYPE_CHOICE, "label": y, "risk": risk,
                      "target": [1.0 if i == y else 0.0 for i in range(3)], "rec": ri, "variant": v})
    return items


def softmax(z: np.ndarray, t: float = 1.0) -> np.ndarray:
    z = z / t
    z = z - z.max(-1, keepdims=True)
    e = np.exp(z)
    return e / e.sum(-1, keepdims=True)


def fit_temperature(logits: np.ndarray, labels: np.ndarray) -> float:
    """Scalar temperature minimizing NLL (golden-section on log T in [0.05, 20])."""
    def nll(lt):
        p = softmax(logits, math.exp(lt))
        return -np.log(np.clip(p[np.arange(len(labels)), labels], 1e-12, 1)).mean()
    a, b = math.log(0.05), math.log(20.0)
    g = (math.sqrt(5) - 1) / 2
    c, d = b - g * (b - a), a + g * (b - a)
    for _ in range(80):
        if nll(c) < nll(d):
            b = d
        else:
            a = c
        c, d = b - g * (b - a), a + g * (b - a)
    return float(math.exp((a + b) / 2))


def ece(probs: np.ndarray, labels: np.ndarray, bins: int = 15) -> float:
    conf = probs.max(-1)
    correct = (probs.argmax(-1) == labels).astype(float)
    edges = np.linspace(0, 1, bins + 1)
    e = 0.0
    for i, (lo, hi) in enumerate(zip(edges[:-1], edges[1:])):
        sel = (conf >= lo if i == 0 else conf > lo) & (conf <= hi)
        if sel.any():
            e += sel.mean() * abs(conf[sel].mean() - correct[sel].mean())
    return float(e)


def autopilot_decide(p_facts: np.ndarray, p_full: np.ndarray, th_a: float = 0.95, th_d: float = 0.90) -> np.ndarray:
    """Spec section 3: approve iff min(p_facts, p_full)[approve] >= th_a, deny iff max(...)[deny] >= th_d,
    else ask. Approve is checked first only when deny does not also fire (deny wins a tie of both)."""
    pa = np.minimum(p_facts[:, 0], p_full[:, 0])
    pd = np.maximum(p_facts[:, 1], p_full[:, 1])
    out = np.full(len(pa), 2)
    out[pa >= th_a] = 0
    out[pd >= th_d] = 1
    return out


# ----------------------------------------------------------------------------- risk (docs/autopilot-risk.md)
def risk_combine(p_facts: np.ndarray, p_full: np.ndarray) -> np.ndarray:
    """The prompt-injection rule for risk: per class k, P(class >= k) is the larger of the two views. Returns the
    combined distribution (rows sum to 1), so the AI's text can only move probability toward the severe end."""
    cf = np.cumsum(p_facts[:, ::-1], -1)[:, ::-1]  # cf[:, k] = P(class >= k)
    cu = np.cumsum(p_full[:, ::-1], -1)[:, ::-1]
    c = np.maximum(cf, cu)
    c[:, 0] = 1.0
    out = c - np.concatenate([c[:, 1:], np.zeros((len(c), 1))], -1)
    return np.clip(out, 0.0, 1.0)


def risk_threshold(p_cat: np.ndarray, is_cat: np.ndarray, recall: float) -> float:
    """The largest threshold on P(catastrophic) that holds at least `recall` of the catastrophic records."""
    pos = np.sort(p_cat[is_cat])
    if len(pos) == 0:
        return 0.5
    misses = int(np.floor((1.0 - recall) * len(pos) + 1e-9))
    return float(pos[min(misses, len(pos) - 1)])


def risk_metrics(recs: List[Dict], rf: np.ndarray, ru: np.ndarray, temperature: float, theta: float) -> Dict:
    """Severity metrics on records with risk labels: per view (accuracy, NLL, ECE) and for the combined two-pass
    distribution (per-class precision / recall, catastrophic recall at `theta`, false holds)."""
    yf = np.array([r["risk_facts"] for r in recs])
    yu = np.array([r["risk_full"] for r in recs])
    pf, pu = softmax(rf, temperature), softmax(ru, temperature)
    p = risk_combine(pf, pu)
    pred = p.argmax(-1)
    held = p[:, CATASTROPHIC] >= theta
    items_p = np.concatenate([pf, pu])
    items_y = np.concatenate([yf, yu])
    m = {"n": len(recs), "acc_facts": float((pf.argmax(-1) == yf).mean()), "acc_full": float((pu.argmax(-1) == yu).mean()),
         "acc_combined": float((pred == yu).mean()),
         "nll": float(-np.log(np.clip(items_p[np.arange(len(items_y)), items_y], 1e-12, 1)).mean()),
         "ece": ece(items_p, items_y), "theta": theta, "held": float(held.mean())}
    cat = yu == CATASTROPHIC
    m["cat_recall"] = float(held[cat].mean()) if cat.any() else float("nan")
    m["cat_missed"] = int((~held & cat).sum())
    m["false_holds_benign"] = float(held[yu == 0].mean()) if (yu == 0).any() else float("nan")
    m["false_holds_noncat"] = float(held[~cat].mean()) if (~cat).any() else float("nan")
    m["false_holds_noncat_n"] = int((held & ~cat).sum())
    for i, name in enumerate(RISK_LABELS):
        tp = int(((pred == i) & (yu == i)).sum())
        m["P_" + name] = tp / max(1, int((pred == i).sum()))
        m["R_" + name] = tp / max(1, int((yu == i).sum()))
    return m


class ExportModel(nn.Module):
    """DecisionModel.forward, returning pooled too (no detach, no checkpointing), and the risk logits when the model
    has a risk head."""

    def __init__(self, m):
        super().__init__()
        self.m = m

    def forward(self, input_ids, attention_mask, marker_pos, marker_mask, qtype):
        m = self.m
        h = m.encoder(input_ids=input_ids, attention_mask=attention_mask).last_hidden_state
        h = h + m.type_emb(qtype)[:, None, :]
        pad = attention_mask == 0
        for layer in m.head.layers:
            h = layer(h, src_key_padding_mask=pad)
        idx = marker_pos.clamp(min=0)[:, :, None].expand(-1, -1, h.size(-1))
        g = torch.gather(h, 1, idx)
        logits = m.scorer(g).squeeze(-1).float()
        logits = logits.masked_fill(~marker_mask, -1e4)
        p = torch.softmax(logits, -1)
        k = marker_mask.sum(-1).clamp(min=2).float()
        ent = -(p * torch.log(p.clamp_min(1e-9))).sum(-1) / torch.log(k)
        top2 = p.topk(2, -1).values
        feats = torch.stack([top2[:, 0], top2[:, 0] - top2[:, 1], ent, k / 255.0], -1)
        pooled = h[:, 0].float()
        act = m.act_head(torch.cat([pooled, feats], -1))
        if getattr(m, "risk_head", None) is not None:
            return logits, act, pooled, m.risk_head(pooled).float()
        return logits, act, pooled
