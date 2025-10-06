#!/usr/bin/env python3
"""
Generate a minimal AIMD model file (ai.mod) with int8 weights.

Header layout (LE, 16 bytes):
  [0x00..0x03] magic b"AIMD"
  [0x04..0x05] n_layers (u16)
  [0x06..0x07] hidden   (u16)
  [0x08..0x0B] vocab    (u32)
  [0x0C]      dtype    (u8) 0=int8
  [0x0D..0x0F] reserved (3x u8) = 0

Weights (contiguous, row-major per layer):
  For each layer l in [0..n_layers-1]:
    in_dim  = hidden
    out_dim = hidden for l < n_layers-1, else (vocab if >0 else hidden)
    W: int8[out_dim][in_dim]
"""
import argparse, os, struct, random

def gen_ai_mod(layers:int, hidden:int, vocab:int, dtype:str, out_path:str, seed:int|None):
    if dtype.lower() not in ("int8",):
        raise SystemExit("Only int8 supported in this generator")
    if layers < 1 or hidden < 1:
        raise SystemExit("layers and hidden must be >= 1")
    if seed is not None:
        random.seed(seed)

    with open(out_path, "wb") as f:
        # header
        f.write(b"AIMD")
        f.write(struct.pack("<H", layers))
        f.write(struct.pack("<H", hidden))
        f.write(struct.pack("<I", vocab))
        f.write(struct.pack("<B", 0))  # dtype=0 (int8)
        f.write(b"\x00\x00\x00")      # reserved

        # weights
        for l in range(layers):
            in_dim = hidden
            out_dim = (hidden if l+1 < layers else (vocab if vocab>0 else hidden))
            for _ in range(out_dim * in_dim):
                # small weights keep activations bounded; range [-8..7]
                f.write(struct.pack("b", random.randint(-8, 7)))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--layers", type=int, default=1)
    ap.add_argument("--hidden", type=int, default=8)
    ap.add_argument("--vocab", type=int, default=0)
    ap.add_argument("--dtype", type=str, default="int8")
    ap.add_argument("--out", type=str, default="ai.mod")
    ap.add_argument("--seed", type=int, default=None)
    args = ap.parse_args()
    os.makedirs(os.path.dirname(args.out) or ".", exist_ok=True)
    gen_ai_mod(args.layers, args.hidden, args.vocab, args.dtype, args.out, args.seed)
    print(f"Wrote {args.out}")

if __name__ == "__main__":
    main()

