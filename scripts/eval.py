"""Runs the generated test set through wt-bench (the app's exact pipeline) and scores it.

usage: python scripts/eval.py [--release] [--no-vocab] [--lang pl] [--only PREFIX]
Writes tests/results/eval-<tag>.json and prints a summary (WER/CER per group, non-speech rejections, timings).
"""
import argparse
import json
import re
import subprocess
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CASES = ROOT / "tests" / "audio" / "generated" / "cases"
MANIFEST = ROOT / "tests" / "audio" / "generated" / "manifest.json"


def norm(s):
    s = unicodedata.normalize("NFC", s.lower())
    s = re.sub(r"[^\w\s.]", " ", s)  # keep dots inside words like three.js
    s = re.sub(r"\.(\s|$)", r" ", s)
    return s.split()


def edit_distance(a, b):
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        cur = [i]
        for j, y in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x != y)))
        prev = cur
    return prev[-1]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--no-vocab", action="store_true")
    ap.add_argument("--lang", default="pl")
    ap.add_argument("--only", default="")
    ap.add_argument("--tag", default="")
    a = ap.parse_args()
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    names = [n for n in manifest if n.startswith(a.only)]
    exe = ROOT / "target" / ("release" if a.release else "debug") / "wt-bench.exe"
    cmd = [str(exe), "--lang", a.lang] + (["--no-vocab"] if a.no_vocab else []) + [str(CASES / f"{n}.wav") for n in names]
    out = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        print(out.stderr)
        sys.exit(out.returncode)
    rows = [json.loads(l) for l in out.stdout.splitlines() if l.strip()]
    loaded = rows[0]
    results = {r["file"][:-4]: r for r in rows[1:]}

    groups = {}
    report = []
    for n in names:
        r = results[n]
        m = manifest[n]
        ref = m["ref"]
        hyp = r["text"]
        entry = {"case": n, "kind": m["kind"], "audio_s": round(r["audio_s"], 2), "total_ms": round(r["total_ms"]),
                 "segments": r["segments"], "verdict": r["verdict"], "text": hyp, "ref": ref, "error": r["error"]}
        if m["kind"] == "nonspeech":
            entry["ok"] = hyp == ""
        elif ref is not None:
            rw, hw = norm(ref), norm(hyp)
            entry["wer"] = edit_distance(rw, hw) / max(1, len(rw))
            rc, hc = " ".join(rw), " ".join(hw)
            entry["cer"] = edit_distance(rc, hc) / max(1, len(rc))
            entry["ok"] = r["error"] is None and hyp != ""
        else:
            entry["ok"] = r["error"] is None
        g = m["kind"] if m["kind"] != "sentence" else "sentence:" + n.split("__")[2]
        groups.setdefault(g, []).append(entry)
        report.append(entry)

    print(f"engine + model load {loaded['load_ms']:.0f} ms")
    print(f"{'group':28} {'n':>3} {'ok':>4} {'WER':>6} {'CER':>6} {'avg ms':>7} {'RTF':>6}")
    for g, es in sorted(groups.items()):
        wers = [e["wer"] for e in es if "wer" in e]
        cers = [e["cer"] for e in es if "cer" in e]
        ok = sum(e["ok"] for e in es)
        avg_ms = sum(e["total_ms"] for e in es) / len(es)
        rtf = sum(e["total_ms"] / 1000 for e in es) / max(1e-9, sum(e["audio_s"] for e in es))
        w = f"{100 * sum(wers) / len(wers):5.1f}%" if wers else "     -"
        c = f"{100 * sum(cers) / len(cers):5.1f}%" if cers else "     -"
        print(f"{g:28} {len(es):3} {ok:4} {w:>6} {c:>6} {avg_ms:7.0f} {rtf:6.3f}")
    bad = [e for e in report if not e["ok"]]
    for e in bad:
        print("FAIL", e["case"], repr(e["text"]), e["error"])
    res_dir = ROOT / "tests" / "results"
    res_dir.mkdir(parents=True, exist_ok=True)
    tag = a.tag or ("novocab" if a.no_vocab else "vocab") + f"-{a.lang}"
    (res_dir / f"eval-{tag}.json").write_text(json.dumps({"load": loaded, "cases": report}, ensure_ascii=False, indent=1), encoding="utf-8")


if __name__ == "__main__":
    main()
