export const METER_FLOOR_DB = -60;
export const METER_CEILING_DB = -12;
const ATTACK_MS = 45;
const RELEASE_MS = 180;

/** Display calibration only: the captured RMS and transcript remain untouched. */
export function normalizeRms(rms: number): number {
  if (!Number.isFinite(rms) || rms <= 0) return 0;
  const db = 20 * Math.log10(Math.min(rms, 1));
  return Math.max(
    0,
    Math.min(1, (db - METER_FLOOR_DB) / (METER_CEILING_DB - METER_FLOOR_DB)),
  );
}

export function smoothMeter(
  previous: number,
  target: number,
  elapsedMs: number,
): number {
  const current = Number.isFinite(previous)
    ? Math.max(0, Math.min(1, previous))
    : 0;
  const next = Number.isFinite(target) ? Math.max(0, Math.min(1, target)) : 0;
  const elapsed = Number.isFinite(elapsedMs) ? Math.max(0, elapsedMs) : 0;
  const timeConstant = next > current ? ATTACK_MS : RELEASE_MS;
  const value =
    current + (next - current) * (1 - Math.exp(-elapsed / timeConstant));
  return next === 0 && value < 0.005 ? 0 : value;
}

export function meterDescription(rms: number, active: boolean): string {
  if (!active) return "Audio input inactive";
  if (!Number.isFinite(rms) || rms <= 0) return "Silence";
  if (normalizeRms(rms) === 0) return `Below ${METER_FLOOR_DB} dBFS`;
  return `${Math.round(20 * Math.log10(Math.min(rms, 1)))} dBFS`;
}
