# WhistleType — research notes

Research date: **2026-10-05**. Everything below was checked against the official sources on that date.
Statements marked **(verified locally)** were reproduced on the development machine (Windows 11 26200,
AMD Ryzen 7 5800H, 8C/16T) rather than taken from documentation.

## 1. What Whistle is today

| Item | Finding | Source |
|---|---|---|
| Model | Whistle — Cactus Compute's speech‑to‑text model, one `whistle.cact` file, 16,919,407 bytes (16.9 MB), weights version 2.0.0 | [HF model card](https://huggingface.co/Cactus-Compute/whistle), `config.json` |
| Runtime | It does **not** have a runtime of its own. It runs on the **Needle 3 C++ engine** (`libneedle`), which loads both Needle (`needle3.cact`) and Whistle (`whistle.cact`) | HF card "This repo holds `whistle.cact`; the engine and its platform folders are in Cactus-Compute/needle3" |
| Python package | `cactus-needle` 3.1.0 (`pip install cactus-needle`). It is a thin `ctypes` wrapper that downloads the engine wheel + weights from Hugging Face on first use | [github.com/cactus-compute/needle](https://github.com/cactus-compute/needle), `needle/agent/whistle.py`, `needle/agent/fetch.py` |
| Engine/weights pairing | `cactus-needle` 3.1.0 pins engine **3.1.0** and Whistle weights **2.0.0** (`ENGINE_VERSIONS` in `fetch.py`). Mixing mismatched engine and `.cact` versions caused crashes in the past (issue #58), so WhistleType pins both | `needle/agent/fetch.py`, [issue #58](https://github.com/cactus-compute/needle/issues/58) |
| Engine source | **Closed source.** Only binaries are published (Apache‑2.0). The Python, training and export code are open | [issue #118](https://github.com/cactus-compute/needle/issues/118) |
| Audio input | 16 kHz, mono, float32 PCM in [-1, 1] (or a 16 kHz WAV via the Python wrapper) | `needle.h`, `config.json` (`sample_rate: 16000`) |
| Max length | **30 s per call** (480,000 samples). Longer input returns error `"audio limit is 30 s"` (it does not crash) **(verified locally)** | `config.json` `max_audio_seconds: 30`, `needle.h` |
| Languages | en, de, fr, es, it, nl, **pl**. Auto‑detect when `language == NULL`, forced when a code is given | `needle.h`, model card |
| Keyword biasing | **Yes**, natively: `keywords` = newline‑separated words/phrases, favoured during beam search | `needle.h` (`needle_transcribe`), model card |
| VAD / no‑speech | Built‑in: "silence and steady noise give an empty text and language". **(verified locally)**: 2 s of digital silence, 3 s of Gaussian noise, NaN input and a square wave all returned `""` | `needle.h`, README |
| Confidence | Optional `word_timestamps` → per‑word `probability`. No utterance‑level no‑speech probability | `needle.h` |
| Decoding | 5‑beam search, fixed in the engine | README / model card |
| CPU / GPU | CPU only, "no dependencies and no GPU". No CUDA anywhere | model card |
| Threads | Not configurable through the C API. The engine sizes its own thread pool (`GetLogicalProcessorInformationEx`). **(verified locally)**: 11 worker threads are created at `needle_load` on the 5800H; they sleep on condition variables, idle CPU after a transcription was 0.0 s over 3 s | DLL import table, local probe |

### Windows x64 support

The blog post lists "Windows ARM" among deployment targets, but the official engine repo
[Cactus-Compute/needle3](https://huggingface.co/Cactus-Compute/needle3) **does publish Windows x64 builds**:

* `windows-x86_64/needle.exe`, `libneedle.a`, `needle.h` (CLI runner + static library), and
* `python/cactus_needle-3.1.0-py3-none-win_amd64.whl`, which contains **`needle/libneedle3.dll`** (1,502,720 bytes, PE32+ AMD64).

The DLL was inspected **(verified locally)**:

* Built with LLVM/MinGW (libc++, libunwind statically linked); depends only on `KERNEL32.dll` and the Universal CRT
  (`api-ms-win-crt-*`), which ships with Windows 10/11. No VC++ redistributable is needed.
* Exports exactly the documented C API: `needle_load`, `needle_models`, `needle_last_error`, `needle_init`,
  `needle_complete`, `needle_set_audio`, `needle_reset`, `needle_transcribe`, `needle_embed`.
* **Imports no networking API at all** (no `WS2_32`, `WinHTTP`, `WinINet`, no `LoadLibrary`/`GetProcAddress`,
  no `GetEnvironmentVariable`). It cannot open a socket. This matches the docs: "Inference never touches the network",
  "The engine reads no environment variables".
* Note: `needle.exe` (the CLI runner) *does* import `WS2_32` for its `--serve` HTTP mode — WhistleType does not use it.

### C API used by WhistleType (from the official `needle.h`)

```c
int needle_load(const unsigned char* cact, unsigned long long n);   // copies the blob (verified: buffer can be freed)
const char* needle_last_error(void);
int needle_transcribe(const float* pcm, int samples, const char* language,
                      const char* keywords /* '\n'-separated */, int word_timestamps,
                      char* out, int out_capacity);
// out: {"text":"...","language":"pl","ttft_ms":..,"decode_tps":..[,"words":[...]]}
```

"One process‑global, non‑thread‑safe model per kind" → WhistleType calls the engine from exactly one dedicated thread.
The engine cannot unload weights (llms.txt), so the model is loaded once per process.

### First local measurements (Python ctypes probe, before writing the app)

| Clip (Polish TTS, 16 kHz) | Length | Wall time `needle_transcribe` |
|---|---|---|
| "To jest test rozpoznawania mowy." | 2.7 s | ~100 ms — transcribed exactly |
| "Sprawdź komponent React i popraw useEffect." | 3.4 s | ~110 ms |
| "Zażółć gęślą jaźń…" | 5.7 s | ~600 ms |

`needle_load` took 17 ms and added ≈ 22 MB RSS. Language choice matters: with pure auto‑detection a fast
synthetic Polish clip was detected as French, while forcing `pl` makes English commands come out Polish‑spelled
(observed by the user with real speech). WhistleType therefore defaults to **Auto (Polish + English)**: detect, and
re‑run as Polish only when the detected language is neither Polish nor English. Keyword biasing visibly helps (`reakt` → `react` with
`React` in the keyword list) but is a soft bias, not a guarantee. Full numbers are in `PERFORMANCE.md`.

## 2. Licences

| Component | Licence | Redistribution |
|---|---|---|
| `cactus-compute/needle` (Python, training/export code) | Apache‑2.0 (`LICENSE`, `pyproject.toml`) | — (not used at runtime) |
| Needle 3 engine binaries (`libneedle3.dll`) — HF `Cactus-Compute/needle3` | Apache‑2.0 (model card `license: apache-2.0`, `LICENSE` file identical to Whistle's) | Allowed with the licence text + attribution. WhistleType ships the unmodified DLL with `LICENSES/Apache-2.0.txt` and `THIRD_PARTY_NOTICES.md` |
| Whistle weights (`whistle.cact`) — HF `Cactus-Compute/whistle` | Apache‑2.0 | Redistribution would be allowed, but WhistleType **downloads it on first run** from the official repo (pinned commit + SHA‑256), so the installer stays small and the user sees exactly what is fetched and from where |
| Neither repo has a `NOTICE` file, so there are no extra NOTICE obligations beyond keeping the licence and attribution. |

Pinned artifacts (immutable Hugging Face revisions):

| File | Repo @ commit | SHA‑256 |
|---|---|---|
| `whistle.cact` | `Cactus-Compute/whistle` @ `b358ddadd89b7a713b5aa131f23032d3cca1b251` | `b6e02f048568ac5d01a2042556c658061e699acbc0aa2a1439f52f3d461dffeb` |
| `python/cactus_needle-3.1.0-py3-none-win_amd64.whl` | `Cactus-Compute/needle3` @ `c7c415a3d1b3d929014bc6e866d51ebb971f7089` | `4f5fc86abfc50d551cdb237a34b501f36d82d4b6f5911ee7bec4e9532d44dd95` |

Hugging Face publishes LFS SHA‑256 digests via its API; the values above match those digests and were
re‑computed locally after download.

## 3. Telemetry

* The **Python package** (`needle/_telemetry.py`) sends anonymous usage counts to a Supabase endpoint unless
  `NEEDLE_TELEMETRY=0` or `DO_NOT_TRACK=1` is set; downloads via `huggingface_hub` also ping `config.json`.
* The README says "telemetry is turned on in the binary" — but the model card says the engine reads no environment
  variables, the devices guide says "Inference never touches the network", and the DLL import table proves it has no
  network capability.
* **Decision:** WhistleType does not use the Python package at all. It calls `libneedle3.dll` directly, so none of the
  Python telemetry code is present. The only network traffic WhistleType can ever make is the one‑time, user‑confirmed
  model download (see `PERFORMANCE.md` / `AUDIT.md` for the offline verification).

## 4. Windows platform research

| Problem | Finding | Decision |
|---|---|---|
| Global push‑to‑talk with key‑up | `RegisterHotKey` has no key‑up event. `WH_KEYBOARD_LL` sees down/up and can swallow the key | Low‑level hook on a dedicated thread; swallow the hotkey's down/repeat/up so the focused app never sees F8 |
| Hook removed silently | Windows removes LL hooks whose callback exceeds `LowLevelHooksTimeout` | Hook thread does nothing but post a message; it never blocks. Hook is reinstalled if a fallback hotkey fires |
| Elevated (admin) target windows | UIPI: a medium‑integrity process neither receives LL‑hook events while an elevated window is focused nor can `SendInput` into it | Also `RegisterHotKey` the same combo (works with elevated windows; only fires when the hook did not swallow the key). Release detected by polling `GetAsyncKeyState` while recording. If the foreground is elevated, the text is left on the clipboard and the overlay says "press Ctrl+V". `uiAccess` would need a signed binary in Program Files — not possible for an unsigned open‑source build |
| Alt/Win menus | Swallowing a key while Alt/Win is held makes the bare Alt/Win release open a menu | Inject a masking key (VK 0xE8, unassigned) like AutoHotkey does |
| Text insertion | Options: (a) clipboard + Ctrl+V, (b) `SendInput` with `KEYEVENTF_UNICODE`, (c) UI Automation `ValuePattern.SetValue` | (a) is the default: fastest, atomic, works in Electron/Chromium, terminals, Office. (b) offered as an option (no clipboard use; slower; editors may auto‑close brackets). (c) rejected: replaces the whole field and many apps do not implement it |
| Clipboard safety | Snapshot every restorable format, set our text with `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory=0`, `CanUploadToCloudClipboard=0`, paste, then restore. The text is offered with **delayed rendering**, so WhistleType knows the moment the target app actually read it and restores only after that (with a timeout fallback) | Implemented in `src/clipboard.rs` + `src/insert.rs` |
| Audio capture | WASAPI shared mode; `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | SRC_DEFAULT_QUALITY` lets the Windows audio engine convert any mic to 16 kHz mono float | Direct WASAPI (no extra dependency). The stream is opened only while recording, so the mic privacy indicator is off and the process is idle otherwise. `IMMNotificationClient` for device add/remove/default changes |
| Settings UI | Native Win32 controls + Common Controls v6 + Per‑Monitor‑V2 DPI manifest | No web view, no GPU context, ~0 MB extra RAM |
| Overlay | Layered, click‑through, `WS_EX_NOACTIVATE` tool window, Direct2D + DirectWrite into a DIB, per‑pixel alpha | Never takes focus, DPI‑correct on each monitor, appears on the monitor of the foreground window |
| Autostart | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (per‑user, no admin) | Standard; respected by Task Manager's Startup tab |
| Model download | WinHTTP (system TLS, system proxy, no extra crates), `.part` file, SHA‑256 check, atomic rename | `src/download.rs` |

## 5. Architecture decision

**Rust + raw Win32 (`windows` crate), one native executable, the official `libneedle3.dll` loaded at runtime.**

Why, against the user's priority list:

1. *Compatibility with Whistle* — the only official Windows x64 runtime is a C ABI DLL. Any language can call it;
   Rust calls it with zero overhead and no interpreter. Python would mean shipping an interpreter (and the telemetry
   module) just to call a DLL.
2. *Reliability* — memory safety in our own code; the non‑thread‑safe engine is confined to one thread by
   construction.
3. *RAM / 4. idle CPU* — no GC, no web view, no runtime. The process sleeps in `GetMessage`; timers and the audio
   thread run only while recording or while a window is visible.
5–6. *Speed/latency* — model loaded once at start‑up and warmed up; audio is kept in memory and handed to the engine
   as a float slice (no temporary WAV ever).
7–8. *Hotkey / insertion* — direct access to `SetWindowsHookEx`, `SendInput`, the clipboard API and token integrity
   checks.
9. *Size* — the app is a single small `.exe` + the 1.5 MB engine DLL.
10. *Installation* — Inno Setup per‑user installer (no admin, no UAC prompt), clean uninstall, portable ZIP.

Rejected: **Electron/Tauri** (web view RAM, no benefit), **C#/.NET** (needs the .NET runtime or a large self‑contained
bundle; no SDK on the dev machine), **Python** (interpreter + telemetry module + packaging pain), **C++** (would work
equally well; Rust chosen for memory safety with the same footprint).

## 6. Assumptions in the brief that turned out different

* Whistle is not a separate library — it is a weights file for the Needle 3 engine. "cactus-needle" is the Python
  wrapper; WhistleType uses the engine directly.
* The blog lists "Windows ARM" but x64 is also published (DLL in the official wheel). Verified to run on AMD64.
* The README mentions telemetry "in the binary"; the DLL has no network code. The telemetry lives in the Python
  package, which WhistleType does not use.
* Whistle has a hard 30 s limit per call. WhistleType records up to 5 minutes and splits the audio at the quietest
  points into ≤ 29.5 s segments, transcribed in order and joined with a space.
* CPU thread count cannot be set through the DLL API; the engine's own pool is used (and measured).

## 7. Second engine for higher accuracy (ACCURATE mode, round 4)

Goal: keep Whistle as the fast CPU engine and add a more accurate model for Polish, using the RTX 3060 Laptop
(6 GB VRAM) when it helps, with an automatic CPU fallback — inside the same app, hotkey, overlay and clipboard code.

### Candidates

| Option | What it is | Windows / GPU | Distribution | Verdict |
|---|---|---|---|---|
| **Whistle** (Needle 3) | 16.9 MB model, closed‑source C engine | CPU only (x64 DLL) | Apache‑2.0, already integrated | stays as **FAST** |
| **OpenAI Whisper** (PyTorch) | reference implementation | CUDA via PyTorch (~2–3 GB of wheels) | MIT | rejected: Python + PyTorch runtime in a tray app |
| **faster‑whisper** | Whisper on **CTranslate2** (C++ inference engine), Python API | CUDA 12 + cuDNN 9 needed separately on Windows; the commonly used Windows CUDA DLL bundles are unofficial | MIT; last release 1.2.1 (2025‑10‑31) | rejected: Python runtime, extra NVIDIA libraries the user has to obtain, no official Windows GPU bundle |
| **CTranslate2** directly (C++) | engine behind faster‑whisper | official Windows wheels contain the DLL, but a C++ Whisper front‑end (feature extraction, tokenizer, decoding) would have to be written | MIT | rejected: large amount of new, untested code |
| **whisper.cpp** (ggml) | C/C++ Whisper, stable C API (`whisper.h`) | **official prebuilt Windows x64 binaries** for CPU and for CUDA 11.8 / 12.4, GPU backend loaded dynamically (`GGML_BACKEND_DL`) | MIT; models in GGML format on Hugging Face (MIT weights from OpenAI) | **chosen as ACCURATE** |

Why whisper.cpp fits this app: same integration pattern as Whistle (a C DLL loaded at run time, called directly from
Rust, no interpreter); the CPU build is ~10 MB and ships with the app; the CUDA build is the official release asset and
is self‑contained (cudart + cuBLAS inside), so the user needs only an NVIDIA driver — no CUDA toolkit, no cuDNN.
Quantised models (q5) keep download size and VRAM low.

### Integration details found while building it

* `whisper_full()` takes `whisper_full_params` **by value**. The Rust `#[repr(C)]` mirror was checked against MSVC
  (`sizeof` = 304, field offsets asserted in a unit test) and, at run time, the library's own defaults are compared
  with the expected values before the first call, so a mismatched DLL fails safely instead of corrupting memory.
* ggml loads its backends with plain `LoadLibrary`, so the pack folder is added with `SetDllDirectoryW` and
  `GGML_BACKEND_PATH` is cleared. Only one runtime pack can be loaded per process — switching CPU ↔ CUDA pack needs a
  restart (the app says so).
* The MSVC‑built DLLs need the Visual C++ runtime; the redistributable DLLs are deployed app‑locally next to them.
* The CUDA 11.8 asset is not self‑contained (no `cublas64_11.dll`); the **CUDA 12.4** asset is. It needs a driver
  ≥ 525 (this PC: 616.64, CUDA 13.4 capable). Only the 16 files that are needed are extracted (1.13 GB on disk),
  each verified with its own SHA‑256.
* The Whisper encoder always processes a 30 s window, so a 2 s and a 25 s dictation cost almost the same on the
  encoder. On this laptop: encoder of *small* 158 ms on the GPU vs 3.7 s on the CPU.
* GPU memory: CUDA reserves a large commit charge (~2 GB) but the working set of the app stays ~0.5 GB; VRAM ~1 GB for
  large‑v3‑turbo q5_0 — fits a 6 GB card comfortably.

### Models offered (pinned revision `5359861c…` of `ggerganov/whisper.cpp`, SHA‑256 checked)

| Model | File | Size | Parameters | Recommended on |
|---|---|---|---|---|
| base | ggml-base-q5_1.bin | 59.7 MB | 74 M | CPU |
| small | ggml-small-q5_1.bin | 190 MB | 244 M | CPU or GPU |
| medium | ggml-medium-q5_0.bin | 539 MB | 769 M | GPU |
| large‑v3‑turbo | ggml-large-v3-turbo-q5_0.bin | 574 MB | 809 M | GPU (default for ACCURATE) |

Measured quality, latency, CPU, RAM, GPU and VRAM for each are in [PERFORMANCE.md](PERFORMANCE.md#fast-vs-accurate-benchmark).

### Modes

* **FAST** — Whistle on the CPU (unchanged).
* **ACCURATE** — Whisper; on the NVIDIA GPU when the GPU pack is installed, otherwise on the CPU. If the GPU fails to
  initialise the model is loaded on the CPU; if a GPU call fails at run time the model is reloaded on the CPU and the
  dictation retried once. If no Whisper model is ready, that dictation falls back to FAST and the overlay/notification
  says so.
* **AUTO** (default) — ACCURATE when Whisper is ready **on the GPU**, otherwise FAST. The benchmark shows why: on the
  GPU large‑v3‑turbo is both clearly more accurate and fast enough for dictation; on this CPU every Whisper model that
  beats Whistle on Polish takes seconds per sentence, which is too slow to be a sensible automatic choice.

Licences and distribution: whisper.cpp/ggml MIT (bundled CPU DLLs, licence in `LICENSES/MIT-whisper.cpp.txt`);
Whisper weights MIT (OpenAI), downloaded by the user; NVIDIA cudart/cuBLAS are on NVIDIA's redistributable list and
reach the user inside the official whisper.cpp release asset, downloaded only on request. See THIRD_PARTY_NOTICES.md.
