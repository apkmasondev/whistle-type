# WhistleType — performance

Measured on **2026-10-05**, AMD Ryzen 7 5800H (8 cores / 16 threads), Windows 11 26200, mains power,
with a normal desktop session running (browser, IDE, Claude app). Numbers come from the scripts in `scripts/`
(`perf.ps1`, `eval.py`, `wt-bench`), raw output in `tests/results/` (not committed).

All audio for latency tests is real-time playback of WAV files through the `test-hooks` build (identical code path
except the microphone); start-up, idle and memory numbers are from the production build.

## Start-up and idle (production build)

| Metric | Value |
|---|---|
| `WhistleType.exe` size | 1.2 MB (+ 1.5 MB `libneedle3.dll`, + 12 MB `whisper-cpu\` for ACCURATE incl. VC++ runtime) |
| Process start → UI ready (`started in`) | **31–37 ms** |
| Model load incl. SHA-256 verification (16.9 MB) | **29–31 ms** |
| Process start → model ready for dictation | **185–210 ms** (FAST) |
| Engine warm-up | < 1 ms |
| **CPU while idle**, 60 s after start | **0.000 s CPU (0 %)** |
| **CPU while idle**, 30 s after 9 dictations | **0.000 s CPU (0 %)** |
| RAM after start (working set / private) | 37 MB / 27 MB |
| RAM after long dictations (working set / private) | 85 MB / 79 MB (engine keeps its buffers) |
| Threads idle | 16–17 (main, keyboard hook, inserter, engine + 11 engine worker threads, COM) |
| Threads while dictating | 23–24 (+ audio capture, WASAPI / audio engine threads) |
| **CPU while recording** (real microphone, overlay animating), 4 × 10 s | 2.2–4.1 % of one core ≈ 0.2 % of the whole CPU (overlay: software Direct2D at 25 fps, text formats cached) |

Idle means: no timers, no polling, no audio stream. The microphone is opened only while the hotkey is held
(the Windows mic privacy indicator is off otherwise). The overlay timer runs only while the overlay is visible.

## Latency: hotkey release → text in Notepad

The "Whistle" column is measured from the moment the recording ends to the finished transcript. The recording
ends **120 ms after you release the key** (a short tail so the last syllable is not cut), so the time from key
release to visible text is: 120 ms + Whistle time + a few ms for the paste (column "Paste read"). The
"clipboard restored" column additionally includes the 200 ms grace period and the clipboard restore, which happen
after the text is already visible.

| Clip | Audio | Whistle | Paste read after keystroke | Recording end → clipboard restored | CPU time used | Peak working set |
|---|---|---|---|---|---|---|
| short phrase | 1.3 s | **46 ms** | 0 ms | 292 ms | 0.6 s | 56 MB |
| sentence (×3) | 3.4 s | **91–93 ms** | 0–3 ms | 314–318 ms | 1.1 s | 59 MB |
| PL + EN terms | 4.0 s | **96 ms** | 2 ms | 315 ms | 1.4 s | 84 MB |
| paragraph | 7.7 s | **595 ms** | 3 ms | 826 ms | 6.8 s | 63 MB |
| long sentence | 17.9 s | **1560 ms** | 3 ms | 1793 ms | 17.7 s | 86 MB |
| 20 s | 19.6 s | **1567 ms** | 3 ms | 1799 ms | 17.6 s | 74 MB |
| near the limit | 26.3 s | **2200 ms** | 4 ms | 2428 ms | 24.9 s | 84 MB |
| over the limit (2 segments) | 38.2 s | **3254 ms** | — | — | — | — |

Observations:

- Up to ~4 s of speech Whistle answers in < 100 ms; between ~5 s and ~8 s the engine's time to first token jumps
  (~30 ms → ~330 ms, an engine characteristic visible in its own `ttft_ms`), after that it grows roughly linearly at
  ~85 ms per second of audio (≈ 0.08× real time).
- The engine parallelises over its 11 worker threads: CPU time ≈ 0.9 core-seconds per second of audio, i.e. a
  20 s dictation briefly uses ~11 logical CPUs for 1.6 s. It cannot be configured through the C API (see RESEARCH.md);
  the burst is short and the system stays responsive (normal priority).
- Microphone stream start (WASAPI open → first buffer): ~80 ms in a debug build, measured with `wt-bench --record`.
  Speech normally starts later than that after pressing the key.

## Memory leak check

`wt-bench --repeat 100` on a 20 s clip and on a 4 s clip (200 transcriptions in one process):
private memory 57.0 → 58.9 MB after the first long clip, then **flat at 58.9 MB for the next 99**, and
57.2 MB flat for the 100 short ones. No growth.

## Accuracy on the synthetic test set (`python scripts/eval.py --release`)

95 cases generated offline with the Polish Windows voices (Paulina, Adam, desktop Paulina) at three speaking rates,
plus noise/length variants. WER/CER are case- and punctuation-insensitive.

| Group | Cases | Transcribed | WER | CER |
|---|---|---|---|---|
| Plain Polish, normal speed ("To jest test rozpoznawania mowy.", the 30-word "story" sentence) | 6 | 6 | **0 %** | 0 % — word-perfect with all three voices |
| Plain Polish, slow/fast speed (same two sentences) | 8 | 8 | 0–80 % | fast Paulina is the outlier; story sentence 0–15 % |
| All sentences, normal speed | 30 | 30 | 45–55 % | 20–21 % |
| Slow speech | 20 | 20 | 42–47 % | 16–18 % |
| Fast speech (rate 1.6×) | 20 | 20 | 61–70 % | 30–37 % |
| Quiet speech (−26 dB, low gain) | 3 | 3 | 30 % | 8 % |
| Speech in pink noise, 15 dB / 5 dB SNR | 6 | 6 | 51 % | 21 % |
| Lengths 1–90 s | 7 | 7 | 35 % | 12 % |
| **Non-speech** (silence, room tone, white/pink noise, mains hum, click, keyboard typing, breathing, 200 ms tap) | 9 | **0 typed** | — | — |

Honest reading of these numbers:

- Most errors are **English technical terms spoken by Polish TTS voices**, which pronounce them the Polish way
  ("Claude Code" → "klotkod", "useEffect" → "useryfekt"). A human saying "React", "Vite" or "WebGL" with English
  pronunciation is a different input; this set is a worst case for mixed-language terms.
- Keyword biasing (custom vocabulary) measurably helps: WER 54.9 % with vs 59.9 % without it for the Adam voice, `reakt` →
  `react`, `wit` → `Vite`, but it does not fix words the speaker mispronounces.
- Recognition language on this synthetic set (WER, all 86 speech cases): forced `pl` 50.1 %, `auto` 51.3 %,
  **`auto_pl_en` (default) 50.9 %**; identical (48.6 %) at normal speaking speed. Pure `auto` heard 4 fast/noisy
  Polish clips as Spanish ("Todo esto está destruido."); `auto_pl_en` re-transcribes those as Polish. The test voices
  pronounce English terms the Polish way, so this set cannot show the benefit of auto-detection for English commands
  that real users observed — that is why `auto_pl_en` is the default. A re-transcription happens only when the
  detected language is neither Polish nor English (one extra engine pass for that clip).
- Without the speech gate, Whistle returned "Dziękuję bardzo." for keyboard typing and breathing noise (with ~0.95
  word probability). With the gate, nothing is typed for any of the 9 non-speech cases while all 86 speech cases
  still pass (margin: speech ≥ 0.37 s voiced, noise ≤ 0.03 s, threshold 0.12 s).

## Insertion

| Method | Measured |
|---|---|
| Clipboard + Ctrl+V | target app read the text 0–8 ms after the keystroke in Notepad, Windows Terminal/PowerShell, Edge, VS Code, WinForms |
| Clipboard restore | 200 ms after the target's read; verified byte-equal sentinel restored in every e2e test |
| Type characters | One key press per character + 12 ms pause (243 characters ≈ 3.5 s). Complete text in Notepad, PowerShell, Edge, VS Code and WinForms (round 4: Notepad 11.2607 dropped letters at the former 6 ms pace). Use the default paste mode for long dictations. |

## FAST vs ACCURATE benchmark

`python scripts/bench-models.py --cuda-runtime <CUDA 12.4 pack>` runs the **same recordings** through the app's exact
pipeline (`wt-bench`: speech gate → engine → text cleanup, language *Auto (Polish + English)*, default vocabulary)
for each engine/model/device, one process per configuration. Raw results: `tests/results/bench-models.json`;
table: `python scripts/bench-table.py`. Measured 2026-10-05 on the Ryzen 7 5800H + **RTX 3060 Laptop (6 GB)**,
driver 616.64, whisper.cpp b5130 (CUDA 12.4 build), Whisper models quantised q5.

Data:
* **FLEURS pl_pl** (Google, CC‑BY‑4.0, dev split, revision `70bb2e84…`): 60 real read‑speech Polish utterances,
  4–16 s, 517 s total, chosen by `scripts/fetch-fleurs.py` (stratified by length). Hard material: news‑style
  sentences with names and numbers.
* **Technical**: the 27 synthetic Polish sentences with English technical terms from `tests/audio/generated`
  (normal speed, 3 voices) — the app's main use case.

Columns: WER/CER case‑ and punctuation‑insensitive; *latency* = engine time per utterance after the recording ends
(add ~120 ms recording tail + ~0.23 s paste/clipboard restore for "text visible"); *RTF* = latency / audio length;
*CPU s / audio s* = process CPU time per second of audio; *RAM working set* / *commit* = peak of the benchmark process
(CUDA reserves a large commit charge it never touches); *VRAM* = rise of the card's used memory while loaded;
*load* = model load incl. warm‑up. CPU rows for small and large‑v3‑turbo use a subset (20 FLEURS / 6 technical) because
they are slow, so compare them on WER only roughly.

**FLEURS pl_pl (real read speech)**

| Configuration | n | WER | CER | latency mean | p90 | RTF | CPU s / audio s | RAM working set | commit | VRAM | GPU util | load |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| FAST · Whistle 2.0.0 · CPU | 60 | 35.5 % | 11.0 % | 730 ms | 1120 ms | 0.082 | 0.98 | 57 MB | 60 MB | — | — | 42 ms |
| Whisper base q5_1 · CPU | 60 | 38.1 % | 11.2 % | 2059 ms | 2435 ms | 0.272 | 1.88 | 352 MB | 840 MB | — | — | 1037 ms |
| Whisper small q5_1 · CPU | 20 | 17.6 % | 4.8 % | 6907 ms | 7319 ms | 0.924 | 6.44 | 562 MB | 1050 MB | — | — | 3802 ms |
| Whisper large-v3-turbo q5_0 · CPU | 20 | 5.0 % | 1.1 % | 30390 ms | 32019 ms | 4.095 | 28.27 | 1054 MB | 1545 MB | — | — | 16403 ms |
| Whisper base q5_1 · GPU | 60 | 33.4 % | 9.3 % | 355 ms | 480 ms | 0.043 | 0.06 | 532 MB | 1341 MB | 388 MB | 33 % | 483 ms |
| Whisper small q5_1 · GPU | 60 | 15.5 % | 4.7 % | 493 ms | 665 ms | 0.059 | 0.08 | 534 MB | 1675 MB | 651 MB | 39 % | 646 ms |
| Whisper medium q5_0 · GPU | 60 | 8.4 % | 2.9 % | 877 ms | 1140 ms | 0.107 | 0.12 | 534 MB | 2369 MB | 1303 MB | 55 % | 1041 ms |
| Whisper large-v3-turbo q5_0 · GPU | 60 | 5.7 % | 2.1 % | 522 ms | 614 ms | 0.066 | 0.08 | 533 MB | 2003 MB | 979 MB | 64 % | 1112 ms |

**Polish + English technical terms (synthetic)**

| Configuration | n | WER | CER | latency mean | p90 | RTF | CPU s / audio s | RAM working set | commit | VRAM | GPU util | load |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| FAST · Whistle 2.0.0 · CPU | 27 | 58.0 % | 23.6 % | 326 ms | 605 ms | 0.058 | 0.76 | 49 MB | 54 MB | — | — | 30 ms |
| Whisper base q5_1 · CPU | 27 | 33.3 % | 11.8 % | 1871 ms | 2033 ms | 0.392 | 2.92 | 309 MB | 796 MB | — | — | 972 ms |
| Whisper small q5_1 · CPU | 6 | 41.7 % | 14.0 % | 6777 ms | 6922 ms | 1.683 | 12.78 | 559 MB | 1047 MB | — | — | 3780 ms |
| Whisper large-v3-turbo q5_0 · CPU | 6 | 25.0 % | 8.2 % | 27279 ms | 27733 ms | 6.813 | 51.70 | 1052 MB | 1542 MB | — | — | 13921 ms |
| Whisper base q5_1 · GPU | 27 | 33.3 % | 11.3 % | 223 ms | 255 ms | 0.043 | 0.07 | 527 MB | 1336 MB | 387 MB | 32 % | 468 ms |
| Whisper small q5_1 · GPU | 27 | 23.5 % | 5.9 % | 293 ms | 444 ms | 0.059 | 0.08 | 528 MB | 1672 MB | 651 MB | 41 % | 619 ms |
| Whisper medium q5_0 · GPU | 27 | 18.5 % | 6.4 % | 564 ms | 701 ms | 0.114 | 0.13 | 527 MB | 2370 MB | 1303 MB | 59 % | 1050 ms |
| Whisper large-v3-turbo q5_0 · GPU | 27 | 16.5 % | 4.2 % | 402 ms | 437 ms | 0.083 | 0.10 | 527 MB | 2003 MB | 979 MB | 72 % | 1125 ms |

**In the app** (production build, `scripts/perf-accurate.ps1`, `scripts/e2e/accurate.ps1`):

| | FAST (Whistle) | ACCURATE large‑v3‑turbo, GPU | ACCURATE small, CPU |
|---|---|---|---|
| Process start → engine ready | 0.21 s | 2.6 s (Whistle ready first) | 3.3 s |
| RAM working set / private, idle with model | 38 / 27 MB | 510 / 1929 MB | 532 / 1019 MB |
| VRAM | — | 972 MB | — |
| **CPU while idle** (60 s) | **0 s** | **0 s** | **0 s** |
| Release → text in Notepad, 3.7–4.8 s sentence | 0.31–0.35 s | 0.62–0.88 s | 6.4 s |
| Release → text, 17.5 s paragraph | 1.8 s | 1.2 s | — |

What the numbers say:

* **Quality:** on real Polish speech large‑v3‑turbo makes ~6× fewer word errors than Whistle (WER 5.7 % vs 35.5 %,
  CER 2.1 % vs 11.0 %); on the technical sentences 16.5 % vs 58 %. medium is close on FLEURS but worse on technical
  terms and slower than turbo (turbo has a 4‑layer decoder). small is the best CPU‑friendly Whisper model. base is
  not better than Whistle on real speech.
* **GPU latency:** turbo needs ~0.4–0.6 s per sentence regardless of length (the encoder always processes a 30 s
  window), so it is slower than Whistle for short phrases (~0.1 s) but **faster for long dictations** (17.5 s:
  1.0 s vs 1.6 s) and uses ~0.1 CPU‑seconds instead of ~1 per second of audio.
* **CPU only:** every Whisper model that beats Whistle takes seconds per sentence on this 8‑core CPU (small ≈ 7 s,
  turbo ≈ 30 s). That is why **AUTO** uses Whisper only when it runs on the GPU, and the Model Manager labels
  medium/turbo "GPU (slow on CPU)".
* **Idle:** neither engine uses any CPU between dictations. ACCURATE keeps the model in RAM/VRAM while it is the
  selected mode; FAST mode frees it (Whisper is not loaded at all).

