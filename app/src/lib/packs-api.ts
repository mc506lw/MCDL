// 整合包（Modrinth / CurseForge）的前端门。
//
// 和 `api.ts` 一样：桌面应用里走 `invoke`，浏览器里（`pnpm dev`，没有 Tauri 注入）
// 自动切到本文件里的演示后端，界面代码不感知差别。
//
// 这里**不碰 `api.ts`**：整合包是独立的一块功能，页面也只依赖这个文件。
//
// CurseForge 的 API key 只存在本机 `localStorage`，不上传、不内置：
// 没有 key 时界面给一条「去申请」的提示，Modrinth 那边完全不需要 key。

import type { InstallReport, LoaderKind, LogLevel, Progress, Source } from './types';

// ---------------------------------------------------------------- 协议类型
//
// 与 `crates/mcdl-core/src/pack/` 一一对应。字段名照 serde 的输出写。

/** 整合包来自哪个平台。 */
export type PackSource = 'modrinth' | 'curseforge';

/** 排序方式。 */
export type PackSort = 'relevance' | 'downloads' | 'updated' | 'newest';

export interface PackSearchQuery {
  query: string;
  source: PackSource;
  /** 每页条数，引擎侧夹在 1..=50 */
  limit?: number;
  offset?: number;
  mc_version?: string | null;
  loader?: LoaderKind | null;
  sort?: PackSort;
  /** 仅 CurseForge 需要 */
  api_key?: string | null;
}

export interface PackHit {
  source: PackSource;
  /** Modrinth：project id；CurseForge：数字 id */
  id: string;
  slug: string;
  title: string;
  description: string;
  author: string | null;
  icon_url: string | null;
  downloads: number;
  followers: number;
  categories: string[];
  game_versions: string[];
  /** Modrinth 搜索接口不给独立的 loaders，要从 categories 里认 */
  loaders: string[];
  date_modified: string;
  latest_version: string | null;
  license: string | null;
}

export interface PackSearchResult {
  hits: PackHit[];
  total: number;
  offset: number;
  limit: number;
}

export interface PackProject {
  source: PackSource;
  id: string;
  slug: string;
  title: string;
  description: string;
  /** Markdown 正文；界面只当纯文本截断显示，不渲染 */
  body: string;
  author: string | null;
  icon_url: string | null;
  downloads: number;
  followers: number;
  categories: string[];
  game_versions: string[];
  loaders: string[];
  license: string | null;
  source_url: string | null;
  issues_url: string | null;
  wiki_url: string | null;
  published: string;
  updated: string;
}

export interface PackFile {
  filename: string;
  /** 空字符串表示平台不给下载地址（CurseForge 的分发限制） */
  url: string;
  size: number;
  sha1: string | null;
  sha512: string | null;
  primary: boolean;
}

export interface PackDep {
  project_id: string | null;
  version_id: string | null;
  file_name: string | null;
  dependency_type: string;
}

export interface PackVersion {
  id: string;
  name: string;
  version_number: string;
  /** release / beta / alpha */
  version_type: string;
  game_versions: string[];
  loaders: string[];
  date_published: string;
  downloads: number;
  files: PackFile[];
  dependencies: PackDep[];
}

/** 一次整合包安装请求。 */
export interface PackInstallRequest {
  game_dir: string;
  source: PackSource;
  project_id: string;
  version_id?: string | null;
  /** 界面已经知道的名字，省引擎一次网络请求 */
  project_title?: string | null;
  api_key?: string | null;
  /** Minecraft 文件走官方还是 BMCLAPI */
  download_source: Source;
  concurrency: number;
  java?: string | null;
  skip_assets: boolean;
}

export interface PackInstallReport {
  source: PackSource;
  pack_id: string;
  pack_name: string;
  pack_version: string;
  mc_version: string;
  /** 例如 `Fabric 0.16.9`；原版整合包是 null */
  loader: string | null;
  installed_version: string;
  game_dir: string;
  pack_files: number;
  pack_bytes: number;
  skipped: number;
  overrides: number;
  warnings: string[];
  duration_ms: number;
  /** 引擎那次「原版 + 加载器」安装的报告 */
  base: InstallReport;
}

/**
 * 整合包安装的进度。
 *
 * - `stage` / `files` / `log`：整合包自己的阶段（共 3 步）
 * - `engine`：装游戏本体和加载器时，引擎的进度原样转发
 */
export type PackProgress =
  | { type: 'stage'; index: number; total: number; label: string }
  | { type: 'files'; done: number; total: number; bytes_done: number; bytes_total: number }
  | { type: 'log'; level: LogLevel; message: string }
  | { type: 'engine'; progress: Progress };

/** 命令层在进度之外补的收尾状态。 */
export type PackEvent =
  | { type: 'progress'; progress: PackProgress }
  | { type: 'done'; report: PackInstallReport }
  | { type: 'failed'; message: string }
  | { type: 'cancelled' };

// ---------------------------------------------------------------- CurseForge key

/** key 申请地址。界面上直接给链接，仓库里不存任何 key。 */
export const CURSEFORGE_KEY_URL = 'https://console.curseforge.com/';

const CF_KEY = 'mcdl.curseforge.key';

/** 读本机存的 CurseForge key；没有就是空串。 */
export function loadCurseforgeKey(): string {
  try {
    return window.localStorage.getItem(CF_KEY) ?? '';
  } catch {
    return '';
  }
}

/** 存 key。传空串等于删掉。 */
export function saveCurseforgeKey(key: string): void {
  try {
    const trimmed = key.trim();
    if (trimmed) window.localStorage.setItem(CF_KEY, trimmed);
    else window.localStorage.removeItem(CF_KEY);
  } catch {
    // 存不进去不该挡住搜索：这一次用输入框里的值就行
  }
}

/** 只用于界面显示：`abcd…wxyz`，别把整串 key 摆在屏幕上。 */
export function maskKey(key: string): string {
  const trimmed = key.trim();
  if (trimmed.length <= 8) return trimmed ? '已保存' : '';
  return `${trimmed.slice(0, 4)}…${trimmed.slice(-4)}`;
}

// ---------------------------------------------------------------- 环境

/** 是否跑在 Tauri 窗口里。与 `api.ts` 用的是同一个判据。 */
export const IS_TAURI =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 通道要活到安装结束，放在模块作用域里。 */
let liveChannel: { onmessage: ((event: PackEvent) => void) | null } | null = null;

async function core() {
  return import('@tauri-apps/api/core');
}

// ---------------------------------------------------------------- 真接口

export const packsApi = {
  /** 搜整合包。CurseForge 需要 `api_key`。 */
  async search(q: PackSearchQuery): Promise<PackSearchResult> {
    if (!IS_TAURI) return searchMock(q);
    const { invoke } = await core();
    return invoke<PackSearchResult>('pack_search', { query: q });
  },

  /** 整合包详情（简介、正文、许可证）。 */
  async project(source: PackSource, id: string, apiKey?: string | null): Promise<PackProject> {
    if (!IS_TAURI) return projectMock(source, id);
    const { invoke } = await core();
    return invoke<PackProject>('pack_project', { source, id, apiKey: apiKey ?? null });
  },

  /** 某个整合包的版本，从新到旧。 */
  async versions(
    source: PackSource,
    id: string,
    apiKey?: string | null,
  ): Promise<PackVersion[]> {
    if (!IS_TAURI) return versionsMock(source, id);
    const { invoke } = await core();
    return invoke<PackVersion[]>('pack_versions', { source, id, apiKey: apiKey ?? null });
  },

  /** 开始装。立刻返回，进度和结果都走 `onEvent`。 */
  async install(
    req: PackInstallRequest,
    onEvent: (event: PackEvent) => void,
  ): Promise<void> {
    if (!IS_TAURI) return installMock(req, onEvent);
    const { invoke, Channel } = await core();
    if (liveChannel) liveChannel.onmessage = null;
    const channel = new Channel<PackEvent>();
    channel.onmessage = onEvent;
    liveChannel = channel;
    await invoke('pack_install', { req, onEvent: channel });
  },

  /** 取消正在跑的整合包安装。返回是否真的取消了。 */
  async cancel(): Promise<boolean> {
    if (!IS_TAURI) return cancelMock();
    const { invoke } = await core();
    return invoke<boolean>('pack_cancel');
  },

  /** 装完之后释放通道。 */
  releaseChannel() {
    liveChannel = null;
  },
};

// ================================================================ 演示后端
//
// 只在浏览器里生效。数字都是演示值，界面上会写明「演示数据」。
// 结构和真接口完全一致，所以接上 Tauri 之后页面一行都不用改。

interface MockPack {
  hit: Omit<PackHit, 'source' | 'loaders'>;
  loaders: LoaderKind[];
  versions: Array<{
    id: string;
    version_number: string;
    version_type: string;
    game_versions: string[];
    loaders: LoaderKind[];
    date_published: string;
    downloads: number;
    /** 演示用的模组文件数 / 字节数 */
    mods: number;
    mod_bytes: number;
  }>;
}

function hit(
  id: string,
  slug: string,
  title: string,
  description: string,
  author: string,
  downloads: number,
  followers: number,
  categories: string[],
  gameVersions: string[],
  dateModified: string,
): Omit<PackHit, 'source' | 'loaders'> {
  return {
    id,
    slug,
    title,
    description,
    author,
    icon_url: null,
    downloads,
    followers,
    categories,
    game_versions: gameVersions,
    date_modified: dateModified,
    latest_version: null,
    license: null,
  };
}

const MOCK_PACKS: MockPack[] = [
  {
    hit: hit(
      '1KVo5zza',
      'fabulously-optimized',
      'Fabulously Optimized',
      '以性能与画面为核心的小型整合包：把 OptiFine 的功能用现代模组重新实现一遍。',
      'robotkoer',
      19_068_662,
      4211,
      ['fabric', 'optimization', 'lightweight'],
      ['1.21.8', '1.21.1', '1.20.4', '1.20.1', '1.19.4'],
      '2026-10-18T09:12:00Z',
    ),
    loaders: ['fabric'],
    versions: [
      {
        id: 'fo-15',
        version_number: '15.0.0',
        version_type: 'release',
        game_versions: ['1.21.8'],
        loaders: ['fabric'],
        date_published: '2026-10-18T09:12:00Z',
        downloads: 131_004,
        mods: 128,
        mod_bytes: 96_400_000,
      },
      {
        id: 'fo-14',
        version_number: '14.2.1',
        version_type: 'release',
        game_versions: ['1.21.1'],
        loaders: ['fabric'],
        date_published: '2026-06-02T11:00:00Z',
        downloads: 402_991,
        mods: 121,
        mod_bytes: 91_200_000,
      },
      {
        id: 'fo-13',
        version_number: '13.0.2',
        version_type: 'beta',
        game_versions: ['1.20.4'],
        loaders: ['fabric'],
        date_published: '2026-01-11T08:20:00Z',
        downloads: 88_213,
        mods: 110,
        mod_bytes: 84_900_000,
      },
    ],
  },
  {
    hit: hit(
      '4Z9TE0js',
      'adrenaline',
      'Adrenaline',
      '极限优化整合包：把能榨的性能全榨出来，默认配置就很轻。',
      'Someone',
      1_284_991,
      612,
      ['fabric', 'optimization'],
      ['1.21.1', '1.20.4', '1.20.1'],
      '2026-09-30T14:00:00Z',
    ),
    loaders: ['fabric'],
    versions: [
      {
        id: 'adr-9',
        version_number: '9.1',
        version_type: 'release',
        game_versions: ['1.21.1'],
        loaders: ['fabric'],
        date_published: '2026-09-30T14:00:00Z',
        downloads: 40_112,
        mods: 74,
        mod_bytes: 51_300_000,
      },
      {
        id: 'adr-8',
        version_number: '8.4',
        version_type: 'release',
        game_versions: ['1.20.4'],
        loaders: ['fabric'],
        date_published: '2026-04-20T10:00:00Z',
        downloads: 22_001,
        mods: 70,
        mod_bytes: 48_700_000,
      },
    ],
  },
  {
    hit: hit(
      'cobblemon',
      'cobblemon-official',
      'Cobblemon Official Modpack',
      '把宝可梦带进 Minecraft：捕捉、对战、进化，配好一整套资源包。',
      'Cobblemon',
      3_912_440,
      1820,
      ['fabric', 'adventure', 'mobs'],
      ['1.21.1', '1.20.1'],
      '2026-08-14T16:40:00Z',
    ),
    loaders: ['fabric'],
    versions: [
      {
        id: 'cob-1',
        version_number: '1.6.1',
        version_type: 'release',
        game_versions: ['1.21.1'],
        loaders: ['fabric'],
        date_published: '2026-08-14T16:40:00Z',
        downloads: 190_233,
        mods: 42,
        mod_bytes: 214_800_000,
      },
    ],
  },
  {
    hit: hit(
      'atm9',
      'all-the-mods-9',
      'All the Mods 9',
      '四百多个模组的大型科技整合包，任务书齐全，NeoForge 平台。',
      'ATMTeam',
      8_112_003,
      2401,
      ['neoforge', 'technology', 'adventure'],
      ['1.20.1'],
      '2026-07-02T12:00:00Z',
    ),
    loaders: ['neoforge'],
    versions: [
      {
        id: 'atm9-1',
        version_number: '0.3.4',
        version_type: 'release',
        game_versions: ['1.20.1'],
        loaders: ['neoforge'],
        date_published: '2026-07-02T12:00:00Z',
        downloads: 512_003,
        mods: 412,
        mod_bytes: 986_400_000,
      },
    ],
  },
  {
    hit: hit(
      'rlcraft',
      'rlcraft',
      'RLCraft',
      '硬核生存：口渴、体温、真实掉落，死了就是死了。',
      'Shivaxi',
      2_401_882,
      901,
      ['forge', 'adventure', 'hardcore'],
      ['1.12.2'],
      '2026-02-19T09:00:00Z',
    ),
    loaders: ['forge'],
    versions: [
      {
        id: 'rlc-1',
        version_number: '2.9.3',
        version_type: 'release',
        game_versions: ['1.12.2'],
        loaders: ['forge'],
        date_published: '2026-02-19T09:00:00Z',
        downloads: 733_120,
        mods: 186,
        mod_bytes: 402_100_000,
      },
    ],
  },
  {
    hit: hit(
      'better-mc',
      'better-mc-fabric',
      'Better MC [FABRIC]',
      '在原版基础上加内容：新的结构、生物、任务，但不上强度。',
      'LunaPixelStudios',
      5_020_113,
      1310,
      ['fabric', 'adventure', 'quests'],
      ['1.21.1', '1.20.1', '1.19.2'],
      '2026-09-05T18:30:00Z',
    ),
    loaders: ['fabric'],
    versions: [
      {
        id: 'bmc-3',
        version_number: 'BMC4 30.5',
        version_type: 'release',
        game_versions: ['1.21.1'],
        loaders: ['fabric'],
        date_published: '2026-09-05T18:30:00Z',
        downloads: 240_881,
        mods: 231,
        mod_bytes: 612_400_000,
      },
    ],
  },
  {
    hit: hit(
      'quilt-pack',
      'quilt-demo-pack',
      'Quilt 示例整合包',
      '演示 Quilt 加载器的整合包（演示数据，用来验证四种加载器都能装）。',
      'MCDL',
      12_004,
      31,
      ['quilt', 'lightweight'],
      ['1.21.1'],
      '2026-03-01T00:00:00Z',
    ),
    loaders: ['quilt'],
    versions: [
      {
        id: 'quilt-1',
        version_number: '1.0.0',
        version_type: 'release',
        game_versions: ['1.21.1'],
        loaders: ['quilt'],
        date_published: '2026-03-01T00:00:00Z',
        downloads: 4021,
        mods: 18,
        mod_bytes: 12_800_000,
      },
    ],
  },
  {
    hit: hit(
      'vanilla-pack',
      'vanilla-plus',
      'Vanilla Plus',
      '不加加载器的原版整合包：只有配置和资源包（演示数据）。',
      'MCDL',
      8_120,
      12,
      ['vanilla', 'lightweight'],
      ['1.21.8'],
      '2026-05-05T00:00:00Z',
    ),
    loaders: [],
    versions: [
      {
        id: 'vp-1',
        version_number: '1.2.0',
        version_type: 'release',
        game_versions: ['1.21.8'],
        loaders: [],
        date_published: '2026-05-05T00:00:00Z',
        downloads: 3021,
        mods: 4,
        mod_bytes: 1_200_000,
      },
    ],
  },
];

function mockHit(pack: MockPack, source: PackSource): PackHit {
  return {
    ...pack.hit,
    source,
    loaders: pack.loaders,
  };
}

function searchMock(q: PackSearchQuery): Promise<PackSearchResult> {
  const text = q.query.trim().toLowerCase();
  const limit = q.limit && q.limit > 0 ? Math.min(50, q.limit) : 20;
  const offset = Math.max(0, q.offset ?? 0);

  let hits = MOCK_PACKS.map((p) => mockHit(p, q.source));
  if (text) {
    hits = hits.filter(
      (h) =>
        h.title.toLowerCase().includes(text) ||
        h.description.toLowerCase().includes(text) ||
        h.slug.includes(text) ||
        h.categories.some((c) => c.toLowerCase().includes(text)),
    );
  }
  if (q.mc_version) {
    hits = hits.filter((h) => h.game_versions.includes(q.mc_version as string));
  }
  if (q.loader) {
    hits = hits.filter((h) => h.loaders.includes(q.loader as LoaderKind));
  }
  if (q.sort === 'downloads') hits = [...hits].sort((a, b) => b.downloads - a.downloads);
  if (q.sort === 'updated' || q.sort === 'newest') {
    hits = [...hits].sort((a, b) => b.date_modified.localeCompare(a.date_modified));
  }

  const total = hits.length;
  const page = hits.slice(offset, offset + limit);
  return new Promise((resolve) =>
    window.setTimeout(() => resolve({ hits: page, total, offset, limit }), 380),
  );
}

function findMock(id: string): MockPack {
  const pack = MOCK_PACKS.find((p) => p.hit.id === id || p.hit.slug === id);
  if (!pack) throw new Error(`演示数据里没有这个整合包：${id}`);
  return pack;
}

function projectMock(source: PackSource, id: string): Promise<PackProject> {
  const pack = findMock(id);
  return new Promise((resolve) =>
    window.setTimeout(
      () =>
        resolve({
          source,
          id: pack.hit.id,
          slug: pack.hit.slug,
          title: pack.hit.title,
          description: pack.hit.description,
          body:
            `${pack.hit.description}\n\n` +
            '这是浏览器预览用的演示详情。装成桌面应用之后，这里显示 Modrinth / CurseForge 返回的真实正文、' +
            '许可证、源码地址和版本列表；安装也会真的走引擎下载。',
          author: pack.hit.author,
          icon_url: null,
          downloads: pack.hit.downloads,
          followers: pack.hit.followers,
          categories: pack.hit.categories,
          game_versions: pack.hit.game_versions,
          loaders: pack.loaders,
          license: '演示数据',
          source_url: null,
          issues_url: null,
          wiki_url: null,
          published: '2024-01-01T00:00:00Z',
          updated: pack.hit.date_modified,
        }),
      260,
    ),
  );
}

function versionsMock(source: PackSource, id: string): Promise<PackVersion[]> {
  const pack = findMock(id);
  const list: PackVersion[] = pack.versions.map((v) => ({
    id: v.id,
    name: `${v.version_number} for ${v.game_versions[0] ?? '?'}`,
    version_number: v.version_number,
    version_type: v.version_type,
    game_versions: v.game_versions,
    loaders: v.loaders,
    date_published: v.date_published,
    downloads: v.downloads,
    files: [
      {
        filename: `${pack.hit.slug}-${v.version_number}.mrpack`,
        url: '',
        size: v.mod_bytes,
        sha1: null,
        sha512: null,
        primary: true,
      },
    ],
    dependencies: [],
  }));
  // 演示后端不需要 source 参与，但保留参数形状，接真接口时不用改调用点
  void source;
  return new Promise((resolve) => window.setTimeout(() => resolve(list), 300));
}

// ---------------------------------------------------------------- 演示安装

let stopCurrent: (() => void) | null = null;

/** 取消正在演示的安装。真后端里这一步是 abort 掉引擎的 future。 */
export function cancelMock(): boolean {
  if (!stopCurrent) return false;
  stopCurrent();
  return true;
}

interface MockStage {
  kind: 'meta' | 'libraries' | 'assets' | 'client' | 'loader';
  label: string;
  files: number;
  bytes: number;
  ms: number;
}

export function installMock(
  req: PackInstallRequest,
  onEvent: (event: PackEvent) => void,
): Promise<void> {
  cancelMock();

  const pack = MOCK_PACKS.find((p) => p.hit.id === req.project_id || p.hit.slug === req.project_id);
  const version =
    pack?.versions.find((v) => v.id === req.version_id) ?? pack?.versions[0] ?? null;

  const mc = version?.game_versions[0] ?? '1.21.1';
  const loader = version?.loaders[0] ?? null;
  const mods = version?.mods ?? 60;
  const modBytes = version?.mod_bytes ?? 80_000_000;
  const archiveBytes = 180_000;

  // 阶段 1：整合包本体
  const phases: Array<{
    label: string;
    files: number;
    bytes: number;
    ms: number;
    engine?: MockStage[];
  }> = [
    { label: '下载整合包', files: 1, bytes: archiveBytes, ms: 800 },
    {
      label: '安装游戏本体与加载器',
      files: 0,
      bytes: 0,
      ms: 0,
      engine: [
        { kind: 'meta', label: '读取版本清单', files: 0, bytes: 0, ms: 700 },
        { kind: 'libraries', label: '下载依赖库', files: 128, bytes: 62_400_000, ms: 1600 },
        ...(req.skip_assets
          ? []
          : [
              {
                kind: 'assets' as const,
                label: '下载资源文件',
                files: 3021,
                bytes: 679_000_000,
                ms: 2600,
              },
            ]),
        { kind: 'client', label: '下载游戏本体', files: 1, bytes: 26_600_000, ms: 900 },
        ...(loader
          ? [
              {
                kind: 'loader' as const,
                label: '安装模组加载器',
                files: 1,
                bytes: 2_400_000,
                ms: 1200,
              },
            ]
          : []),
      ],
    },
    { label: '写入整合包文件', files: mods, bytes: modBytes, ms: 2200 },
  ];

  const startedAt = performance.now();
  let phase = 0;
  let phaseStartedAt = startedAt;
  let engineIndex = 0;
  let engineStartedAt = startedAt;
  let finished = false;

  const emit = (progress: PackProgress) => onEvent({ type: 'progress', progress });

  const finish = () => {
    if (finished) return;
    finished = true;
    window.clearInterval(timer);
    stopCurrent = null;
    const engineMs = (phases[1].engine ?? []).reduce((sum, s) => sum + s.ms, 0);
    const report: PackInstallReport = {
      source: req.source,
      pack_id: req.project_id,
      pack_name: pack?.hit.title ?? req.project_title ?? req.project_id,
      pack_version: version?.version_number ?? '演示版本',
      mc_version: mc,
      loader: loader ? `${loader} 0.16.10` : null,
      installed_version: loader ? `fabric-loader-0.16.10-${mc}` : mc,
      game_dir: req.game_dir,
      pack_files: mods,
      pack_bytes: modBytes,
      skipped: 0,
      overrides: 12,
      warnings: [],
      duration_ms: Math.round(performance.now() - startedAt),
      base: {
        version_id: mc,
        game_dir: req.game_dir,
        files: 128 + (req.skip_assets ? 0 : 3021) + 1,
        bytes: 62_400_000 + (req.skip_assets ? 0 : 679_000_000) + 26_600_000,
        duration_ms: engineMs,
        java: 'C:\\Java\\jdk-21\\bin\\java.exe',
        installed_version: loader ? `fabric-loader-0.16.10-${mc}` : mc,
      },
    };
    emit({
      type: 'log',
      level: 'info',
      message: `完成：${report.pack_name} ${report.pack_version} · 整合包文件 ${report.pack_files} 个 · 覆盖 ${report.overrides} 个`,
    });
    onEvent({ type: 'done', report });
  };

  const announce = () => {
    const current = phases[phase];
    if (!current) return;
    emit({ type: 'stage', index: phase + 1, total: phases.length, label: current.label });
  };
  announce();

  const timer = window.setInterval(() => {
    if (finished) return;
    const current = phases[phase];
    if (!current) {
      finish();
      return;
    }
    const now = performance.now();
    const raw = Math.min(1, (now - phaseStartedAt) / current.ms);
    const eased = 1 - Math.pow(1 - raw, 2.2);

    if (current.engine) {
      // 第 2 步：把引擎的阶段和文件进度原样转成 engine 事件
      const stages = current.engine;
      const stage = stages[engineIndex];
      if (stage) {
        if (engineStartedAt === phaseStartedAt) {
          emit({
            type: 'engine',
            progress: {
              type: 'stage',
              kind: stage.kind,
              index: engineIndex + 1,
              total: stages.length,
              label: stage.label,
            },
          });
        }
        const sub = Math.min(1, (now - engineStartedAt) / stage.ms);
        if (stage.files > 0) {
          emit({
            type: 'engine',
            progress: {
              type: 'files',
              kind: stage.kind,
              done: Math.round(sub * stage.files),
              total: stage.files,
              bytes_done: Math.round(sub * stage.bytes),
              bytes_total: stage.bytes,
            },
          });
        }
        if (sub >= 1) {
          engineIndex += 1;
          engineStartedAt = now;
          const next = stages[engineIndex];
          if (next) {
            emit({
              type: 'engine',
              progress: {
                type: 'stage',
                kind: next.kind,
                index: engineIndex + 1,
                total: stages.length,
                label: next.label,
              },
            });
          }
        }
      }
      if (engineIndex >= stages.length) {
        phase += 1;
        phaseStartedAt = now;
        engineIndex = 0;
        engineStartedAt = now;
        announce();
      }
      return;
    }

    if (current.files > 0) {
      emit({
        type: 'files',
        done: Math.round(eased * current.files),
        total: current.files,
        bytes_done: Math.round(eased * current.bytes),
        bytes_total: current.bytes,
      });
    }

    if (raw >= 1) {
      phase += 1;
      phaseStartedAt = now;
      if (phase >= phases.length) finish();
      else announce();
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

// ---------------------------------------------------------------- 小工具

/** 演示数据里可用的排序项，界面直接渲染。 */
export const PACK_SORTS: Array<{ value: PackSort; label: string }> = [
  { value: 'relevance', label: '相关度' },
  { value: 'downloads', label: '下载量' },
  { value: 'updated', label: '最近更新' },
  { value: 'newest', label: '最新发布' },
];

export const PACK_SOURCE_LABEL: Record<PackSource, string> = {
  modrinth: 'Modrinth',
  curseforge: 'CurseForge',
};
