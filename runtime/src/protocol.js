// Bridge v1 constants and the error type shared by every runtime module.
// The Rust side lives in crates/core/src/bridge.rs; both must change together.

export const BRIDGE_VERSION = 1;
export const RUNTIME_NAME = 'tessel-three';
export const RUNTIME_VERSION = '0.1.0';

/** Layer kinds of the render plan this runtime draws (crates/core/src/plan.rs). */
export const LAYERS = ['image', 'video', 'text', 'scene', 'waveform'];

/** Blend modes drawn with fixed-function blending. `overlay` needs the backdrop and is refused. */
export const BLEND_MODES = ['normal', 'multiply', 'screen', 'darken', 'lighten'];

export class RuntimeError extends Error {
  constructor(code, message, details) {
    super(message);
    this.code = code;
    this.details = details;
  }

  toJSON() {
    const error = { code: this.code, message: this.message };
    if (this.details !== undefined) error.details = this.details;
    return error;
  }
}

/** Normalizes anything thrown into the `{code, message}` shape of the bridge. */
export const toErrorJson = (error, fallbackCode = 'render_failed') => {
  if (error instanceof RuntimeError) return error.toJSON();
  return { code: fallbackCode, message: error?.message ?? String(error) };
};

/** Mirrors `expand_pattern` in crates/core/src/assets.rs: `%d` and `%0Nd`. */
export const expandPattern = (pattern, index) => {
  const match = /%(0?)(\d*)d/.exec(pattern);
  if (!match) return null;
  const width = match[2] === '' ? 0 : Number(match[2]);
  const digits = String(index).padStart(match[1] === '0' ? width : 0, '0');
  return pattern.slice(0, match.index) + digits + pattern.slice(match.index + match[0].length);
};
