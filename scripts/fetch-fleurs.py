"""Downloads a sample of real Polish read speech for benchmarking: Google FLEURS, pl_pl dev split
(CC-BY-4.0, https://huggingface.co/datasets/google/fleurs), pinned to a dataset revision.

Writes tests/audio/fleurs/<id>.wav (16 kHz mono) and tests/audio/fleurs/manifest.json with the reference
transcriptions. Nothing of this is committed (.gitignore). usage: python scripts/fetch-fleurs.py [--count 60]
"""
import argparse
import csv
import io
import json
import random
import tarfile
import time
import urllib.request
from pathlib import Path

REV = "70bb2e84b976b7e960aa89f1c648e09c59f894dd"
BASE = f"https://huggingface.co/datasets/google/fleurs/resolve/{REV}/data/pl_pl"
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tests" / "audio" / "fleurs"
CACHE = ROOT / "tests" / "audio" / "cache"


def fetch(url, dest):
    if dest.exists():
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    for attempt in range(8):
        try:
            with urllib.request.urlopen(url, timeout=60) as r, open(tmp, "wb") as f:
                while True:
                    b = r.read(1 << 20)
                    if not b:
                        break
                    f.write(b)
            tmp.rename(dest)
            return dest
        except Exception as e:  # flaky connections to the Hub
            print(f"  retry {attempt + 1}: {e}")
            time.sleep(2 + 2 * attempt)
    raise SystemExit(f"download failed: {url}")


def wav_seconds(data, name):
    """Duration of a RIFF/WAVE blob (FLEURS uses 16 kHz mono 32-bit float, which `wave` cannot read)."""
    import struct
    pos, fmt = 12, None
    while pos + 8 <= len(data):
        cid, size = data[pos:pos + 4], struct.unpack("<I", data[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            tag, ch, rate, _, align, bits = struct.unpack("<HHIIHH", data[pos + 8:pos + 24])
            fmt = (ch, rate, align)
        elif cid == b"data":
            ch, rate, align = fmt
            assert rate == 16000 and ch == 1, name
            return size / align / rate
        pos += 8 + size + (size & 1)
    raise ValueError(f"no data chunk in {name}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--count", type=int, default=60)
    a = ap.parse_args()
    tsv = fetch(f"{BASE}/dev.tsv", CACHE / "fleurs-pl-dev.tsv")
    tgz = fetch(f"{BASE}/audio/dev.tar.gz", CACHE / "fleurs-pl-dev.tar.gz")
    rows = {}
    with open(tsv, encoding="utf-8") as f:
        for r in csv.reader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
            # id, file name, raw transcription, normalized transcription, phonemes, num samples, gender
            rows[r[1]] = {"id": r[0], "raw": r[2], "norm": r[3], "samples": int(r[5]), "gender": r[6]}
    # deterministic, length-stratified sample (short / medium / long utterances)
    names = sorted(rows, key=lambda n: rows[n]["samples"])
    random.seed(7)
    thirds = [names[i * len(names) // 3:(i + 1) * len(names) // 3] for i in range(3)]
    pick = set()
    for t in thirds:
        pick.update(random.sample(t, min(len(t), a.count // 3)))
    OUT.mkdir(parents=True, exist_ok=True)
    manifest = {}
    with tarfile.open(tgz, "r:gz") as tar:
        for m in tar:
            name = Path(m.name).name
            if name not in pick:
                continue
            data = tar.extractfile(m).read()
            secs = wav_seconds(data, name)
            stem = f"fleurs_{rows[name]['id']}_{name[:-4]}"
            (OUT / f"{stem}.wav").write_bytes(data)
            manifest[stem] = {"ref": rows[name]["raw"], "kind": "fleurs", "seconds": round(secs, 2), "gender": rows[name]["gender"]}
    (OUT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
    secs = sum(v["seconds"] for v in manifest.values())
    print(f"{len(manifest)} utterances, {secs:.0f} s of audio -> {OUT}")


if __name__ == "__main__":
    main()
