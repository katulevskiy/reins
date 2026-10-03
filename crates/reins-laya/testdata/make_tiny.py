"""Writes `tiny.onnx`: a few-kilobyte model with Laya's ONNX inputs and outputs (spec section 6.2), for testing the
`ort` runtime without the real package. Its numbers mean nothing; its shapes and names are the real ones.

    ~/.cache/reins-laya/.venv/bin/python crates/reins-laya/testdata/make_tiny.py
"""
import os

import torch
import torch.nn as nn

HIDDEN = 8
VOCAB = 128


class Tiny(nn.Module):
    def __init__(self):
        super().__init__()
        torch.manual_seed(7)
        self.emb = nn.Embedding(VOCAB, HIDDEN)
        self.score = nn.Linear(HIDDEN, 1)
        self.act = nn.Linear(HIDDEN, 2)
        self.qtype = nn.Embedding(3, HIDDEN)

    def forward(self, input_ids, attention_mask, marker_pos, marker_mask, qtype):
        # Padding never matters: every position past the mask is zero.
        h = (self.emb(input_ids.clamp(0, VOCAB - 1)) + self.qtype(qtype)[:, None, :]) * attention_mask.unsqueeze(-1).float()
        idx = marker_pos.clamp(min=0)[:, :, None].expand(-1, -1, HIDDEN)
        logits = self.score(torch.gather(h, 1, idx)).squeeze(-1).masked_fill(~marker_mask, -1e4)
        pooled = h.sum(1) / attention_mask.sum(1, keepdim=True).clamp(min=1).float()
        return logits, self.act(pooled), pooled


def main():
    model = Tiny().eval()
    b, l, k = 2, 16, 3
    args = (torch.randint(0, VOCAB, (b, l)), torch.ones(b, l, dtype=torch.long), torch.tensor([[1, 2, 3]] * b),
            torch.ones(b, k, dtype=torch.bool), torch.zeros(b, dtype=torch.long))
    out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "tiny.onnx")
    torch.onnx.export(
        model, args, out, dynamo=False, opset_version=17,
        input_names=["input_ids", "attention_mask", "marker_pos", "marker_mask", "qtype"],
        output_names=["logits", "act", "pooled"],
        dynamic_axes={"input_ids": {0: "b", 1: "l"}, "attention_mask": {0: "b", 1: "l"},
                      "marker_pos": {0: "b", 1: "k"}, "marker_mask": {0: "b", 1: "k"}, "qtype": {0: "b"},
                      "logits": {0: "b", 1: "k"}, "act": {0: "b"}, "pooled": {0: "b"}})
    print("wrote", out, os.path.getsize(out), "bytes")

    # What ONNX Runtime (Python) returns for one padded batch, to check the Rust runtime against.
    import json

    import numpy as np
    import onnxruntime
    feeds = {
        "input_ids": np.array([[2, 10, 20, 30, 3, 4, 40, 4, 50, 4, 60, 3, 70, 3], [2, 11, 21, 3, 4, 41, 4, 51, 4, 61, 3, 71,
                                                                                3, 0]], dtype=np.int64),
        "attention_mask": np.array([[1] * 14, [1] * 13 + [0]], dtype=np.int64),
        "marker_pos": np.array([[5, 7, 9], [4, 6, 8]], dtype=np.int64),
        "marker_mask": np.ones((2, 3), dtype=bool),
        "qtype": np.zeros(2, dtype=np.int64),
    }
    logits, act, pooled = onnxruntime.InferenceSession(out).run(None, feeds)
    expected = {k: v.astype(np.int64 if v.dtype != bool else np.uint8).reshape(-1).tolist() for k, v in feeds.items()}
    expected.update({"batch": 2, "seq_len": 14, "k": 3, "hidden": HIDDEN, "logits": logits.reshape(-1).tolist(),
                     "act": act.reshape(-1).tolist(), "pooled": pooled.reshape(-1).tolist()})
    path = os.path.join(os.path.dirname(out), "tiny_expected.json")
    with open(path, "w") as f:
        json.dump(expected, f)
        f.write("\n")
    print("wrote", path)


if __name__ == "__main__":
    main()
