// 本机设置。浏览器里存在 localStorage；接上 Tauri 之后改成写配置文件，
// 调用点不用动。

import { api } from './api';

export interface Settings {
  /** 并发下载数，引擎默认 16 */
  concurrency: number;
  /** 当前选中的游戏目录（.minecraft 所在目录） */
  gameDir: string;
  /** 用过/收藏的游戏目录，安装页可以从这里切 */
  dirs: string[];
  /** 指定 Java；留空则自动探测 */
  javaPath: string;
  /** 只装能启动的最小集合 */
  skipAssets: boolean;
}

const KEY = 'mcdl.settings';

export const DEFAULT_SETTINGS: Settings = {
  concurrency: 16,
  gameDir: '',
  dirs: [],
  javaPath: '',
  skipAssets: false,
};

export function loadSettings(): Settings {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULT_SETTINGS };
    const parsed = JSON.parse(raw) as Partial<Settings>;
    return {
      concurrency: clampConcurrency(parsed.concurrency),
      gameDir: typeof parsed.gameDir === 'string' ? parsed.gameDir : '',
      dirs: Array.isArray(parsed.dirs)
        ? parsed.dirs.filter((d): d is string => typeof d === 'string' && d.length > 0)
        : [],
      javaPath: typeof parsed.javaPath === 'string' ? parsed.javaPath : '',
      skipAssets: Boolean(parsed.skipAssets),
    };
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function saveSettings(settings: Settings): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(settings));
  } catch {
    // 存不进去也不该挡住安装，界面会照常用这一次的值
  }
}

export function clearSettings(): void {
  try {
    window.localStorage.removeItem(KEY);
  } catch {
    // 同上
  }
}

export function clampConcurrency(value: unknown): number {
  const n = Number(value);
  if (!Number.isFinite(n)) return DEFAULT_SETTINGS.concurrency;
  return Math.max(1, Math.min(64, Math.round(n)));
}

/** 第一次打开时把系统默认目录填进设置，省得用户手打。 */
export async function resolveDefaultGameDir(): Promise<string> {
  const fromSettings = loadSettings().gameDir;
  if (fromSettings) return fromSettings;
  try {
    return await api.defaultGameDir();
  } catch {
    return '';
  }
}

/** 把一个目录记进列表（去重，最多留 8 个），并设为当前目录。 */
export function rememberDir(settings: Settings, dir: string): Settings {
  const trimmed = dir.trim();
  if (!trimmed) return settings;
  const dirs = [trimmed, ...settings.dirs.filter((d) => d !== trimmed)].slice(0, 8);
  return { ...settings, gameDir: trimmed, dirs };
}

/** 从列表里移除一个目录；移的正好是当前目录时，退回列表里的第一个。 */
export function forgetDir(settings: Settings, dir: string): Settings {
  const dirs = settings.dirs.filter((d) => d !== dir);
  const gameDir = settings.gameDir === dir ? (dirs[0] ?? '') : settings.gameDir;
  return { ...settings, dirs, gameDir };
}
