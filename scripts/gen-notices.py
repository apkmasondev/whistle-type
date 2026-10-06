"""Generates THIRD_PARTY_NOTICES.md from `cargo metadata` (normal, non-dev dependencies of the shipped
executable for x86_64-pc-windows-msvc) plus the Cactus Compute engine and model.

usage: python scripts/gen-notices.py
"""
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", "x86_64-pc-windows-msvc"],
        cwd=ROOT))
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    # walk normal (runtime) dependencies only - build/dev deps are not shipped
    seen, stack = set(), [root]
    while stack:
        nid = stack.pop()
        for d in nodes[nid]["deps"]:
            if any(k["kind"] is None for k in d["dep_kinds"]) and d["pkg"] not in seen:
                seen.add(d["pkg"])
                stack.append(d["pkg"])
    crates = sorted((pkgs[i] for i in seen), key=lambda p: p["name"])

    out = []
    out.append("# Third-party notices\n")
    out.append("WhistleType itself is Copyright (c) 2026 apkmasondev, MIT licence (`LICENSE`). It bundles or downloads the "
               "following third-party components.\n")
    out.append("## Cactus Compute — Needle 3 engine (`libneedle3.dll`)\n")
    out.append("- Source of the binary: https://huggingface.co/Cactus-Compute/needle3 "
               "(`python/cactus_needle-3.1.0-py3-none-win_amd64.whl`, revision c7c415a3d1b3d929014bc6e866d51ebb971f7089)")
    out.append("- Project: https://github.com/cactus-compute/needle")
    out.append("- Copyright: Cactus Compute, Inc.")
    out.append("- Licence: Apache License 2.0 — full text in `LICENSES/Apache-2.0.txt`")
    out.append("- Shipped unmodified. The binary statically links LLVM libc++/libunwind (Apache-2.0 WITH LLVM-exception) "
               "and the mingw-w64 runtime; those notices are the engine vendor's.\n")
    out.append("## Cactus Compute — Whistle 2.0.0 speech model (`whistle.cact`)\n")
    out.append("- Not bundled. Downloaded by the user on first run from "
               "https://huggingface.co/Cactus-Compute/whistle (revision b358ddadd89b7a713b5aa131f23032d3cca1b251, "
               "SHA-256 b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb)")
    out.append("- Copyright: Cactus Compute, Inc.")
    out.append("- Licence: Apache License 2.0 — full text in `LICENSES/Apache-2.0.txt`")
    out.append('- Citation: Mroz, Ndubuaku, Mosoyan, Cylich, Kumar, Sandhu, Shemet, Lee — "Whistle: Speech Recognition '
               'for Tiny Devices", Cactus Compute, Inc., 2026.\n')
    out.append("## whisper.cpp / ggml runtime (`whisper-cpu\\*.dll`, ACCURATE mode)\n")
    out.append("- Official prebuilt binaries of whisper.cpp v1.9.4, release b5130 (commit 927cfce34f31707e17f2bff35c349632fb9e2c3a): "
               "https://github.com/ggml-org/whisper.cpp/releases/tag/b5130 (`whisper-bin-x64.zip`, SHA-256 "
               "f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c). Shipped unmodified.")
    out.append("- Copyright (c) 2023-2024 The ggml authors")
    out.append("- Licence: MIT — full text in `LICENSES/MIT-whisper.cpp.txt`\n")
    out.append("## Microsoft Visual C++ runtime (`whisper-cpu\\msvcp140.dll`, `vcruntime140*.dll`, `vcomp140.dll`)\n")
    out.append("- Needed by the whisper.cpp DLLs (built with MSVC). App-local copies of the Visual C++ 2015-2022 "
               "redistributable files (14.44), distributed under the Microsoft Visual Studio licence terms for "
               "\"Distributable Code\" (redist list). Copyright (c) Microsoft Corporation.\n")
    out.append("## OpenAI Whisper models (GGML conversions, ACCURATE mode)\n")
    out.append("- Not bundled. Downloaded only when the user clicks Download in Speech models, from "
               "https://huggingface.co/ggerganov/whisper.cpp (revision 5359861c739e955e79d9a303bcbc70fb988958b1), "
               "each file verified with a pinned SHA-256: ggml-base-q5_1.bin, ggml-small-q5_1.bin, ggml-medium-q5_0.bin, "
               "ggml-large-v3-turbo-q5_0.bin.")
    out.append("- Model weights: Copyright (c) 2022 OpenAI, MIT licence — full text in `LICENSES/MIT-openai-whisper.txt`. "
               "Quantised GGML conversions by the whisper.cpp project (MIT).\n")
    out.append("## Optional GPU pack (NVIDIA CUDA 12.4 build of whisper.cpp)\n")
    out.append("- Not bundled. Downloaded only when the user clicks Download for the GPU pack, from the official whisper.cpp "
               "release asset https://github.com/ggml-org/whisper.cpp/releases/download/b5130/whisper-cublas-12.4.0-bin-x64.zip "
               "(SHA-256 af520ddd034d985b55dfeea3e465ed93653ba2aee1a55e865033edc548c272a7; every extracted DLL is verified "
               "with its own pinned SHA-256).")
    out.append("- Contains whisper.cpp/ggml (MIT, see above) and the NVIDIA CUDA runtime libraries `cudart64_12.dll`, "
               "`cublas64_12.dll` and `cublasLt64_12.dll`, which NVIDIA lists as redistributable in the CUDA Toolkit EULA "
               "(https://docs.nvidia.com/cuda/eula/). Copyright (c) NVIDIA Corporation.\n")
    out.append("## Rust crates compiled into WhistleType.exe\n")
    out.append("| Crate | Version | Licence | Repository |")
    out.append("|---|---|---|---|")
    for p in crates:
        out.append(f"| {p['name']} | {p['version']} | {p.get('license') or 'see repository'} | {p.get('repository') or ''} |")
    out.append("\nThe MIT, Apache-2.0 and Zlib (zlib-rs) licence texts are in `LICENSES/`. Each crate is used under one of its "
               "offered licences (MIT where dual-licensed MIT OR Apache-2.0).\n")
    (ROOT / "THIRD_PARTY_NOTICES.md").write_text("\n".join(out), encoding="utf-8")
    print(f"{len(crates)} crates")
    for p in crates:
        print(f"  {p['name']} {p['version']}: {p.get('license')}")


if __name__ == "__main__":
    main()
