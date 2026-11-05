// Container video drawn straight from a browser `<video>` element (no media-adapter frames).
//
// TIME CONTRACT (the same one the core uses for headless, export and the UI):
//   - A clip has a constant frame rate `r = rate.num/rate.den` and its first picture is
//     presented at PTS 0. Frame `f` (0-based) owns the PTS interval `[f/r, (f+1)/r)`.
//   - Frame `f` is read by seeking to the *middle* of its interval, `(f + 1/2)/r`, never to its
//     first instant. A browser seek lands on "the frame shown at that time"; on an exact
//     boundary the answer depends on rounding in the demuxer and WebKit/Chromium returned the
//     previous picture (B2), while the middle of an interval is stable for every engine tried.
//     This is a property of the contract, not a magic offset: the target is derived from the
//     rate, so any frame rate gets its own middle.
//   - Variable frame rate and clips whose first picture is not at 0 do not fit that contract.
//     The media adapter conforms those to a constant rate while it decodes frames (it reads
//     timestamps); the native path refuses them (`unsupported_timing`) instead of guessing.
//
// READ PATH: one path only, `createImageBitmap(video)`. Frames are snapshotted into the same
// per-asset frame cache sequences use (bounded, LRU, budgeted texture), so two layers of one
// asset at different times each get their own picture, a wait can be abandoned when a newer
// seek overtakes it, and the GPU upload is a plain image upload. Mixing readers on one
// `<video>` (drawImage + createImageBitmap + texImage2D) made WebKit stall for ~1 s on the
// first mixed read (B4, see docs/viewport-bugfix-report.md), so nothing else touches the element.

import { RuntimeError } from './protocol.js';

const LOAD_TIMEOUT_MS = 30_000;
const SEEK_TIMEOUT_MS = 10_000;
const ABORT_POLL_MS = 20;
const FINGERPRINT_SIZE = 32;

/** Seconds at the middle of frame `frame`: `(frame + 1/2) * den / num`, see the contract above. */
export const frameMidpoint = (frame, rate) => ((2 * frame + 1) * rate.den) / (2 * rate.num);

/** Number of whole frames a clip of `duration` seconds holds at `rate`. */
export const frameCount = (duration, rate) => Math.max(1, Math.round((duration * rate.num) / rate.den));

const waitFor = (video, events, { timeoutMs, aborted, describe }) =>
  new Promise((resolve, reject) => {
    const cleanups = [];
    const done = (callback) => (value) => {
      cleanups.forEach((cleanup) => cleanup());
      callback(value);
    };
    const succeed = done(resolve);
    const fail = done(reject);
    events.forEach((name) => {
      video.addEventListener(name, succeed, { once: true });
      cleanups.push(() => video.removeEventListener(name, succeed));
    });
    video.addEventListener('error', onError, { once: true });
    cleanups.push(() => video.removeEventListener('error', onError));
    function onError() {
      fail(new RuntimeError('decode_failed', `Cannot decode ${describe}: ${video.error?.message || 'unsupported codec or container'}`));
    }
    const timer = setTimeout(
      () => fail(new RuntimeError('timeout', `${describe} did not complete in ${timeoutMs} ms`)),
      timeoutMs,
    );
    cleanups.push(() => clearTimeout(timer));
    if (aborted) {
      const poll = setInterval(() => {
        if (aborted()) fail(new RuntimeError('cancelled', 'The frame was superseded'));
      }, ABORT_POLL_MS);
      cleanups.push(() => clearInterval(poll));
    }
  });

/** Loads the element up to the first decoded picture; rejects with a structured error. */
export const openVideo = async (url, label) => {
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  video.crossOrigin = 'anonymous';
  const loaded = waitFor(video, ['loadeddata'], { timeoutMs: LOAD_TIMEOUT_MS, describe: `\`${label}\`` });
  video.src = url;
  try {
    await loaded;
  } catch (error) {
    closeVideo(video);
    throw error;
  }
  if (!(video.videoWidth > 0 && video.videoHeight > 0)) {
    closeVideo(video);
    throw new RuntimeError('decode_failed', `${label} has no decodable picture (0x0)`);
  }
  return video;
};

export const closeVideo = (video) => {
  video.pause();
  video.removeAttribute('src');
  video.load();
};

/**
 * Moves the element to `seconds` and resolves once it shows a decoded picture there. `seeked`
 * alone is not enough on every engine: the picture must also be current (`readyState >= 2`).
 */
const seekTo = async (video, seconds, { aborted, describe }) => {
  if (video.currentTime === seconds && !video.seeking && video.readyState >= 2) return;
  const seeked = waitFor(video, ['seeked'], { timeoutMs: SEEK_TIMEOUT_MS, aborted, describe });
  video.currentTime = seconds;
  await seeked;
  if (video.readyState >= 2) return;
  await waitFor(video, ['canplay', 'loadeddata'], { timeoutMs: SEEK_TIMEOUT_MS, aborted, describe });
};

/** A tiny fixed-size digest of a picture, enough to tell two pictures apart. */
const fingerprint = (bitmap, label) => {
  try {
    return digest(bitmap);
  } catch (error) {
    // WebKit hands out a bitmap of a WebM picture and only refuses to read it back (tainted).
    throw new RuntimeError(
      'unsupported_format',
      `This engine cannot read pictures of ${label} (${error.name}); import it so the media adapter prepares frames`,
      { engine_error: error.name },
    );
  }
};

const digest = (bitmap) => {
  const canvas = document.createElement('canvas');
  canvas.width = FINGERPRINT_SIZE;
  canvas.height = FINGERPRINT_SIZE;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  context.drawImage(bitmap, 0, 0, FINGERPRINT_SIZE, FINGERPRINT_SIZE);
  const { data } = context.getImageData(0, 0, FINGERPRINT_SIZE, FINGERPRINT_SIZE);
  let hash = 2166136261;
  for (const byte of data) hash = Math.imul(hash ^ byte, 16777619);
  return hash >>> 0;
};

const snapshot = async (video, label) => {
  try {
    return await createImageBitmap(video);
  } catch (error) {
    // WebKit marks WebM (VP8/VP9) pictures as cross-origin even from our own scheme, so every
    // read path throws SecurityError there; other failures are the engine refusing the codec.
    throw new RuntimeError(
      'unsupported_format',
      `This engine cannot read pictures of ${label} (${error.name}); import it so the media adapter prepares frames`,
      { engine_error: error.name },
    );
  }
};

/**
 * Proves the engine can show pictures of this clip and that seeks really change them: reads the
 * first, a middle and the last frame, and rejects when a read fails or every picture is the
 * same. That second case is how AV1 in WebKit fails (stuck on the first frame) and how a
 * black read looks; a genuinely static clip is rejected too, and the media adapter handles it.
 * Returns the first frame's bitmap so the first render does not read it again.
 */
export const proveVideo = async (video, { rate, label, frames }) => {
  const describe = `\`${label}\``;
  // A clip whose timeline starts after 0 (first picture at PTS > 0) shifts every frame by that
  // much under this contract; the engine reports it as where the seekable range begins.
  const start = video.seekable.length > 0 ? video.seekable.start(0) : 0;
  if (start > rate.den / (2 * rate.num)) {
    throw new RuntimeError(
      'unsupported_timing',
      `${label}: the first picture is at ${start.toFixed(3)} s, not 0; import it so the media adapter aligns it to frames`,
      { start },
    );
  }
  const probes = [...new Set([0, Math.floor((frames - 1) / 2), frames - 1])];
  const prints = [];
  let first = null;
  for (const frame of probes) {
    await seekTo(video, frameMidpoint(frame, rate), { describe });
    const bitmap = await snapshot(video, label);
    try {
      prints.push(fingerprint(bitmap, label));
    } catch (error) {
      bitmap.close();
      first?.close();
      throw error;
    }
    if (frame === 0) first = bitmap;
    else bitmap.close();
  }
  if (probes.length > 1 && new Set(prints).size === 1) {
    first?.close();
    throw new RuntimeError(
      'unsupported_format',
      `${label}: the engine shows the same picture at the start, middle and end (stuck decoder, black read or a static clip); import it so the media adapter prepares frames`,
      { probes },
    );
  }
  return first;
};

/**
 * Snapshot of frame `frame` of `video`: seek to its interval's middle, wait for the decoded
 * picture, read it. Seeks of one element are serialized by `entry.queue` because a `<video>`
 * has a single playhead; a task that was superseded before its turn does nothing.
 */
export const readFrame = (entry, frame, { aborted, timings }) => {
  const run = async () => {
    if (aborted()) throw new RuntimeError('cancelled', 'The frame was superseded');
    const describe = `\`${entry.label}\``;
    const seekStarted = performance.now();
    await seekTo(entry.video, frameMidpoint(frame, entry.rate), { aborted, describe });
    const readStarted = performance.now();
    const bitmap = await snapshot(entry.video, entry.label);
    if (timings) {
      timings.video_seek_ms = (timings.video_seek_ms ?? 0) + (readStarted - seekStarted);
      timings.video_read_ms = (timings.video_read_ms ?? 0) + (performance.now() - readStarted);
    }
    return bitmap;
  };
  const result = entry.queue.then(run, run);
  entry.queue = result.catch(() => {});
  return result;
};
