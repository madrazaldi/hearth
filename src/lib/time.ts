import type { Settings } from './types';

export function formatClock(ms: number, s: Pick<Settings, 'clock' | 'showSeconds'>): string {
  const d = new Date(ms);
  let h = d.getHours();
  const m = String(d.getMinutes()).padStart(2, '0');
  const sec = s.showSeconds ? ':' + String(d.getSeconds()).padStart(2, '0') : '';
  if (s.clock === '12h') {
    const ap = h >= 12 ? 'p' : 'a';
    h = h % 12 || 12;
    return `${h}:${m}${sec}${ap}`;
  }
  return `${String(h).padStart(2, '0')}:${m}${sec}`;
}

export function dayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`;
}

export function formatDay(ms: number): string {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date(Date.now() - 86400000);
  if (dayKey(ms) === dayKey(today.getTime())) return 'Today';
  if (dayKey(ms) === dayKey(yesterday.getTime())) return 'Yesterday';
  const sameYear = d.getFullYear() === today.getFullYear();
  return d.toLocaleDateString(undefined, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
}

export function formatFull(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'medium' });
}

export function formatDuration(secs: number): string {
  const units: [string, number][] = [['d', 86400], ['h', 3600], ['m', 60], ['s', 1]];
  const parts: string[] = [];
  for (const [u, n] of units) {
    if (secs >= n) {
      parts.push(Math.floor(secs / n) + u);
      secs %= n;
    }
    if (parts.length === 2) break;
  }
  return parts.join(' ') || '0s';
}
