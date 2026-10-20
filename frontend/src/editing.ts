export type Cut = { id: string; media_id: string; start_ms: number; end_ms: number }

export function removedDuration(cuts: Cut[], mediaId: string, duration: number) {
  const ranges = cuts.filter(cut => cut.media_id === mediaId)
    .map(cut => [Math.max(0, cut.start_ms), Math.min(duration, cut.end_ms)])
    .filter(([start, end]) => end > start).sort((a, b) => a[0] - b[0])
  let total = 0, cursor = 0
  for (const [start, end] of ranges) {
    total += Math.max(0, end - Math.max(cursor, start))
    cursor = Math.max(cursor, end)
  }
  return total
}

export function readCuts(value: string | null): Cut[] {
  try {
    const parsed: unknown = JSON.parse(value ?? '[]')
    if (!Array.isArray(parsed)) return []
    return parsed.filter((cut): cut is Cut => cut !== null && typeof cut === 'object' &&
      typeof cut.id === 'string' && typeof cut.media_id === 'string' &&
      Number.isSafeInteger(cut.start_ms) && Number.isSafeInteger(cut.end_ms) &&
      cut.start_ms >= 0 && cut.end_ms > cut.start_ms)
  } catch { return [] }
}
