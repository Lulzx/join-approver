"""An original boom-bap beat for the demo: 90 BPM, A minor, 9.5 bars.

Everything is synthesized here (no samples), so the track is ours to use.
Bar 1 is keys and vinyl only; the drums drop on bar 2, where the title lands.
The scene cuts in src/lib.rs sit on the same bar lines (BAR = 8/3 s).

    python3 scripts/beat.py   # writes media/beat.wav
"""
import wave
from pathlib import Path

import numpy as np

SR = 44100
BPM = 90
BEAT = 60 / BPM
BAR = 4 * BEAT
STEP = BEAT / 4                 # 16th note
BARS = 9.5
LEN = int(SR * BAR * BARS) + SR  # a second of tail for the last hit
SWING = 0.58                    # late offbeat 16ths: the head nod
DROP = 1                        # drums start on this bar (0-based)

rng = np.random.default_rng(7)
out = np.zeros(LEN)


def t(n):
    return np.arange(n) / SR


def step_time(bar, step):
    base = bar * BAR + step * STEP
    if step % 2 == 1:
        base += (SWING - 0.5) * 2 * STEP
    return base


def place(sig, at, gain=1.0):
    i = int(at * SR)
    if i >= LEN:
        return
    j = min(LEN, i + len(sig))
    out[i:j] += sig[: j - i] * gain


def lowpass(x, cutoff):
    a = np.exp(-2 * np.pi * cutoff / SR)
    y = np.empty_like(x)
    acc = 0.0
    for k, v in enumerate(x):
        acc = (1 - a) * v + a * acc
        y[k] = acc
    return y


def highpass(x, cutoff):
    return x - lowpass(x, cutoff)


# ---- drums ----
def kick():
    n = int(0.55 * SR)
    tt = t(n)
    freq = 45 + 85 * np.exp(-tt * 28)
    phase = 2 * np.pi * np.cumsum(freq) / SR
    body = np.sin(phase) * np.exp(-tt * 6.5)
    click = rng.standard_normal(n) * np.exp(-tt * 400) * 0.35
    return np.tanh((body + click) * 1.8)


def snare():
    n = int(0.32 * SR)
    tt = t(n)
    tone = np.sin(2 * np.pi * 185 * tt) * np.exp(-tt * 22) * 0.6
    noise = highpass(rng.standard_normal(n), 1200) * np.exp(-tt * 13)
    return np.tanh((tone + noise * 0.9) * 1.4)


def hat(open_=False):
    n = int((0.28 if open_ else 0.07) * SR)
    tt = t(n)
    x = highpass(rng.standard_normal(n), 7000)
    return x * np.exp(-tt * (9 if open_ else 70))


K, S, H, HO = kick(), snare(), hat(), hat(True)

# 16th-step patterns (0-15), a classic boom-bap shape with a variation bar
KICK_A = [0, 7, 10]
KICK_B = [0, 3, 10, 13]
SNARE = [4, 12]
GHOST = [15]

# ---- harmony ----
NOTE = {"A": 0, "B": 2, "C": 3, "D": 5, "E": 7, "F": 8, "G": 10, "G#": 11}


def hz(name, octave):
    semis = NOTE[name] + 12 * (octave - 4)  # relative to A4
    return 440 * 2 ** (semis / 12)


CHORDS = [  # (bass root, chord tones)
    (("A", 1), [("A", 3), ("C", 4), ("E", 4), ("G", 4)]),
    (("F", 1), [("F", 3), ("A", 3), ("C", 4), ("E", 4)]),
    (("D", 2), [("D", 3), ("F", 3), ("A", 3), ("C", 4)]),
    (("E", 1), [("E", 3), ("G#", 3), ("B", 3), ("D", 4)]),
]


def keys_note(f, dur):
    n = int(dur * SR)
    tt = t(n)
    env = (1 - np.exp(-tt * 60)) * np.exp(-tt * 1.3)
    trem = 1 + 0.12 * np.sin(2 * np.pi * 4.5 * tt)
    tone = np.sin(2 * np.pi * f * tt) + 0.35 * np.sin(2 * np.pi * 2 * f * tt) * np.exp(-tt * 4)
    tone += 0.08 * np.sin(2 * np.pi * 3.01 * f * tt) * np.exp(-tt * 9)  # bell bite
    return tone * env * trem


def bass_note(f, dur):
    n = int(dur * SR)
    tt = t(n)
    env = (1 - np.exp(-tt * 200)) * np.exp(-tt * 1.8)
    return np.tanh(np.sin(2 * np.pi * f * tt) * 1.6) * env


keys = np.zeros(LEN)
bass = np.zeros(LEN)
drums = np.zeros(LEN)

bars_total = int(np.ceil(BARS))
for bar in range(bars_total):
    root, tones = CHORDS[bar % 4]
    start = bar * BAR
    last = bar >= BARS - 0.5  # the final half bar: one chord, one kick, ring out
    # keys: a held chord on 1, a softer stab on the "and" of 2
    for name, octv in tones:
        sig = keys_note(hz(name, octv), BAR * (1.0 if last else 1.1))
        i = int(start * SR)
        j = min(LEN, i + len(sig))
        keys[i:j] += sig[: j - i] * 0.16
        if not last:
            stab = keys_note(hz(name, octv), BEAT * 1.2)
            i2 = int(step_time(bar, 6) * SR)
            j2 = min(LEN, i2 + len(stab))
            keys[i2:j2] += stab[: j2 - i2] * 0.07
    if bar < DROP:
        continue
    # bass follows the kick
    kp = KICK_B if bar % 4 == 3 else KICK_A
    if last:
        kp = [0]
    for s in kp:
        sig = bass_note(hz(*root), STEP * (6 if s == 0 else 3))
        i = int(step_time(bar, s) * SR)
        j = min(LEN, i + len(sig))
        bass[i:j] += sig[: j - i] * 0.55
    # drums
    for s in kp:
        place_at = step_time(bar, s)
        i = int(place_at * SR)
        j = min(LEN, i + len(K))
        drums[i:j] += K[: j - i] * 0.95
    if last:
        continue
    for s in SNARE:
        i = int(step_time(bar, s) * SR)
        j = min(LEN, i + len(S))
        drums[i:j] += S[: j - i] * 0.7
    for s in GHOST:
        i = int(step_time(bar, s) * SR)
        j = min(LEN, i + len(S))
        drums[i:j] += S[: j - i] * 0.16
    for s in range(0, 16, 2):
        h = HO if s == 14 else H
        vel = 0.32 if s % 4 == 0 else 0.22
        i = int(step_time(bar, s) * SR)
        j = min(LEN, i + len(h))
        drums[i:j] += h[: j - i] * vel
    for s in (3, 11):  # swung 16th hats for the shuffle
        i = int(step_time(bar, s) * SR)
        j = min(LEN, i + len(H))
        drums[i:j] += H[: j - i] * 0.12

# sidechain: keys dip under every kick
duck = np.ones(LEN)
for bar in range(DROP, bars_total):
    kp = KICK_B if bar % 4 == 3 else KICK_A
    for s in kp:
        i = int(step_time(bar, s) * SR)
        n = int(0.25 * SR)
        j = min(LEN, i + n)
        duck[i:j] = np.minimum(duck[i:j], 1 - 0.45 * np.exp(-t(j - i) * 12))

# lo-fi: warm the keys, filter bar 1 so the drop opens up
keys = lowpass(keys, 2600)
intro_end = int(DROP * BAR * SR)
keys[:intro_end] = lowpass(keys[:intro_end], 900)

# vinyl: hiss plus sparse crackle
vinyl = lowpass(highpass(rng.standard_normal(LEN), 3000), 9000) * 0.006
pops = rng.random(LEN) < 9 / SR
vinyl[pops] += rng.choice([-1, 1], pops.sum()) * rng.uniform(0.05, 0.18, pops.sum())

# riser into the drop: filtered noise swelling over the last beat of bar 1
r_len = int(BEAT * 1.6 * SR)
rr = t(r_len)
riser = highpass(rng.standard_normal(r_len), 2500) * (rr / rr[-1]) ** 2.5 * 0.18
place(riser, DROP * BAR - BEAT * 1.6)

mix = keys * duck + bass + drums + vinyl
out += mix

# tail fade and normalisation to -1 dBFS peak (fframes sets the final level)
fade = int(1.4 * SR)
out[-fade:] *= np.linspace(1, 0, fade) ** 2
out = np.tanh(out * 1.1)
out *= 10 ** (-1 / 20) / np.max(np.abs(out))

media = Path(__file__).resolve().parent.parent / "media"
media.mkdir(exist_ok=True)
with wave.open(str(media / "beat.wav"), "wb") as w:
    w.setnchannels(1)
    w.setsampwidth(2)
    w.setframerate(SR)
    w.writeframes((out * 32767).astype("<i2").tobytes())
print(f"media/beat.wav: {len(out) / SR:.2f} s, {BPM} BPM, bar {BAR:.4f} s")
