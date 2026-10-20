// 界面上的数字怎么排：字节、速度、时长、计数。
//
// 全部用宽体等宽数字（tabular-nums 由 endfield.css 的 .ef-field / .ef-figs 负责），
// 单位用拉丁缩写，和数字之间留一个空格。

const UNITS = ['B', 'KB', 'MB', 'GB', 'TB'] as const;

/** 字节拆成「数值 + 单位」，方便把数字做大、单位做小。 */
export function splitBytes(bytes: number): { value: string; unit: string } {
  if (!Number.isFinite(bytes) || bytes <= 0) return { value: '0', unit: 'B' };
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < UNITS.length - 1) {
    value /= 1024;
    i += 1;
  }
  const digits = i === 0 ? 0 : value >= 100 ? 0 : 1;
  return { value: value.toFixed(digits), unit: UNITS[i] };
}

export function formatBytes(bytes: number): string {
  const { value, unit } = splitBytes(bytes);
  return `${value} ${unit}`;
}

/** 下载速度：字节/秒 → `12.4 MB/s`。 */
export function formatSpeed(bytesPerSecond: number): { value: string; unit: string } {
  const { value, unit } = splitBytes(bytesPerSecond);
  return { value, unit: `${unit}/s` };
}

/** 一段已经过去的时长：`12.4 s` / `01:24`。 */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '—';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  return clock(seconds);
}

/** 剩余时间：秒 → `02:41`；不到一分钟就直接给秒数，别显示成 `00:00`。 */
export function formatEta(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '—';
  if (seconds < 1) return '< 1 s';
  if (seconds < 60) return `${Math.round(seconds)} s`;
  if (seconds > 99 * 60) return '> 99 分';
  return clock(seconds);
}

function clock(totalSeconds: number): string {
  const s = Math.round(totalSeconds);
  const m = Math.floor(s / 60);
  return `${String(m).padStart(2, '0')}:${String(s % 60).padStart(2, '0')}`;
}

/** 计数带千位分隔：`3,150`。 */
export function formatCount(n: number): string {
  if (!Number.isFinite(n)) return '0';
  return Math.round(n).toLocaleString('en-US');
}

/** 读数补零：`063%`。 */
export function padPercent(ratio: number): string {
  const clamped = Math.max(0, Math.min(1, Number.isFinite(ratio) ? ratio : 0));
  return `${String(Math.floor(clamped * 100)).padStart(3, '0')}%`;
}

/** 阶段编号补零：`03 / 05`。 */
export function pad2(n: number): string {
  return String(Math.max(0, n)).padStart(2, '0');
}

/** 一天里的时间：`12:04:31`。 */
export function formatClock(at: number): string {
  const d = new Date(at);
  return [d.getHours(), d.getMinutes(), d.getSeconds()]
    .map((n) => String(n).padStart(2, '0'))
    .join(':');
}

/** 版本清单里的日期：`2025.07.17`。 */
export function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return [
    d.getFullYear(),
    String(d.getMonth() + 1).padStart(2, '0'),
    String(d.getDate()).padStart(2, '0'),
  ].join('.');
}
