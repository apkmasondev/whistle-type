"""Benchmark of the FAST (Whistle) and ACCURATE (Whisper) engines on the same recordings.

Runs wt-bench (the app's exact pipeline) per configuration and measures quality (WER/CER), latency, CPU time,
RAM (working set and commit), GPU utilisation and VRAM (sampled with nvidia-smi every 100 ms).

Data: tests/audio/fleurs (real Polish read speech, scripts/fetch-fleurs.py) and the synthetic Polish + English-terms
sentences from tests/audio/generated (scripts/gen-test-audio.ps1).

usage: python scripts/bench-models.py [--cuda-runtime DIR] [--configs whistle,turbo-gpu,...] [--cpu-subset 20]
"""
import argparse
import json
import re
import statistics
import subprocess
import threading
import time
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "target" / "release" / "wt-bench.exe"

CONFIGS = {  # name: (wt-bench args, uses gpu, heavy on CPU)
    "whistle": (["--engine", "whistle"], False, False),
    "base-cpu": (["--engine", "whisper", "--model", "whisper-base", "--cpu"], False, False),
    "small-cpu": (["--engine", "whisper", "--model", "whisper-small", "--cpu"], False, True),
    "turbo-cpu": (["--engine", "whisper", "--model", "whisper-large-v3-turbo", "--cpu"], False, True),
    "base-gpu": (["--engine", "whisper", "--model", "whisper-base"], True, False),
    "small-gpu": (["--engine", "whisper", "--model", "whisper-small"], True, False),
    "medium-gpu": (["--engine", "whisper", "--model", "whisper-medium"], True, False),
    "turbo-gpu": (["--engine", "whisper", "--model", "whisper-large-v3-turbo"], True, False),
}


def norm(s):
    s = unicodedata.normalize("NFC", s.lower())
    s = re.sub(r"[^\w\s]", " ", s)
    return s.split()


def ed(a, b):
    prev = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        cur = [i]
        for j, y in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (x != y)))
        prev = cur
    return prev[-1]


def datasets(cpu_subset):
    fl = json.loads((ROOT / "tests/audio/fleurs/manifest.json").read_text(encoding="utf-8"))
    gen = json.loads((ROOT / "tests/audio/generated/manifest.json").read_text(encoding="utf-8"))
    fleurs = [(ROOT / "tests/audio/fleurs" / f"{k}.wav", v["ref"]) for k, v in sorted(fl.items())]
    tech = [(ROOT / "tests/audio/generated/cases" / f"{k}.wav", v["ref"]) for k, v in sorted(gen.items())
            if v["kind"] == "sentence" and "_normal" in k and "story" not in k]
    return {"fleurs": fleurs, "tech": tech, "fleurs_cpu": fleurs[:cpu_subset], "tech_cpu": tech[:max(6, cpu_subset // 3)]}


class GpuSampler:
    """Samples GPU utilisation and memory. Per-process VRAM is "[N/A]" under WDDM on Windows, so VRAM is the rise of
    the card's used memory over the value measured just before the benchmark process starts."""

    def __init__(self):
        self.stop = False
        self.util, self.used = [], []
        self.baseline = self.query()[1] if self.query() else 0.0

    @staticmethod
    def query():
        try:
            u = subprocess.run(["nvidia-smi", "--query-gpu=utilization.gpu,memory.used", "--format=csv,noheader,nounits"],
                               capture_output=True, text=True, timeout=5).stdout.strip().split(",")
            return float(u[0]), float(u[1])
        except Exception:
            return None

    @property
    def vram(self):
        return [max(0.0, u - self.baseline) for u in self.used]

    def run(self):
        while not self.stop:
            q = self.query()
            if q:
                self.util.append(q[0])
                self.used.append(q[1])
            time.sleep(0.1)


def run_config(name, args, cuda_runtime, files, gpu):
    cmd = [str(BENCH), *args, "--lang", "auto_pl_en"]
    if gpu and cuda_runtime:
        cmd += ["--runtime", cuda_runtime]
    cmd += [str(f) for f, _ in files]
    sampler = GpuSampler()
    th = threading.Thread(target=sampler.run, daemon=True)
    th.start()
    t0 = time.time()
    proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8")
    out, err = proc.communicate()
    sampler.stop = True
    th.join()
    rows = [json.loads(l) for l in out.splitlines() if l.startswith("{")]
    if not rows:
        raise SystemExit(f"{name}: no output\n{err[-2000:]}")
    return rows[0], rows[1:], sampler, time.time() - t0


def score(rows, files):
    refs = {f.name: r for f, r in files}
    w_err = w_n = c_err = c_n = 0
    ms, rtf = [], []
    for r in rows:
        ref = norm(refs[r["file"]])
        hyp = norm(r["text"])
        w_err += ed(ref, hyp)
        w_n += len(ref)
        rc, hc = " ".join(ref), " ".join(hyp)
        c_err += ed(rc, hc)
        c_n += len(rc)
        ms.append(r["engine_ms"])
        rtf.append(r["engine_ms"] / 1000 / r["audio_s"])
    ms.sort()
    return {
        "n": len(rows),
        "wer": 100 * w_err / max(1, w_n),
        "cer": 100 * c_err / max(1, c_n),
        "ms_mean": statistics.mean(ms),
        "ms_median": statistics.median(ms),
        "ms_p90": ms[int(0.9 * (len(ms) - 1))],
        "rtf": statistics.mean(rtf),
        "cpu_s_per_audio_s": sum(r["cpu_s"] for r in rows) / sum(r["audio_s"] for r in rows),
        "ws_mb_max": max(r["working_set_mb"] for r in rows),
        "commit_mb_max": max(r["private_mb"] for r in rows),
        "errors": sum(1 for r in rows if r.get("error")),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cuda-runtime", default="")
    ap.add_argument("--configs", default=",".join(CONFIGS))
    ap.add_argument("--cpu-subset", type=int, default=20)
    ap.add_argument("--merge", action="store_true", help="update the existing results file instead of replacing it")
    a = ap.parse_args()
    data = datasets(a.cpu_subset)
    out = ROOT / "tests" / "results" / "bench-models.json"
    results = json.loads(out.read_text(encoding="utf-8")) if a.merge and out.exists() else {}
    for name in a.configs.split(","):
        args, gpu, heavy = CONFIGS[name]
        res = {}
        for ds in ("fleurs", "tech"):
            files = data[ds + "_cpu"] if heavy else data[ds]
            loaded, rows, sampler, wall = run_config(name, args, a.cuda_runtime, files, gpu)
            sc = score(rows, files)
            sc["load_ms"] = loaded["load_ms"]
            sc["device"] = rows[0]["device"] if rows else ""
            sc["gpu_util_mean"] = statistics.mean(sampler.util) if sampler.util else None
            sc["vram_mb_max"] = max(sampler.vram) if sampler.vram else 0
            sc["wall_s"] = wall
            res[ds] = sc
            res[ds + "_samples"] = [{"file": r["file"], "text": r["text"], "ms": r["engine_ms"]} for r in rows[:5]]
            print(f"{name:11} {ds:6} n={sc['n']:2} WER {sc['wer']:5.1f}% CER {sc['cer']:5.1f}% | {sc['ms_mean']:6.0f} ms mean, "
                  f"p90 {sc['ms_p90']:6.0f} | RTF {sc['rtf']:.3f} | CPU {sc['cpu_s_per_audio_s']:.2f} s/s | "
                  f"RAM {sc['ws_mb_max']:.0f} MB (commit {sc['commit_mb_max']:.0f}) | VRAM {sc['vram_mb_max']:.0f} MB | "
                  f"GPU {sc['gpu_util_mean'] or 0:.0f}% | load {sc['load_ms']:.0f} ms | {sc['device']}", flush=True)
        results[name] = res
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(results, ensure_ascii=False, indent=1), encoding="utf-8")
    print("written", out)


if __name__ == "__main__":
    main()
