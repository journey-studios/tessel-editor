// FNV-1a 64 over bytes. Done with two 32-bit lanes because BigInt is far too slow for the
// 8 MB of a 1080p readback; every product stays below 2^53 so doubles are exact.

const PRIME_LOW = 0x1b3; // FNV prime is 2^40 + 0x1b3
const TWO_32 = 4294967296;

export const fnv1a64 = (bytes) => {
  let high = 0xcbf29ce4;
  let low = 0x84222325;
  for (let index = 0; index < bytes.length; index += 1) {
    low = (low ^ bytes[index]) >>> 0;
    const product = low * PRIME_LOW;
    const nextLow = product % TWO_32;
    const carry = (product - nextLow) / TWO_32;
    // Multiplying by 2^40 only moves the low lane's bits into the high lane, shifted by 8.
    high = (high * PRIME_LOW + carry + ((low << 8) >>> 0)) % TWO_32;
    low = nextLow;
  }
  return high.toString(16).padStart(8, '0') + low.toString(16).padStart(8, '0');
};

/** Per-channel mean (0-255) of an RGBA buffer. */
export const meanColor = (rgba) => {
  const sums = [0, 0, 0, 0];
  for (let index = 0; index < rgba.length; index += 4) {
    sums[0] += rgba[index];
    sums[1] += rgba[index + 1];
    sums[2] += rgba[index + 2];
    sums[3] += rgba[index + 3];
  }
  const pixels = Math.max(1, rgba.length / 4);
  return sums.map((sum) => sum / pixels);
};
