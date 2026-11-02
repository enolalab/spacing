#!/usr/bin/env python3
"""Pair emit-bench 'E <ts> <seq>' lines with listen 'R <ts> <type> <code>
<val>' lines and print latency stats (ms). LAN runs need a skew
correction: pass S1,T1,S2,T2 (two NTP-style skew samples bracketing the
emit window) to interpolate per-event clock offset.

Usage: analyze.py <emit.log> <recv.log> key|mouse [S1 T1 S2 T2]
"""
import statistics as st
import sys


def pct(a, p):
    return a[min(len(a) - 1, int(p * len(a)))]


def load_e(path):
    e = {}
    for line in open(path):
        p = line.split()
        if len(p) == 3 and p[0] == "E":
            e[int(p[2])] = int(p[1])
    return e


def load_r(path):
    r = []
    for line in open(path):
        p = line.split()
        if len(p) == 5 and p[0] == "R":
            r.append((int(p[1]), int(p[4])))  # (ts, value)
    return r


def main():
    emit_f, recv_f, kind = sys.argv[1], sys.argv[2], sys.argv[3]
    skew_fn = (lambda t: 0)
    if len(sys.argv) == 8:
        s1, t1, s2, t2 = map(int, sys.argv[4:8])
        skew_fn = lambda t: s1 + (s2 - s1) * (t - t1) / (t2 - t1)

    E, R = load_e(emit_f), load_r(recv_f)
    seqs = sorted(E)
    lats = []
    if kind == "key":
        downs = [r for r in R if r[1] == 1]
        drops = len(seqs) * 2 - len(R)
        for i, s in enumerate(seqs):
            if i < len(downs):
                lats.append(downs[i][0] - E[s] - skew_fn(E[s]))
    else:
        drops = len(seqs) - len(R)
        for i, s in enumerate(seqs):
            if i < len(R):
                lats.append(R[i][0] - E[s] - skew_fn(E[s]))

    lats.sort()
    rate = len(R) / ((R[-1][0] - R[0][0]) / 1e9) if len(R) > 1 else 0
    print(
        f"n={len(lats)} recv={len(R)} drops={drops} rate={rate:.0f}ev/s\n"
        f"min={lats[0]/1e6:.3f} p50={pct(lats,.5)/1e6:.3f} "
        f"p95={pct(lats,.95)/1e6:.3f} p99={pct(lats,.99)/1e6:.3f} "
        f"max={lats[-1]/1e6:.3f} mean={st.mean(lats)/1e6:.3f}"
    )


if __name__ == "__main__":
    main()
