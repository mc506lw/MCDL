// 「扫描并导入其它启动器实例」的前端门。
//
// 和 `api.ts` 一个套路：桌面应用里走 `invoke`，浏览器里（`pnpm dev`，没有 Tauri
// 注入）自动切到本文件下面的演示后端。两边函数签名、事件结构完全一致，
// `ImportPage.tsx` 不感知差别。
//
// 字段名照 `crates/mcdl-core/src/import.rs` 的 serde 输出写（snake_case）。

import { IS_TAURI } from './api';
import { loadSettings } from './settings';
import type { LogLevel } from './types';

// ---------------------------------------------------------------- 协议

/** 认出来的启动器家族，对应引擎 `import::Launcher`。 */
export type LauncherId =
  | 'pcl'
  | 'pcl_ce'
  | 'hmcl'
  | 'baka_xl'
  | 'prism'
  | 'multi_mc'
  | 'vanilla'
  | 'unknown';

export const LAUNCHER_LABEL: Record<LauncherId, string> = {
  pcl: 'PCL',
  pcl_ce: 'PCL CE',
  hmcl: 'HMCL',
  baka_xl: 'BakaXL',
  prism: 'Prism',
  multi_mc: 'MultiMC',
  vanilla: '官方启动器',
  unknown: '未知来源',
};

/** 可导入单元的类型，对应引擎 `import::ItemKind`。 */
export type ItemKind = 'game_dir' | 'prism_instance' | 'multi_mc_instance';

export const ITEM_KIND_LABEL: Record<ItemKind, string> = {
  game_dir: '游戏目录',
  prism_instance: 'Prism 实例',
  multi_mc_instance: 'MultiMC 实例',
};

/** `versions/<id>/` 里的一个版本。 */
export interface ImportVersion {
  id: string;
  /** release / snapshot / old_beta / old_alpha；实例的合成条目是 instance */
  kind: string;
  loader: string | null;
  /** 能不能按目录复制过去。实例的合成条目是 false（没有目录，要重装） */
  importable: boolean;
}

/** 扫到的一项，对应引擎 `import::ImportItem`。 */
export interface ImportItem {
  key: string;
  launchers: LauncherId[];
  kind: ItemKind;
  name: string;
  /** 要导入的根；实例是实例目录本身 */
  path: string;
  /** 真正的 .minecraft（实例是 path/minecraft） */
  game_dir: string;
  versions: ImportVersion[];
  has_mods: boolean;
  has_config: boolean;
  has_saves: boolean;
  modified: number;
  /** 找到它的证据，界面原样显示 */
  evidence: string[];
}

/** 一次探针：扫了哪里、找到没有、为什么。 */
export interface ScanProbe {
  launcher: LauncherId;
  path: string;
  found: boolean;
  note: string;
}

export interface ImportScan {
  items: ImportItem[];
  probes: ScanProbe[];
  scanned: number;
  duration_ms: number;
}

export interface DirSize {
  bytes: number;
  files: number;
}

/** 一次导入请求，对应引擎 `import::ImportRequest`。 */
export interface ImportRequest {
  source: string;
  target: string;
  version_ids: string[];
  include_mods: boolean;
  include_config: boolean;
  include_saves: boolean;
  /** 目标同名时覆盖。默认 false：已存在的就跳过 */
  overwrite: boolean;
  /** 同盘硬链代替复制。默认 false：复制更安全 */
  hardlink: boolean;
  /** 版本隔离：导入的 mods 进 `versions/<版本id>/mods/`（默认开）还是根 `mods/` */
  isolated?: boolean;
}

export interface ImportReport {
  target: string;
  versions: string[];
  skipped_versions: string[];
  files: number;
  bytes: number;
  skipped_files: number;
  links: number;
  failed: number;
  duration_ms: number;
}

export type ImportProgress =
  | { type: 'stage'; index: number; total: number; label: string }
  | {
      type: 'files';
      done: number;
      total: number;
      bytes_done: number;
      bytes_total: number;
      current: string;
    }
  | { type: 'log'; level: LogLevel; message: string };

/** 命令层在进度之外补的三种收尾状态。 */
export type ImportEvent =
  | { type: 'progress'; progress: ImportProgress }
  | { type: 'done'; report: ImportReport }
  | { type: 'failed'; message: string }
  | { type: 'cancelled' };

/** 默认的导入请求：版本 + mods + config，不覆盖、不硬链。 */
export function makeImportRequest(
  source: string,
  target: string,
  versionIds: string[],
): ImportRequest {
  return {
    source,
    target,
    version_ids: versionIds,
    include_mods: true,
    include_config: true,
    include_saves: false,
    overwrite: false,
    hardlink: false,
    // 版本隔离是全局设置：导入的 mods 跟着版本进 versions/<id>/mods/。
    isolated: loadSettings().isolated,
  };
}

// ---------------------------------------------------------------- 真后端

/** 通道要活到导入结束，放在模块作用域里。 */
let liveChannel: { onmessage: ((event: ImportEvent) => void) | null } | null = null;

async function core() {
  return import('@tauri-apps/api/core');
}

export const importApi = {
  /** 扫本机其它启动器。永远不 reject 成「找不到」——空列表就是没找到。 */
  async scan(): Promise<ImportScan> {
    if (!IS_TAURI) return scanMock();
    const { invoke } = await core();
    return invoke<ImportScan>('import_scan');
  },

  /** 懒加载体积：这一项「导入会搬多少」（版本目录 + mods/config/saves）。 */
  async measure(item: ImportItem): Promise<DirSize> {
    if (!IS_TAURI) return measureMock(item.path);
    const { invoke } = await core();
    return invoke<DirSize>('import_measure', { item });
  },

  /** 开始一次导入。立刻返回，进度与结果都从 `onEvent` 来。 */
  async start(req: ImportRequest, onEvent: (event: ImportEvent) => void): Promise<void> {
    if (!IS_TAURI) return startMock(req, onEvent);
    const { invoke, Channel } = await core();
    if (liveChannel) liveChannel.onmessage = null;
    const channel = new Channel<ImportEvent>();
    channel.onmessage = onEvent;
    liveChannel = channel;
    await invoke('import_start', { req, onEvent: channel });
  },

  /** 取消正在跑的导入。返回是否真的取消了。 */
  async cancel(): Promise<boolean> {
    if (!IS_TAURI) return cancelMock();
    const { invoke } = await core();
    return invoke<boolean>('import_cancel');
  },

  /** 导入结束后释放通道。 */
  releaseChannel() {
    liveChannel = null;
  },
};

// ---------------------------------------------------------------- 演示后端
//
// 浏览器里读不到磁盘，所以这里给一台「假机器」：PCL 的 .minecraft、一个 Prism
// 实例、一个 HMCL 目录，外加一份探针列表（说明为什么没找到 BakaXL / MultiMC）。
// 导入会真的在内存里记账：第二次导同一个版本会报「已存在」。

const MOCK_ROOT = 'C:\\Users\\Player\\AppData\\Roaming';

const MOCK_SCAN: ImportScan = {
  items: [
    {
      key: 'c:/users/player/appdata/roaming/.minecraft',
      launchers: ['pcl', 'vanilla'],
      kind: 'game_dir',
      name: '.minecraft',
      path: `${MOCK_ROOT}\\.minecraft`,
      game_dir: `${MOCK_ROOT}\\.minecraft`,
      versions: [
        { id: '1.21.1', kind: 'release', loader: null, importable: true },
        {
          id: 'fabric-loader-0.16.10-1.21.1',
          kind: 'release',
          loader: 'Fabric',
          importable: true,
        },
        { id: '1.20.1-forge-47.3.0', kind: 'release', loader: 'Forge', importable: true },
      ],
      has_mods: true,
      has_config: true,
      has_saves: true,
      modified: 1792000000,
      evidence: [
        '游戏目录里有 PCL\\Setup.ini',
        'launcher_profiles.json 里有官方启动器的档案',
      ],
    },
    {
      key: 'c:/users/player/appdata/roaming/prismlauncher/instances/1.20.1 fabric',
      launchers: ['prism'],
      kind: 'prism_instance',
      name: '1.20.1 Fabric',
      path: `${MOCK_ROOT}\\PrismLauncher\\instances\\1.20.1 Fabric`,
      game_dir: `${MOCK_ROOT}\\PrismLauncher\\instances\\1.20.1 Fabric\\minecraft`,
      versions: [
        { id: '1.20.1', kind: 'instance', loader: 'Fabric 0.15.11', importable: false },
      ],
      has_mods: true,
      has_config: true,
      has_saves: false,
      modified: 1791900000,
      evidence: [
        'Prism 的 instances 目录里的实例',
        'mmc-pack.json 声明：1.20.1 / Fabric 0.15.11',
      ],
    },
    {
      key: 'd:/mc/hmcl/.minecraft',
      launchers: ['hmcl'],
      kind: 'game_dir',
      name: '.minecraft',
      path: 'D:\\mc\\HMCL\\.minecraft',
      game_dir: 'D:\\mc\\HMCL\\.minecraft',
      versions: [
        { id: '1.19.2', kind: 'release', loader: null, importable: true },
        {
          id: 'quilt-loader-0.23.1-1.19.2',
          kind: 'release',
          loader: 'Quilt',
          importable: true,
        },
      ],
      has_mods: true,
      has_config: false,
      has_saves: false,
      modified: 1791800000,
      evidence: ['HMCL 的 hmcl.json 里记着这个目录'],
    },
  ],
  probes: [
    {
      launcher: 'pcl',
      path: `${MOCK_ROOT}\\PCL`,
      found: true,
      note: '找到 PCL 的配置目录',
    },
    {
      launcher: 'pcl',
      path: `${MOCK_ROOT}\\.minecraft\\PCL\\Setup.ini`,
      found: true,
      note: '读到 PCL 的 Setup.ini，里面记着 2 个目录',
    },
    {
      launcher: 'hmcl',
      path: `${MOCK_ROOT}\\.hmcl\\hmcl.json`,
      found: true,
      note: '读到 HMCL 的 hmcl.json，里面记着 1 个目录',
    },
    {
      launcher: 'baka_xl',
      path: `${MOCK_ROOT}\\BakaXL`,
      found: false,
      note: '没有这个目录',
    },
    {
      launcher: 'prism',
      path: `${MOCK_ROOT}\\PrismLauncher\\instances`,
      found: true,
      note: '找到 1 个实例',
    },
    {
      launcher: 'multi_mc',
      path: `${MOCK_ROOT}\\MultiMC\\instances`,
      found: false,
      note: '没有这个目录',
    },
  ],
  scanned: 7,
  duration_ms: 3,
};

function scanMock(): Promise<ImportScan> {
  return new Promise((resolve) =>
    window.setTimeout(() => resolve(structuredClone(MOCK_SCAN)), 420),
  );
}

/** 假体积：按路径算个稳定值，别每次刷新都变。 */
function measureMock(path: string): Promise<DirSize> {
  let hash = 0;
  for (let i = 0; i < path.length; i += 1) hash = (hash * 31 + path.charCodeAt(i)) % 100_000;
  const files = 120 + (hash % 3_400);
  const bytes = files * (18_000 + (hash % 90_000));
  return new Promise((resolve) => window.setTimeout(() => resolve({ bytes, files }), 260));
}

// 演示用的「目标目录里已经有什么」。导入会写进来，所以重复导入能演示「已存在」。
const imported = new Set<string>();
let stopImport: (() => void) | null = null;

function cancelMock(): boolean {
  if (!stopImport) return false;
  stopImport();
  return true;
}

function startMock(
  req: ImportRequest,
  onEvent: (event: ImportEvent) => void,
): Promise<void> {
  cancelMock();

  const item = MOCK_SCAN.items.find((i) => i.path === req.source);
  const source = item?.game_dir ?? req.source;
  const importable = req.version_ids.filter((id) =>
    (item?.versions ?? []).some((v) => v.id === id && v.importable),
  );

  // 规划：没被「已存在」挡住的版本各一项，再加 mods / config / saves
  const skippedVersions = importable.filter((id) => imported.has(`${req.target}|${id}`));
  const stages: string[] = importable
    .filter((id) => !skippedVersions.includes(id))
    .map((id) => `版本 ${id}`);
  if (req.include_mods && item?.has_mods) stages.push('mods');
  if (req.include_config && item?.has_config) stages.push('config');
  if (req.include_saves && item?.has_saves) stages.push('saves');

  const startedAt = performance.now();
  let index = 0;
  let stageStarted = startedAt;
  let done = 0;
  let total = 0;
  let bytesDone = 0;
  let bytesTotal = 0;
  let finished = false;

  const emit = (progress: ImportProgress) => onEvent({ type: 'progress', progress });
  const stageMs = 900;

  if (skippedVersions.length > 0) {
    emit({
      type: 'log',
      level: 'warn',
      message: `目标里已经有版本 ${skippedVersions.join('、')}，跳过（不覆盖）`,
    });
  }
  emit({ type: 'log', level: 'info', message: `${source} → ${req.target}` });

  const finish = () => {
    if (finished) return;
    finished = true;
    window.clearInterval(timer);
    stopImport = null;
    importable.forEach((id) => imported.add(`${req.target}|${id}`));
    const files = Math.round(240 + (total % 900));
    onEvent({
      type: 'done',
      report: {
        target: req.target,
        versions: importable.filter((id) => !skippedVersions.includes(id)),
        skipped_versions: skippedVersions,
        files,
        bytes: files * 42_000,
        skipped_files: 0,
        links: 0,
        failed: 0,
        duration_ms: Math.round(performance.now() - startedAt),
      },
    });
  };

  const timer = window.setInterval(() => {
    const now = performance.now();
    const label = stages[index];
    if (!label) {
      finish();
      return;
    }

    if (done === 0) {
      total = 260 + ((index * 173) % 800);
      bytesTotal = total * 42_000;
      emit({ type: 'stage', index: index + 1, total: stages.length, label });
    }

    const raw = Math.min(1, (now - stageStarted) / stageMs);
    const eased = 1 - Math.pow(1 - raw, 2);
    done = Math.round(eased * total);
    bytesDone = Math.round(eased * bytesTotal);
    emit({
      type: 'files',
      done,
      total,
      bytes_done: bytesDone,
      bytes_total: bytesTotal,
      current: `${req.target}\\${label}`,
    });

    if (raw >= 1) {
      index += 1;
      stageStarted = now;
      done = 0;
      if (index >= stages.length) finish();
    }
  }, 90);

  stopImport = () => {
    finished = true;
    window.clearInterval(timer);
    stopImport = null;
    onEvent({ type: 'cancelled' });
  };

  return Promise.resolve();
}
