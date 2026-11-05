"""Spectral check shared by the proof scripts: is a given tone present in an exported file?

Decodes a slice of the file's audio with ffmpeg and measures one frequency with the Goertzel
algorithm. This proves *signal content* (the video's own sound reached the mix), never that anyone
heard it.
"""
import math
import struct
import subprocess

RATE = 48000


def tone_level(path, start, duration, freq):
    """RMS amplitude (0..1) of `freq` Hz in `[start, start + duration)` seconds of the file's audio."""
    raw = subprocess.run(
        ["ffmpeg", "-loglevel", "error", "-ss", str(start), "-t", str(duration), "-i", path,
         "-vn", "-ac", "1", "-ar", str(RATE), "-f", "s16le", "-"],
        capture_output=True, check=True).stdout
    count = len(raw) // 2
    if count == 0:
        return 0.0
    samples = struct.unpack(f"<{count}h", raw[: count * 2])
    omega = 2 * math.pi * freq / RATE
    coeff = 2 * math.cos(omega)
    previous = previous2 = 0.0
    for value in samples:
        previous, previous2 = value / 32768 + coeff * previous - previous2, previous
    power = previous2 ** 2 + previous ** 2 - coeff * previous * previous2
    return 2 * math.sqrt(max(power, 0.0)) / count


def explain(path, windows, tones):
    """`{window: {freq: level}}` for printing next to a failed check."""
    return {f"{start}-{start + duration}s": {freq: round(tone_level(path, start, duration, freq), 4) for freq in tones}
            for start, duration in windows}
