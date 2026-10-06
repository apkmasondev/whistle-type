"""Developer helper: downloads the Whisper GGML models used by the benchmark (the app itself downloads models
through its Model Manager). Pinned to an immutable Hugging Face revision of ggerganov/whisper.cpp (MIT) and
verified with SHA-256.

usage: python scripts/fetch-whisper-models.py [--dest third_party/whisper-models] [model ...]
"""
import argparse
import hashlib
import sys
import time
import urllib.request
from pathlib import Path

REV = "5359861c739e955e79d9a303bcbc70fb988958b1"
MODELS = {  # file: sha256 (keep in sync with src/models.rs)
    "ggml-base-q5_1.bin": "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898",
    "ggml-small-q5_1.bin": "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb",
    "ggml-medium-q5_0.bin": "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f",
    "ggml-large-v3-turbo-q5_0.bin": "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
}
ROOT = Path(__file__).resolve().parent.parent


def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


def fetch(name, dest_dir):
    dest = dest_dir / name
    if dest.exists() and sha256(dest) == MODELS[name]:
        print(f"{name}: ok (cached)")
        return
    url = f"https://huggingface.co/ggerganov/whisper.cpp/resolve/{REV}/{name}"
    tmp = dest.with_suffix(".part")
    for attempt in range(10):
        try:
            have = tmp.stat().st_size if tmp.exists() else 0
            req = urllib.request.Request(url, headers={"Range": f"bytes={have}-"} if have else {})
            with urllib.request.urlopen(req, timeout=60) as r, open(tmp, "ab" if have and r.status == 206 else "wb") as f:
                while True:
                    b = r.read(1 << 20)
                    if not b:
                        break
                    f.write(b)
            break
        except Exception as e:  # the Hub drops connections now and then: resume
            print(f"  {name}: retry {attempt + 1} ({e})")
            time.sleep(2 + 2 * attempt)
    got = sha256(tmp)
    if got != MODELS[name]:
        tmp.unlink()
        sys.exit(f"{name}: SHA-256 mismatch ({got})")
    tmp.replace(dest)
    print(f"{name}: ok ({dest.stat().st_size / 1e6:.0f} MB)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dest", default=str(ROOT / "third_party" / "whisper-models"))
    ap.add_argument("models", nargs="*")
    a = ap.parse_args()
    dest = Path(a.dest)
    dest.mkdir(parents=True, exist_ok=True)
    for m in a.models or MODELS:
        fetch(m, dest)


if __name__ == "__main__":
    main()
