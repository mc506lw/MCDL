// 浏览器里跑的演示后端。
//
// 只在 `window.__TAURI_INTERNALS__` 不存在时启用：`pnpm dev` 打开就能看到完整界面，
// 不用等 Rust 那边编译。事件结构和 `mcdl_core::Progress` 完全一致（包括
// `Stage.index` 1 起、`total` 是本次计划的阶段数、`label` 直接显示），
// 所以接真后端时界面代码一行都不用改。
//
// 这里的时间、文件数、字节数都是演示值，界面会明说「演示数据」。

import type {
  InstallEvent,
  InstallReport,
  InstallRequest,
  InstalledVersion,
  JavaRuntime,
  Progress,
  StageKind,
  VersionManifest,
} from './types';

// ---------------------------------------------------------------- 版本清单

/** 真实存在过的正式版与它们的发布日（演示用，只保留常用版本）。 */
const VERSIONS: ReadonlyArray<readonly [string, string]> = [
  ['1.21.8', '2025-07-17T00:00:00Z'],
  ['1.21.5', '2025-03-25T00:00:00Z'],
  ['1.21.4', '2024-12-03T00:00:00Z'],
  ['1.21.1', '2024-08-08T00:00:00Z'],
  ['1.21', '2024-06-13T00:00:00Z'],
  ['1.20.6', '2024-04-29T00:00:00Z'],
  ['1.20.4', '2023-12-07T00:00:00Z'],
  ['1.20.1', '2023-06-12T00:00:00Z'],
  ['1.19.4', '2023-03-14T00:00:00Z'],
  ['1.18.2', '2022-02-28T00:00:00Z'],
  ['1.17.1', '2021-07-06T00:00:00Z'],
  ['1.16.5', '2021-01-14T00:00:00Z'],
  ['1.12.2', '2017-09-18T00:00:00Z'],
  ['1.8.9', '2015-12-09T00:00:00Z'],
];

export const MOCK_MANIFEST: VersionManifest = {
  latest: { release: VERSIONS[0][0], snapshot: VERSIONS[0][0] },
  versions: VERSIONS.map(([id, releaseTime]) => ({
    id,
    type: 'release',
    url: '',
    sha1: null,
    releaseTime,
  })),
};

const JAVA_DEMO: JavaRuntime[] = [
  {
    path: 'C:\\Program Files\\Eclipse Adoptium\\jdk-21.0.5.11-hotspot\\bin\\javaw.exe',
    major: 21,
    version: '21.0.5+11-LTS',
    vendor: 'Eclipse Adoptium',
    arch: 'x86_64',
  },
  {
    path: 'C:\\Program Files\\Eclipse Adoptium\\jdk-17.0.13.11-hotspot\\bin\\javaw.exe',
    major: 17,
    version: '17.0.13+11-LTS',
    vendor: 'Eclipse Adoptium',
    arch: 'x86_64',
  },
];

/** 浏览器读不到系统路径，给一个占位值；界面上会写明这是占位。 */
export const MOCK_GAME_DIR = 'C:\\Users\\Player\\AppData\\Roaming\\.minecraft';

const LOADER_VERSION: Record<string, string> = {
  fabric: '0.16.10',
  quilt: '0.27.0',
  forge: '51.0.33',
  neoforge: '21.1.72',
};

// ---------------------------------------------------------------- 阶段计划

interface StagePlan {
  kind: StageKind;
  label: string;
  files: number;
  bytes: number;
  ms: number;
  note?: string;
  warn?: { at: number; message: string };
}

function buildPlan(req: InstallRequest): StagePlan[] {
  const loader = req.loader ? req.loader.kind : null;
  const plan: StagePlan[] = [
    {
      kind: 'meta',
      label: '读取版本清单',
      files: 0,
      bytes: 0,
      ms: 900,
      note: `版本 ${req.mc_version}（release）`,
    },
    {
      kind: 'libraries',
      label: '下载依赖库',
      files: 128,
      bytes: 62_400_000,
      ms: 2400,
      note: '依赖库 128 个文件，共 59.5 MB',
    },
  ];

  if (!req.skip_assets) {
    plan.push({
      kind: 'assets',
      label: '下载资源文件',
      files: 3021,
      bytes: 679_000_000,
      ms: 6200,
      note: '资源文件 3,021 个文件，共 647.6 MB',
      warn: { at: 0.55, message: 'assets 里有 2 个文件校验失败，已重新下载' },
    });
  }

  plan.push({
    kind: 'client',
    label: '下载游戏本体',
    files: 1,
    bytes: 26_600_000,
    ms: 900,
    note: `游戏本体 ${req.mc_version}.jar，25.4 MB`,
  });

  if (loader) {
    const version = req.loader?.loader_version || LOADER_VERSION[loader];
    plan.push({
      kind: 'loader',
      label: '安装模组加载器',
      files: 1,
      bytes: 2_400_000,
      ms: 2000,
      note: `调用官方 installer：${loader} ${version} / ${req.mc_version}`,
    });
  }

  return plan;
}

function installedVersion(req: InstallRequest): string {
  const loader = req.loader;
  if (!loader) return req.mc_version;
  const version = loader.loader_version || LOADER_VERSION[loader.kind];
  return loader.kind === 'fabric' || loader.kind === 'quilt'
    ? `${loader.kind}-loader-${version}-${req.mc_version}`
    : `${req.mc_version}-${loader.kind}-${version}`;
}

// ---------------------------------------------------------------- 模拟安装

let stopCurrent: (() => void) | null = null;

/** 取消正在演示的安装。真后端里这一步是 abort 掉引擎的 future。 */
export function cancelMock(): boolean {
  if (!stopCurrent) return false;
  stopCurrent();
  return true;
}

export function installMock(
  req: InstallRequest,
  onEvent: (event: InstallEvent) => void,
): Promise<void> {
  cancelMock();

  const plan = buildPlan(req);
  const total = plan.length;
  const startedAt = performance.now();

  let stageIndex = 0;
  let stageStartedAt = startedAt;
  let announced = -1;
  let warned = false;
  let totals = { files: 0, bytes: 0 };
  let finished = false;

  const emit = (progress: Progress) => onEvent({ type: 'progress', progress });

  const finish = () => {
    if (finished) return;
    finished = true;
    window.clearInterval(timer);
    stopCurrent = null;
    const report: InstallReport = {
      version_id: req.mc_version,
      game_dir: req.game_dir,
      files: totals.files,
      bytes: totals.bytes,
      duration_ms: Math.round(performance.now() - startedAt),
      java: req.java || JAVA_DEMO[0].path,
      installed_version: installedVersion(req),
    };
    emit({
      type: 'log',
      level: 'info',
      message: `完成：新增 ${report.files} 个文件，共 ${(report.bytes / 1_048_576).toFixed(1)} MB，用时 ${(report.duration_ms / 1000).toFixed(1)}s`,
    });
    onEvent({ type: 'done', report });
  };

  const timer = window.setInterval(() => {
    const stage = plan[stageIndex];
    if (!stage) {
      finish();
      return;
    }
    const now = performance.now();

    if (announced !== stageIndex) {
      announced = stageIndex;
      emit({
        type: 'stage',
        kind: stage.kind,
        index: stageIndex + 1,
        total,
        label: stage.label,
      });
      if (stage.note) emit({ type: 'log', level: 'info', message: stage.note });
    }

    // 前段快、后段慢：真实下载也是这样，大文件收尾时速度会掉。
    const raw = Math.min(1, (now - stageStartedAt) / stage.ms);
    const eased = 1 - Math.pow(1 - raw, 2.2);

    if (stage.files > 0) {
      emit({
        type: 'files',
        kind: stage.kind,
        done: Math.round(eased * stage.files),
        total: stage.files,
        bytes_done: Math.round(eased * stage.bytes),
        bytes_total: stage.bytes,
      });
    }

    if (stage.warn && !warned && eased >= stage.warn.at) {
      warned = true;
      emit({ type: 'log', level: 'warn', message: stage.warn.message });
    }

    if (raw >= 1) {
      totals = { files: totals.files + stage.files, bytes: totals.bytes + stage.bytes };
      stageIndex += 1;
      stageStartedAt = now;
      warned = false;
      if (stageIndex >= plan.length) finish();
    }
  }, 90);

  stopCurrent = () => {
    finished = true;
    window.clearInterval(timer);
    stopCurrent = null;
    onEvent({ type: 'cancelled' });
  };

  return Promise.resolve();
}

// ---------------------------------------------------------------- 其它命令

/** 加载器版本列表的演示数据：和引擎一样，最多 20 条、从新到旧。 */
const LOADER_VERSIONS: Record<string, string[]> = {
  fabric: [
    '0.16.10', '0.16.9', '0.16.8', '0.16.7', '0.16.5', '0.16.4', '0.16.3', '0.16.2',
    '0.16.0', '0.15.11', '0.15.10', '0.15.9', '0.15.7', '0.15.6', '0.15.4', '0.15.3',
    '0.15.2', '0.15.0', '0.14.25', '0.14.24',
  ],
  quilt: [
    '0.27.0', '0.26.4', '0.26.3', '0.26.2', '0.26.1', '0.26.0', '0.25.4', '0.25.3',
    '0.25.1', '0.25.0', '0.24.0', '0.23.4', '0.23.3', '0.23.2', '0.23.1', '0.22.0',
    '0.21.2', '0.21.1', '0.21.0', '0.20.0',
  ],
  forge: [
    '51.0.33', '51.0.32', '51.0.31', '51.0.30', '51.0.29', '51.0.27', '51.0.26',
    '51.0.25', '51.0.23', '51.0.22', '51.0.21', '51.0.20', '51.0.19', '51.0.17',
    '51.0.16', '51.0.14', '51.0.13', '51.0.12', '51.0.11', '51.0.10',
  ],
  neoforge: [
    '21.1.72', '21.1.71', '21.1.70', '21.1.69', '21.1.68', '21.1.66', '21.1.65',
    '21.1.64', '21.1.63', '21.1.61', '21.1.60', '21.1.59', '21.1.57', '21.1.56',
    '21.1.55', '21.1.54', '21.1.52', '21.1.51', '21.1.50', '21.1.49',
  ],
};

/** 引擎侧限 20 条、从新到旧；这里也一样，别让界面做无限滚动。 */
export function listLoadersMock(kind: string): Promise<string[]> {
  const list = (LOADER_VERSIONS[kind] ?? []).slice(0, 20);
  return new Promise((resolve) => window.setTimeout(() => resolve(list), 450));
}

/** 演示环境里没有下载过运行时。 */
export function listRuntimesMock(): Promise<string[]> {
  return Promise.resolve([]);
}

/** 假装过一次网络：让界面的「正在拉清单」状态是真的出现过。 */
export function listVersionsMock(): Promise<VersionManifest> {
  return new Promise((resolve) => window.setTimeout(() => resolve(MOCK_MANIFEST), 420));
}

export function detectJavaMock(): Promise<JavaRuntime[]> {
  return new Promise((resolve) => window.setTimeout(() => resolve(JAVA_DEMO), 260));
}

export function defaultGameDirMock(): Promise<string> {
  return Promise.resolve(MOCK_GAME_DIR);
}

export function openGameDirMock(): Promise<void> {
  return Promise.reject(new Error('浏览器里打不开文件管理器，装成桌面应用后可用。'));
}

/** 演示用的已装版本。接真后端时这些数字都来自磁盘扫描。 */
const INSTALLED_DEMO: InstalledVersion[] = [
  {
    id: '1.21.8',
    kind: 'release',
    loader: null,
    main_class: 'net.minecraft.client.main.Main',
    bytes: 612_344_512,
    files: 4188,
    modified: 1792000000,
  },
  {
    id: 'fabric-loader-0.19.5-1.21.1',
    kind: 'release',
    loader: 'Fabric',
    main_class: 'net.fabricmc.loader.impl.launch.knot.KnotClient',
    bytes: 96_141_312,
    files: 96,
    modified: 1791900000,
  },
  {
    id: '1.20.4-forge-49.2.13',
    kind: 'release',
    loader: 'Forge',
    main_class: 'net.minecraftforge.bootstrap.ForgeBootstrap',
    bytes: 188_743_680,
    files: 141,
    modified: 1791800000,
  },
];

export function listInstalledMock(_gameDir: string): Promise<InstalledVersion[]> {
  return new Promise((resolve) =>
    window.setTimeout(() => resolve([...INSTALLED_DEMO]), 240),
  );
}

export function removeVersionMock(id: string): Promise<void> {
  return new Promise((resolve, reject) =>
    window.setTimeout(() => {
      const i = INSTALLED_DEMO.findIndex((v) => v.id === id);
      if (i < 0) reject(new Error(`没有这个版本：${id}`));
      else {
        INSTALLED_DEMO.splice(i, 1);
        resolve();
      }
    }, 220),
  );
}
