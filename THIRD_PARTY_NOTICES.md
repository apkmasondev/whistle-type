# Third-party notices

WhistleType itself is Copyright (c) 2026 apkmasondev, MIT licence (`LICENSE`). It bundles or downloads the following third-party components.

## Cactus Compute — Needle 3 engine (`libneedle3.dll`)

- Source of the binary: https://huggingface.co/Cactus-Compute/needle3 (`python/cactus_needle-3.1.0-py3-none-win_amd64.whl`, revision c7c415a3d1b3d929014bc6e866d51ebb971f7089)
- Project: https://github.com/cactus-compute/needle
- Copyright: Cactus Compute, Inc.
- Licence: Apache License 2.0 — full text in `LICENSES/Apache-2.0.txt`
- Shipped unmodified. The binary statically links LLVM libc++/libunwind (Apache-2.0 WITH LLVM-exception) and the mingw-w64 runtime; those notices are the engine vendor's.

## Cactus Compute — Whistle 2.0.0 speech model (`whistle.cact`)

- Not bundled. Downloaded by the user on first run from https://huggingface.co/Cactus-Compute/whistle (revision b358ddadd89b7a713b5aa131f23032d3cca1b251, SHA-256 b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb)
- Copyright: Cactus Compute, Inc.
- Licence: Apache License 2.0 — full text in `LICENSES/Apache-2.0.txt`
- Citation: Mroz, Ndubuaku, Mosoyan, Cylich, Kumar, Sandhu, Shemet, Lee — "Whistle: Speech Recognition for Tiny Devices", Cactus Compute, Inc., 2026.

## whisper.cpp / ggml runtime (`whisper-cpu\*.dll`, ACCURATE mode)

- Official prebuilt binaries of whisper.cpp v1.9.4, release b5130 (commit 927cfce34f31707e17f2bff35c349632fb9e2c3a): https://github.com/ggml-org/whisper.cpp/releases/tag/b5130 (`whisper-bin-x64.zip`, SHA-256 f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c). Shipped unmodified.
- Copyright (c) 2023-2024 The ggml authors
- Licence: MIT — full text in `LICENSES/MIT-whisper.cpp.txt`

## Microsoft Visual C++ runtime (`whisper-cpu\msvcp140.dll`, `vcruntime140*.dll`, `vcomp140.dll`)

- Needed by the whisper.cpp DLLs (built with MSVC). App-local copies of the Visual C++ 2015-2022 redistributable files (14.44), distributed under the Microsoft Visual Studio licence terms for "Distributable Code" (redist list). Copyright (c) Microsoft Corporation.

## OpenAI Whisper models (GGML conversions, ACCURATE mode)

- Not bundled. Downloaded only when the user clicks Download in Speech models, from https://huggingface.co/ggerganov/whisper.cpp (revision 5359861c739e955e79d9a303bcbc70fb988958b1), each file verified with a pinned SHA-256: ggml-base-q5_1.bin, ggml-small-q5_1.bin, ggml-medium-q5_0.bin, ggml-large-v3-turbo-q5_0.bin.
- Model weights: Copyright (c) 2022 OpenAI, MIT licence — full text in `LICENSES/MIT-openai-whisper.txt`. Quantised GGML conversions by the whisper.cpp project (MIT).

## Optional GPU pack (NVIDIA CUDA 12.4 build of whisper.cpp)

- Not bundled. Downloaded only when the user clicks Download for the GPU pack, from the official whisper.cpp release asset https://github.com/ggml-org/whisper.cpp/releases/download/b5130/whisper-cublas-12.4.0-bin-x64.zip (SHA-256 af520ddd034d985b55dfeea3e465ed93653ba2aee1a55e865033edc548c272a7; every extracted DLL is verified with its own pinned SHA-256).
- Contains whisper.cpp/ggml (MIT, see above) and the NVIDIA CUDA runtime libraries `cudart64_12.dll`, `cublas64_12.dll` and `cublasLt64_12.dll`, which NVIDIA lists as redistributable in the CUDA Toolkit EULA (https://docs.nvidia.com/cuda/eula/). Copyright (c) NVIDIA Corporation.

## Rust crates compiled into WhistleType.exe

| Crate | Version | Licence | Repository |
|---|---|---|---|
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cpufeatures | 0.2.17 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| equivalent | 1.0.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| generic-array | 0.14.7 | MIT | https://github.com/fizyk20/generic-array.git |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| itoa | 1.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| syn | 3.0.6 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| typenum | 1.20.1 | MIT OR Apache-2.0 | https://github.com/paholg/typenum |
| unicode-ident | 1.0.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| windows | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-collections | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-future | 0.3.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-numerics | 0.3.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-threading | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| zip | 4.6.1 | MIT | https://github.com/zip-rs/zip2.git |
| zlib-rs | 0.6.8 | Zlib | https://github.com/trifectatechfoundation/zlib-rs |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |

The MIT, Apache-2.0 and Zlib (zlib-rs) licence texts are in `LICENSES/`. Each crate is used under one of its offered licences (MIT where dual-licensed MIT OR Apache-2.0).
