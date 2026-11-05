// Seleção dos pontos de amostra (frames) de um bundle JÁ ACEITO pelo schema da origem.
//
// Isto escolhe ONDE medir, não QUAL é o resultado: os valores esperados vêm sempre das funções da
// origem. Os frames escolhidos são gravados no arquivo esperado, então um teste Rust consome a lista
// gravada e nunca precisa repetir esta seleção.

const unique = (values) => [...new Set(values)].sort((a, b) => a - b)

/** Frames de borda do tempo de edição: cada corte, o primeiro/último frame e fora do intervalo. */
export const selectEditFrames = (bundle, extraEditFrames = []) => {
  const frames = [0, bundle.editDurationFrames - 1]
  for (const shot of bundle.shots) {
    const { startFrame, endFrameExclusive } = shot.editRange
    frames.push(startFrame, endFrameExclusive - 1)
    if (startFrame + 1 < endFrameExclusive) {
      frames.push(startFrame + 1)
    }
    if (endFrameExclusive - 2 > startFrame) {
      frames.push(endFrameExclusive - 2)
    }
  }
  // Fora do intervalo: o oráculo grava a mensagem de erro real da origem.
  frames.push(-1, bundle.editDurationFrames, ...extraEditFrames)
  return unique(frames)
}

const trackBoundaries = (segments) => {
  const boundaries = [0]
  let cursor = 0
  for (const segment of segments) {
    cursor += segment.durationFrames
    boundaries.push(cursor)
  }
  return boundaries
}

/** Frames de borda de cada movimento de um clipe (trilha principal e camadas) e depois do fim. */
export const selectClipFrames = (clip) => {
  const tracks = [clip.segments, ...(clip.layers ?? []).map((l) => l.segments)]
  const frames = []
  let total = 0
  for (const segments of tracks) {
    const boundaries = trackBoundaries(segments)
    total = Math.max(total, boundaries.at(-1))
    boundaries.forEach((boundary, index) => {
      frames.push(boundary - 1, boundary, boundary + 1)
      const next = boundaries[index + 1]
      if (next !== undefined) {
        frames.push(boundary + Math.floor((next - boundary) / 2))
      }
    })
  }
  frames.push(total - 1, total, total + 5, -1)
  return unique(frames.filter((frame) => frame >= -1))
}

export const selectProbes = (bundle, extraEditFrames) => ({
  editFrames: selectEditFrames(bundle, extraEditFrames),
  clipFrames: Object.fromEntries(
    bundle.cameraClips.map((clip) => [clip.id, selectClipFrames(clip)]),
  ),
})
