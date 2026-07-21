// Small display helpers shared across panels.

/** Unix-nanosecond timestamp → local time string. */
export function fmtTime(tsNanos) {
  if (!tsNanos) return '—';
  const ms = Number(tsNanos) / 1e6;
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return String(tsNanos);
  return d.toLocaleString();
}

/** Pull a short request summary (the query/content) out of a record. */
export function reqSummary(record) {
  const r = record?.request;
  if (!r) return '';
  return r.query ?? r.content ?? JSON.stringify(r);
}

export function truncate(s, n = 140) {
  if (!s) return '';
  return s.length > n ? s.slice(0, n) + '…' : s;
}

/** Human-readable byte size. */
export function fmtBytes(n) {
  n = Number(n) || 0;
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}
