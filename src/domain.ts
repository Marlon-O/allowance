import type { Snapshot, UsageWindow } from './types';
export const remaining = (used: number | null): number | null => used === null || !Number.isFinite(used) ? null : Math.max(0, Math.min(100, 100 - used));
export function freshness(s: Snapshot, now: number): 'fresh' | 'stale' | 'unavailable' {
  if (s.observedAt === null) return 'unavailable';
  const maxAge = s.source === 'Claude Desktop usage cache' ? 1200 : 600;
  return s.state === 'ready' && now >= s.observedAt && now - s.observedAt < maxAge ? 'fresh' : 'stale';
}
export function windowState(s: Snapshot, w: UsageWindow, now: number) {
  if (w.resetsAt !== null && w.resetsAt <= now) return 'awaiting';
  if (w.usedPercent === null) return 'unavailable';
  return freshness(s, now);
}
export function countdown(reset: number | null, now: number, usedPercent: number | null = null): string {
  if (reset === null) return usedPercent === 0 ? 'Full allowance available' : 'Reset time unavailable';
  const minutes = Math.max(0, Math.ceil((reset - now) / 60));
  if (!minutes) return 'Awaiting update';
  const days = Math.floor(minutes / 1440), hours = Math.floor(minutes % 1440 / 60), mins = minutes % 60;
  return `Resets in ${days ? `${days}d ${hours}h` : hours ? `${hours}h ${mins}m` : `${mins}m`}`;
}
export function age(at: number | null, now: number): string {
  if (at === null) return 'No reading yet';
  const minutes = Math.max(0, Math.floor((now - at) / 60));
  if (minutes < 1) return 'Updated just now';
  if (minutes < 60) return `Updated ${minutes}m ago`;
  if (minutes < 1440) return `Updated ${Math.floor(minutes / 60)}h ago`;
  return `Updated ${Math.floor(minutes / 1440)}d ago`;
}
