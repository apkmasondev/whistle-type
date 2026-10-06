"""Composes derived test cases from the TTS clips in tests/audio/generated/tts.

Writes 16 kHz mono PCM16 WAVs to tests/audio/generated/cases and a manifest.json with the expected
reference text for each case (None = nothing must be typed). Requires numpy.
"""
import json
import wave
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent / "tests" / "audio" / "generated"
TTS = ROOT / "tts"
OUT = ROOT / "cases"
SR = 16000
rng = np.random.default_rng(1234)

SENTENCES = {
    "pl_basic": "To jest test rozpoznawania mowy.",
    "pl_tech": "Uruchom Gradle i sprawdź aplikację Kotlin Compose.",
    "pl_mixed": "Sprawdź komponent React i popraw useEffect.",
    "pl_mixed_long": "Sprawdź komponent React i zobacz, czy useEffect nie powoduje ponownego renderowania.",
    "pl_gradle": "Uruchom Gradle i sprawdź build release.",
    "pl_diacritics": "Zażółć gęślą jaźń. Źdźbło trawy, łódź i chrząszcz brzmią w trzcinie.",
    "pl_claude": "Otwórz Claude Code i poproś Codex o przegląd kodu w TypeScript.",
    "pl_web": "Zbuduj projekt w Vite, potem sprawdź shader WebGL i model Three.js w Blenderze.",
    "pl_github": "Wypchnij zmiany na GitHub i opublikuj stronę na GitHub Pages.",
    "pl_story": "Wczoraj wieczorem pracowałem nad nową wersją aplikacji. Najpierw poprawiłem błędy w interfejsie, potem dodałem testy, a na końcu przygotowałem notatki do wydania. Dzisiaj chcę jeszcze sprawdzić wydajność i zużycie pamięci na starszym laptopie.",
}


def read(path):
    with wave.open(str(path), "rb") as w:
        ch, width, rate, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
        raw = w.readframes(n)
    assert width == 2, path
    x = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    if ch > 1:
        x = x.reshape(-1, ch).mean(axis=1)
    if rate != SR:
        # good-enough polyphase-free resampling for test material (band-limited by FFT)
        n_out = int(round(len(x) * SR / rate))
        spec = np.fft.rfft(x)
        keep = n_out // 2 + 1
        spec = spec[:keep] if keep <= len(spec) else np.pad(spec, (0, keep - len(spec)))
        x = np.fft.irfft(spec, n_out).astype(np.float32) * (n_out / len(x))
    return x


def write(name, x):
    OUT.mkdir(parents=True, exist_ok=True)
    x = np.clip(x, -1, 1)
    with wave.open(str(OUT / f"{name}.wav"), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes((x * 32767).astype("<i2").tobytes())


def sil(sec):
    return np.zeros(int(sec * SR), np.float32)


def room(sec, level=0.002):
    return (rng.standard_normal(int(sec * SR)) * level).astype(np.float32)


def pink(n):
    white = rng.standard_normal(n)
    spec = np.fft.rfft(white)
    f = np.arange(len(spec))
    f[0] = 1
    return (np.fft.irfft(spec / np.sqrt(f), n)).astype(np.float32)


def at_rms(x, rms):
    cur = np.sqrt(np.mean(x ** 2)) + 1e-12
    return x * (rms / cur)


def main():
    manifest = {}
    clips = {p.stem: read(p) for p in sorted(TTS.glob("*.wav"))}
    voices = sorted({k.split("__")[1] for k in clips})
    print("voices:", voices)
    main_voice = next((v for v in voices if v.startswith("paulina_normal")), voices[0])

    def clip(sentence, voice=main_voice):
        return clips[f"{sentence}__{voice}"]

    # 1) every sentence with every voice/rate (accuracy on Polish + English terms, fast/slow speech)
    for key, x in clips.items():
        sentence = key.split("__")[0]
        name = f"sent__{key}"
        write(name, np.concatenate([room(0.3), x, room(0.3)]))
        manifest[name] = {"ref": SENTENCES[sentence], "kind": "sentence"}

    # 2) lengths: ~1 s, 5 s, 10 s, 20 s, 29 s, 45 s, 90 s (the last two exceed Whistle's 30 s limit)
    order = ["pl_basic", "pl_tech", "pl_mixed", "pl_gradle", "pl_claude", "pl_web", "pl_github", "pl_diacritics", "pl_story", "pl_mixed_long"]
    one = clip("pl_basic")
    # ~1 s: first word(s) of a clip
    write("len_01s", np.concatenate([room(0.2), one[: int(1.0 * SR)], room(0.1)]))
    manifest["len_01s"] = {"ref": None, "kind": "length", "note": "partial sentence, any short text is fine"}
    for target in (5, 10, 20, 29, 45, 90):
        parts, refs, total = [room(0.3)], [], 0.3
        i = 0
        while True:
            s = order[i % len(order)]
            x = clip(s)
            if total + len(x) / SR + 0.45 > target and refs:
                break
            parts += [x, room(0.45)]
            refs.append(SENTENCES[s])
            total += len(x) / SR + 0.45
            i += 1
        x = np.concatenate(parts)
        name = f"len_{target:02d}s"
        write(name, x)
        manifest[name] = {"ref": " ".join(refs), "kind": "length", "seconds": round(len(x) / SR, 2)}

    # 3) non-speech: nothing may be typed
    write("ns_digital_silence", sil(3))
    write("ns_room_tone", room(3, 0.003))
    write("ns_white_noise", (rng.standard_normal(3 * SR) * 0.08).astype(np.float32))
    write("ns_pink_noise", at_rms(pink(4 * SR), 0.06))
    hum = (0.05 * np.sin(2 * np.pi * 50 * np.arange(3 * SR) / SR) + 0.02 * np.sin(2 * np.pi * 150 * np.arange(3 * SR) / SR)).astype(np.float32)
    write("ns_mains_hum", hum + room(3, 0.002))
    click = room(1.5, 0.002)
    click[SR // 2: SR // 2 + 160] += np.hanning(160).astype(np.float32) * 0.9
    write("ns_single_click", click)
    keys = room(3, 0.002)
    for t in np.arange(0.2, 2.8, 0.17):
        i = int(t * SR)
        keys[i: i + 240] += (rng.standard_normal(240) * np.hanning(240) * 0.3).astype(np.float32)
    write("ns_keyboard_typing", keys)
    write("ns_too_short_200ms", one[int(0.2 * SR): int(0.4 * SR)])
    breath = at_rms(pink(2 * SR), 0.01) * np.hanning(2 * SR).astype(np.float32)
    write("ns_breath", breath + room(2, 0.001))
    for n in ("ns_digital_silence", "ns_room_tone", "ns_white_noise", "ns_pink_noise", "ns_mains_hum",
              "ns_single_click", "ns_keyboard_typing", "ns_too_short_200ms", "ns_breath"):
        manifest[n] = {"ref": None, "kind": "nonspeech"}

    # 4) speech in noise (SNR 15 dB and 5 dB) and quiet speech (low mic gain)
    for sentence in ("pl_mixed", "pl_tech", "pl_basic"):
        x = clip(sentence)
        speech_rms = np.sqrt(np.mean(x ** 2))
        for snr in (15, 5):
            n = at_rms(pink(len(x) + SR), speech_rms / (10 ** (snr / 20)))
            y = n.copy()
            y[SR // 2: SR // 2 + len(x)] += x
            name = f"noisy{snr}db__{sentence}"
            write(name, y)
            manifest[name] = {"ref": SENTENCES[sentence], "kind": "noisy"}
        name = f"quiet__{sentence}"
        write(name, np.concatenate([room(0.3, 0.0003), x * 0.05, room(0.3, 0.0003)]))
        manifest[name] = {"ref": SENTENCES[sentence], "kind": "quiet"}

    (ROOT / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=1), encoding="utf-8")
    print("cases:", len(manifest))


if __name__ == "__main__":
    main()
