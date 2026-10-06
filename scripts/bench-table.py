"""Prints the markdown tables of tests/results/bench-models.json (written by scripts/bench-models.py)."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
NAMES = {
    "whistle": "FAST · Whistle 2.0.0 · CPU",
    "base-cpu": "Whisper base q5_1 · CPU",
    "small-cpu": "Whisper small q5_1 · CPU",
    "turbo-cpu": "Whisper large-v3-turbo q5_0 · CPU",
    "base-gpu": "Whisper base q5_1 · GPU",
    "small-gpu": "Whisper small q5_1 · GPU",
    "medium-gpu": "Whisper medium q5_0 · GPU",
    "turbo-gpu": "Whisper large-v3-turbo q5_0 · GPU",
}


def main():
    r = json.loads((ROOT / "tests/results/bench-models.json").read_text(encoding="utf-8"))
    for ds, title in (("fleurs", "FLEURS pl_pl (real read speech)"), ("tech", "Polish + English technical terms (synthetic)")):
        print(f"\n**{title}**\n")
        print("| Configuration | n | WER | CER | latency mean | p90 | RTF | CPU s / audio s | RAM working set | commit | VRAM | GPU util | load |")
        print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
        for k, v in r.items():
            s = v.get(ds)
            if not s:
                continue
            vram = f"{s['vram_mb_max']:.0f} MB" if s.get("vram_mb_max") else "—"
            gpu = f"{s['gpu_util_mean']:.0f} %" if s.get("gpu_util_mean") and "gpu" in k else "—"
            print(f"| {NAMES.get(k, k)} | {s['n']} | {s['wer']:.1f} % | {s['cer']:.1f} % | {s['ms_mean']:.0f} ms | {s['ms_p90']:.0f} ms | "
                  f"{s['rtf']:.3f} | {s['cpu_s_per_audio_s']:.2f} | {s['ws_mb_max']:.0f} MB | {s['commit_mb_max']:.0f} MB | {vram} | {gpu} | "
                  f"{s['load_ms']:.0f} ms |")


if __name__ == "__main__":
    main()
