// Serialização única de tudo que o oráculo grava, para que regenerar produza bytes idênticos.
//
// Limites conhecidos (documentados no README): `-0` vira `0` (JSON não distingue) e valores não
// finitos viram `{ "$nonFinite": "NaN" | "Infinity" | "-Infinity" }` em vez de `null`, para não
// confundir NaN com ausência de valor. Chaves mantêm a ordem de inserção do harness (estável).

const replacer = (_key, value) => {
  if (typeof value === "number" && !Number.isFinite(value)) {
    return { $nonFinite: String(value) }
  }
  return value
}

export const serialize = (value) => `${JSON.stringify(value, replacer, 2)}\n`
