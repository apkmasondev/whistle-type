# WhistleType — audit

Audit date: 2026-10-05. A fresh review of all code after the first working version and the first test pass.
Results: unit tests (35), end-to-end suites (Notepad, PowerShell terminal, Edge, VS Code, WinForms, 45 s dictation,
toggle mode, rapid tapping, Esc, silence/noise), first-run download suite, settings UI suite, performance, leak test.

## Problems found and fixed during testing

| # | Area | Problem | Fix |
|---|---|---|---|
| T1 | Clipboard | Delayed-render `SetClipboardData(fmt, NULL)` returns NULL on success; treated as an error → nothing pasted | Treat code 0 as success; restore the snapshot on real failure |
| T2 | Clipboard / privacy | Clipboard monitors (Windows clipboard history) read the text before Ctrl+V → paste detection fell back to a fixed 500 ms | Reads before the paste keystroke are declined (text stays unrendered, never enters clipboard history); the target app now reads 0–8 ms after Ctrl+V |
| T3 | Speech gate | Whistle produced "Dziękuję bardzo." / "Thank you." (word probability ~0.95) for keyboard typing and breathing | Pitch-periodicity (voicing) check: 9/9 non-speech cases rejected, 86/86 speech cases kept |
| T4 | Settings | `settings.json` saved with a UTF-8 BOM (Notepad, Windows PowerShell) was treated as corrupt | BOM accepted; unit test added |
| T5 | Typing mode | VK_PACKET characters merge/drop when the target app lags ("wwwwą") | One key press per character; layout-mapped keys for plain/Shift characters; VK_PACKET only as a fallback |
| T6 | Typing mode | Sending AltGr letters as Ctrl+Alt+key triggered app shortcuts (Notepad opened a tab and a sign-in dialog) | Never synthesise Ctrl/Alt: such characters go through VK_PACKET |
| T7 | Model | A corrupted model was detected but the repair window did not open | Setup/repair window opens (tray notification when autostarted) |
| T8 | Overlay | Message pill too narrow ("No speech detectec"); z-order re-asserted every frame | Width includes the icon; topmost set once when shown; text formats cached; 25 fps |
| T9 | Test harness | The toggle-mode suite restarted the app in toggle mode and left it there, so the WinForms suite that followed held F8 in toggle mode and never finished | Toggle suite runs last; WinForms check waits up to 3 s for the text |

## Audit findings (code review) and fixes

| # | Severity | Finding | Status |
|---|---|---|---|
| B1 | low | Overlay "Off" still showed info messages | Fixed: only warnings are shown when the overlay is off |
| B2 | medium | After the max-length limit the overlay was forced to "Transcribing…" even if the recording was silence → stuck overlay | Fixed |
| B3 | medium | On failures (no mic, no model) the "key held" flag was cleared → key auto-repeat retried the failing action ~30×/s (log spam, repeated windows) | Fixed: the hook keeps the key held until the real key-up |
| B4 | **high (privacy)** | If the target app read the clipboard later than the 1.5 s timeout, the restored *previous* clipboard (e.g. a password) would have been pasted instead of the dictation | Fixed: timeout 3 s and, when no paste is detected, the dictated text stays on the clipboard instead of restoring old content |
| B5 | low | Alt/Win mask key was sent from inside the low-level hook callback (re-entrancy) | Fixed: queued to the hook thread |
| B6 | medium (security) | `WHISTLETYPE_SKIP_DLL_HASH` / `WHISTLETYPE_ENGINE_DLL` overrides were active in production | Fixed: DLL hash always verified; override only in debug/test builds |
| B7 | medium (usability) | All injected keystrokes were ignored → F8 sent by mouse/macro-key software (G Hub, Synapse, AutoHotkey) did nothing | Fixed: only WhistleType's own keystrokes (tagged `dwExtraInfo`) are ignored. Verified: the e2e harness now drives the production hook without any test flag |
| B8 | low | Recording CPU: overlay rebuilt DirectWrite text formats every frame at 30 fps | Formats cached, 25 fps; recording ≈ 2–4 % of one core |
| B9 | info | Managed UI Automation (.NET) reports the controls as "Pane" | Checked with MSAA (what screen readers use): correct button/checkbox/combobox roles and label names. No change needed |

## Checklist

- **Architecture**: single UI thread owns all state; workers (hook, audio, engine, inserter, downloader) only post
  messages. The non-thread-safe engine is confined to one thread. Modal loops (tray menu, dialogs) run outside the
  state borrow. ✔
- **Hotkey**: LL hook swallows down/repeat/up; capture mode; Esc cancel; RegisterHotKey fallback for admin windows;
  reinstall after resume. ✔ Not testable here: elevated target windows (needs UAC). Known limit: if focus moves to an
  admin window *while* holding the key, its key-up is invisible; pressing the key again ends the recording.
- **Audio**: opened on demand, closed on stop; device list/default changes via IMMNotificationClient; real mic test
  passed (SM900). Unplugging during recording was code-reviewed, not physically tested.
- **Transcription / limits**: ≤ 29.5 s segments, engine never receives > 30 s; 45 s dictation verified.
- **Clipboard**: all HGLOBAL formats + enhanced metafiles saved (64 MB cap), restored only if we still own the
  clipboard; byte-exact restore verified in every e2e test.
- **Race conditions**: sequential job ids; engine and inserter queues preserve order; re-entrant messages re-posted;
  15 rapid taps → no text, no crash.
- **Resources / leaks**: 200 transcriptions → flat memory; handles stable; idle CPU 0.000 s over 60 s.
- **DPI / multi-monitor**: Per-Monitor-V2 manifest, forms re-laid out on WM_DPICHANGED, overlay on the monitor of
  the foreground window (verified on a two-monitor 100 %/150 % setup).
- **Accessibility**: MSAA roles verified (push button / checkbox / combobox with label names); overlay states use text +
  icon + shape, not colour alone; animations off when Windows animations are disabled.
- **Security**: engine DLL loaded by absolute path with restricted search and pinned SHA-256; model pinned revision +
  SHA-256; control characters never inserted (no terminal command injection); no elevation requested.
- **Privacy**: no network except the user-confirmed download (no sockets observed during e2e); audio never written to
  disk; transcripts not logged by default; the log does contain device names and target process names (e.g.
  `Code.exe`) for diagnostics.
- **Licences**: Apache-2.0 engine/model attributed, licence texts shipped, crate list generated; the project's own
  licence is intentionally left to the author.
- **Installer**: per-user, no admin, version info, uninstall entry, Start-menu shortcut, optional autostart,
  uninstall removes the Run value and offers to delete user data — silent install/uninstall verified.

## Final test pass (audited build)

| Suite | Result |
|---|---|
| Unit tests (`cargo test --lib`) | 35 / 35 |
| End-to-end, paste mode (Notepad, PowerShell, Edge, VS Code, WinForms, 45 s, silence/noise ×4, 15 rapid taps, Esc, toggle mode, clipboard restore, no sockets, no transcript in log) | 24 / 24 |
| End-to-end, "Type characters" mode (PowerShell, Edge, VS Code, WinForms) | 12 / 12 |
| Settings window (real mic test, vocabulary, shortcut capture, autostart registry) | 11 / 11 |
| First run (download only after click, cancel + resume, SHA-256, offline restart, corrupted model) | 9 / 9 |
| Accuracy / speech gate (`eval.py`, 95 cases) | 86 / 86 speech transcribed, 9 / 9 non-speech rejected; WER as in PERFORMANCE.md |
| Installer (silent install, files, version, uninstall entry, uninstall) | passed |

## Remaining limitations (by design or not testable here)

- Elevated (administrator) target windows: hook + fallback logic is code-reviewed; the paste is replaced by
  "text on the clipboard, press Ctrl+V". Not tested (needs a UAC prompt).
- Physically unplugging the microphone during a recording: code-reviewed (capture ends, what was recorded is
  transcribed, overlay says "Microphone disconnected"); not physically tested.
- "Type characters" loses characters in Windows 11 Notepad (paste mode works there).
- If focus moves to an admin window *while* the key is held, that key-up is invisible to a normal process; pressing
  the key again ends the recording (max length 5 min otherwise).
- Builds are not code-signed (SmartScreen warning on first run).

## Round 3 (2026-10-05): Auto (Polish + English), Polish/English interface, final audit

New features
- **Auto (Polish + English)** recognition mode, now the default: Whistle detects the language; a detection other than
  Polish/English is re-transcribed as Polish (`LanguagePlan` in `engine.rs`, unit-tested). On the test set it removes the
  4 "Polish heard as Spanish" errors of pure auto-detection; WER equals forced Polish at normal speed.
- **Interface language** Polish / English (`i18n.rs`): one table, both translations required at compile time, placeholder
  consistency unit-tested; default = Windows display language; live switch rebuilds open windows; installer messages
  localised. Logs stay mostly English (error texts follow the UI language).

Findings and fixes in this round

| # | Severity | Finding | Fix |
|---|---|---|---|
| C1 | medium | Single-instance mutex was global: a test/portable run with its own data folder silently handed over to the user's running installed copy | Mutex scoped by data folder when `WHISTLETYPE_DATA_DIR` is set; normal installs still run once |
| C2 | medium (tests) | Test scripts found windows by class name and could have touched the user's own running instance | All UI scripts filter windows by the test process id |
| C3 | low | Polish UI showed "16.9 MB" | Decimal comma in Polish |
| C4 | low | Polish captions clipped ("Przywróć domyślne", setup description) | Wider button column / taller text area; checked with screenshots in both languages |
| C5 | info | Cross-process `GetWindowText` does not read combo boxes (test-only issue) | Tests address controls by id |
| C6 | info | `cargo clippy`: 8 style warnings, no correctness lints | 5 auto-fixed; remaining 3 are stylistic (argument count, complex type, missing `# Safety` doc) |

Final test pass (this build)

| Suite | Result |
|---|---|
| Unit tests | 36 / 36 |
| End-to-end, paste mode | 24 / 24 |
| End-to-end, "Type characters" | 12 / 12 |
| Settings window incl. live interface-language switch | 13 / 13 |
| First run (download, cancel/resume, SHA-256, offline restart, corrupted model) | 9 / 9 |
| Accuracy with Auto (Polish + English) | 86 / 86 speech transcribed, 9 / 9 non-speech rejected |
| Installer | Built; silent install/uninstall **not re-run** because the user's own installation was running (it would have been closed and removed). The previous installer build passed it; this round only added localised installer messages. |


## Round 4 (2026-10-05/06): FAST + ACCURATE engines, Model Manager, GPU — fresh audit of the whole project

New features (version **1.1.0**)
- **ACCURATE engine**: OpenAI Whisper via the official whisper.cpp binaries (b5130), C API called directly from Rust
  (`whisper.rs`), behind a common `SpeechEngine` trait (`stt.rs`) shared with Whistle. Runtime loaded on demand;
  struct layout verified against MSVC and checked at run time against the library's defaults.
- **Modes** FAST (Whistle, CPU) / ACCURATE (Whisper, GPU if available) / AUTO (default: Whisper only when it is ready
  on the GPU, otherwise Whistle) — in Settings → Recognition and in the tray menu. Fallbacks: no GPU pack/no NVIDIA
  GPU → CPU runtime; GPU init failure → CPU context; GPU error at run time → reload on CPU and retry once; Whisper not
  ready → that dictation uses FAST and says so.
- **Model Manager** ("Speech models"): Whistle, 4 Whisper models and the optional CUDA 12.4 GPU pack with size, Polish
  quality (measured), where it runs and state; download (confirmation, pinned URL, resume, SHA-256; the GPU pack is
  unpacked and every DLL verified), delete (the pack in use is removed at the next start), "Use for ACCURATE".
- Whisper hallucination guard (subtitle credits such as "Napisy stworzone przez społeczność Amara.org" are removed).
- Benchmark tooling: `bench-models.py`, `fetch-fleurs.py`, `fetch-whisper-models.py`, `bench-table.py`,
  `perf-accurate.ps1`; results in PERFORMANCE.md.

Findings and fixes in this round

| # | Severity | Finding | Fix |
|---|---|---|---|
| D1 | high | Closing the first-run window ("Not now") cancelled **any** running download — including a GPU-pack or Whisper download started in Speech models; a Whisper download error was also shown there as a FAST-model error | The first-run window only shows, cancels and retries the FAST model; it says "another download is in progress" otherwise |
| D2 | medium | With the FAST model missing or broken, dictation was refused even when Whisper was ready in ACCURATE/AUTO (tray "Start dictation" was greyed out too) | Dictation and the tray item are allowed when Whisper is ready or loading |
| D3 | medium | Temporary download name was hard-coded to `*.cact.part` | `<file>.part` for every download; deleting a Whisper model also removes its unfinished `.part` |
| D4 | medium | `Path::with_extension` cut "whisper-cuda-12.4-b5130" at the last dot: staging folder and removal marker were named `whisper-cuda-12.part` / `.remove` (worked, but fragile and confusing) | `sibling()` helper appends the suffix; unit-tested |
| D5 | medium | Settings window grew to ~880 DIP: taller than the work area at 125 % scaling on a 1080p screen (bottom buttons unreachable) | `Form::fit` adds a vertical scroll bar only when needed (wheel, scroll bar, focus kept visible on Tab); rounding fixed so a fitting form never shows one |
| D6 | medium | "Type characters" mode lost letters in Windows 11 Notepad 11.2607 at 6 ms per character (13/13 → 11/13, not caused by this round's code) | 12 ms per character; 21/21 in Notepad, PowerShell, Edge, VS Code, WinForms |
| D7 | low | AUTO evaluated the GPU via DXGI enumeration on every check | Enumerated once per process |
| D8 | low | `wt-bench` lost the `segments` field / load timings used by `eval.py` after the rewrite | Restored; eval runs again |
| D9 | low | VRAM column of the first benchmark was 0 (per-process GPU memory is "N/A" under WDDM) | Sampler measures the card's memory rise over a baseline; GPU configurations re-run |
| D10 | low | i18n placeholder test covered 6 hand-picked strings | Generated table of all strings; every Polish text must have exactly the English placeholders |
| D11 | info | Screenshots for docs could show the Windows user name in paths / screen content behind windows | New `capture-windows.ps1` renders only the app's windows (PrintWindow) from a data folder inside the repo |
| D12 | info | clippy: 3 remaining style warnings from earlier rounds + 3 new | All fixed; `cargo clippy --all-targets` is clean |

Reviewed and found correct (no change): FFI ownership (`whisper_free_params` after copying the defaults, context
freed on drop, model freed before the next one loads to release VRAM); C strings kept alive for the call;
zip extraction only writes the 16 expected names into a staging folder (no path traversal) and verifies size +
SHA-256 of each; downloads only after a user click + confirmation (tested: cancelling the confirmation makes no request);
no transcript text in logs (whisper.cpp log callback forwards warnings/errors only — checked in the e2e logs);
idle CPU 0 in all modes; FAST mode never loads Whisper; settings files from 1.0 load with AUTO (= FAST without a GPU pack).

Known limitations (documented in README)
- Only NVIDIA GPUs are used (official Windows GPU build is CUDA); on a CPU, Whisper small ≈ 7 s per sentence.
- Switching CPU ↔ GPU runtime after installing/removing the GPU pack needs an app restart (one ggml runtime per process).
- `SetDllDirectoryW` points to the whisper runtime folder for the life of the process (needed because ggml loads its
  backends with plain `LoadLibrary`); the folder contains only SHA-verified DLLs.
- AUTO keeps ~0.5 GB RAM and ~1 GB VRAM while Whisper is loaded on the GPU; choose FAST to free them.

Final test pass (build 1.1.0)

| Suite | Result |
|---|---|
| Unit tests | 40 / 40 |
| `cargo clippy --all-targets` | 0 warnings |
| End-to-end, paste mode (FAST) | 24 / 24 (incl. no network sockets during dictation) |
| End-to-end, "Type characters" | 21 / 21 |
| **End-to-end ACCURATE** (`accurate.ps1`): CPU, GPU-pack install via Model Manager with resumed network download, GPU after restart, AUTO, FAST, removal of the pack in use | 17 / 17 |
| Settings + Model Manager + interface language | 23 / 23 |
| First run (download, cancel/resume, SHA-256, offline restart, corrupted model) | 9 / 9 |
| Accuracy (FAST, synthetic set) | 86 / 86 speech transcribed, 9 / 9 non-speech rejected (unchanged) |
| FAST vs ACCURATE benchmark | PERFORMANCE.md — e.g. FLEURS WER: Whistle 35.5 %, Whisper large-v3-turbo GPU 5.7 % |
| Installer / portable ZIP 1.1.0 | Built (`whisper-cpu\` included). Install/uninstall **not run**: the user's own installed copy was running |

## Round 5 (2026-10-06): final audit of the application and code

Scope: all of `src/` (≈ 9 600 lines), with the new FFI/engine/download/install code read line by line, plus
cross-cutting checks (network, files written, logs, panics, external processes) and extra runtime tests.

| # | Severity | Finding | Fix / evidence |
|---|---|---|---|
| E1 | medium | **AUTO** loaded Whisper on the **CPU** when the GPU could not be used (pack installed but CUDA failing): ~30 s of blocked engine and ~1 GB RAM for a model AUTO then never uses (it dictates with FAST on a CPU). A GPU error at run time also reloaded on the CPU | `gpu_only` load requests in AUTO: no GPU → no load, the GPU error path drops Whisper and finishes that dictation with Whistle. New e2e case with CUDA hidden (`CUDA_VISIBLE_DEVICES=-1`) |
| E2 | low | `LoadLibraryExW(LOAD_WITH_ALTERED_SEARCH_PATH)` needs an absolute path; a relative runtime folder would fail (only reachable from developer tools) | Runtime folder made absolute in `WhisperRuntime::open` |
| E3 | low | Context default params pointer was not null-checked when loading a model (only at start-up) | Checked; a NULL fails the load cleanly |
| E4 | low | In AUTO a missing GPU showed a raw error in Settings | Explains that AUTO uses FAST without the GPU pack/GPU, with the reason |
| E5 | info | No unit test for the new settings fields | Test: a 1.0 settings file gives AUTO/no model/GPU on; invalid values repaired |

Verified without changes
- **Non-ASCII paths** (Polish user names are common in `%LOCALAPPDATA%`): Whisper runtime and model loaded from
  `…\test-Paweł-łąę\…` (whisper.cpp converts UTF-8 to a wide path; ggml backends loaded from the same folder).
- **Memory**: 120 Whisper large-v3-turbo transcriptions on the GPU in one process — private memory flat (1943 MB,
  mostly CUDA reservation), working set levels off at 525 MB after ~90 runs. No leak.
- **Network**: WinHTTP only in `download.rs`, reached only from the Download buttons after a confirmation.
- **Files written**: settings (atomic temp + rename), log, downloads (`.part` → verified → rename), GPU-pack staging
  folder and removal marker. Recordings are never written.
- **Logs**: transcript text only with the opt-in `log_transcripts`; engine errors contain no text.
- **External processes**: none started; `ShellExecute` opens only the log folder and the fixed model page URL.
- **Panics**: every `unwrap`/`expect`/index in non-test code reviewed — all guarded by earlier checks or fixed
  catalogue data; worker threads are created at start-up only.
- **FFI**: struct layouts asserted (unit test + runtime defaults check); every whisper.cpp object freed once
  (`Drop`), parameters copied before `whisper_free_params`, C strings outlive the call, single engine thread.
- **Supply chain**: every downloaded or bundled binary pinned by URL/revision and SHA-256 (per file for the GPU pack).

Final test pass (build 1.1.0)

| Suite | Result |
|---|---|
| Unit tests | 41 / 41 |
| `cargo clippy --all-targets` | 0 warnings |
| End-to-end ACCURATE (CPU, GPU-pack install, GPU, AUTO, AUTO without usable GPU, FAST, pack removal) | 18 / 18 |
| End-to-end, paste mode | 24 / 24 |
| End-to-end, "Type characters" | 21 / 21 |
| Settings + Model Manager + interface language | 23 / 23 |
| First run | 9 / 9 |
| Accuracy (FAST) | 86 / 86 speech, 9 / 9 non-speech (unchanged) |
| Installer / portable ZIP | Rebuilt (`dist/`); install/uninstall not run while the user's own copy is running |
