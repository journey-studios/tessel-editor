// Aritmética de ENTRADA do corpus estendido (K / BL018): o que o gerador precisa para ESCOLHER
// onde medir (frames, segundos, doubles vizinhos), nunca para calcular o que a origem responde.
//
// Nada aqui reimplementa uma função da origem. `exactFloorFrame` é a definição matemática do piso
// (BigInt), usada só para CLASSIFICAR uma resposta da origem em ponto flutuante; a expectativa
// gravada continua sendo a da origem.

const view = new DataView(new ArrayBuffer(8))

const bitsOf = (value) => {
  view.setFloat64(0, value)
  return view.getBigUint64(0)
}

const fromBits = (bits) => {
  view.setBigUint64(0, bits)
  return view.getFloat64(0)
}

/** Próximo double acima (`Number.EPSILON`-style ulp). Só para valores finitos positivos. */
export const nextUp = (value) => {
  if (!(Number.isFinite(value) && value > 0)) {
    throw new RangeError(`nextUp só aceita finitos positivos: ${value}`)
  }
  return fromBits(bitsOf(value) + 1n)
}

/** Próximo double abaixo. Só para valores finitos positivos. */
export const nextDown = (value) => {
  if (!(Number.isFinite(value) && value > 0)) {
    throw new RangeError(`nextDown só aceita finitos positivos: ${value}`)
  }
  return fromBits(bitsOf(value) - 1n)
}

/** Valor exato de um double finito como fração `{ num, den }` (decimais em string, den = 2^k). */
export const exactRationalOfDouble = (value) => {
  if (!Number.isFinite(value)) {
    throw new RangeError(`double não finito: ${value}`)
  }
  if (value === 0) {
    return { num: "0", den: "1" }
  }
  const bits = bitsOf(Math.abs(value))
  const exponentField = Number((bits >> 52n) & 0x7ffn)
  const fraction = bits & ((1n << 52n) - 1n)
  const mantissa = exponentField === 0 ? fraction : fraction | (1n << 52n)
  const exponent = (exponentField === 0 ? 1 : exponentField) - 1075
  let num = mantissa
  let den = 1n
  if (exponent >= 0) {
    num <<= BigInt(exponent)
  } else {
    den <<= BigInt(-exponent)
  }
  // Reduz por potências de 2: o denominador é potência de 2, então basta tirar fatores 2.
  while (den > 1n && num % 2n === 0n) {
    num /= 2n
    den /= 2n
  }
  return { num: String(value < 0 ? -num : num), den: String(den) }
}

const floorDivide = (numerator, denominator) => {
  const quotient = numerator / denominator
  const remainder = numerator % denominator
  return remainder !== 0n && remainder < 0n !== denominator < 0n
    ? quotient - 1n
    : quotient
}

/** Piso exato de `exact * fps` (definição matemática; NÃO é uma função da origem). */
export const exactFloorFrame = (exact, fps) =>
  Number(
    floorDivide(
      BigInt(exact.num) * BigInt(fps.numerator),
      BigInt(exact.den) * BigInt(fps.denominator),
    ),
  )

/** Frações exatas candidatas a "segundos" em torno do início do frame `frame`. */
export const frameBoundaryRational = (frame, fps) => ({
  num: String(BigInt(frame) * BigInt(fps.denominator)),
  den: String(fps.numerator),
})

export const rationalToDouble = (exact) => Number(exact.num) / Number(exact.den)

/** Quaternion de um Euler XYZ em graus (só para ESCREVER entradas; a origem recebe o quaternion). */
export const quaternionFromEulerXyzDegrees = (xDeg, yDeg, zDeg) => {
  const half = Math.PI / 360
  const [c1, c2, c3] = [Math.cos(xDeg * half), Math.cos(yDeg * half), Math.cos(zDeg * half)]
  const [s1, s2, s3] = [Math.sin(xDeg * half), Math.sin(yDeg * half), Math.sin(zDeg * half)]
  const q = {
    x: s1 * c2 * c3 + c1 * s2 * s3,
    y: c1 * s2 * c3 - s1 * c2 * s3,
    z: c1 * c2 * s3 + s1 * s2 * c3,
    w: c1 * c2 * c3 - s1 * s2 * s3,
  }
  const length = Math.hypot(q.x, q.y, q.z, q.w)
  return { x: q.x / length, y: q.y / length, z: q.z / length, w: q.w / length }
}

/** PRNG determinístico (xorshift32) para ordens "embaralhadas" reproduzíveis. */
export const seededShuffle = (items, seed) => {
  let state = seed >>> 0 || 1
  const next = () => {
    state ^= state << 13
    state >>>= 0
    state ^= state >>> 17
    state ^= state << 5
    state >>>= 0
    return state
  }
  const result = [...items]
  for (let index = result.length - 1; index > 0; index -= 1) {
    const swap = next() % (index + 1)
    ;[result[index], result[swap]] = [result[swap], result[index]]
  }
  return result
}
