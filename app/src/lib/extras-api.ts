// 「装完之后补东西」的前端门：Fabric API、OptiFine、自定义版本名。
//
// 和 `lib/api.ts` 一样：桌面应用里走 `invoke`，浏览器里（`pnpm dev`，没有
// Tauri 注入）自动切到本文件自带的演示实现，界面代码不感知差别。
//
// 这里还带一个模块级的结论仓库：安装结束后由 `InstallForm` 调
// `runExtrasAfterInstall`，`InstallExtras` 用 `useExtrasState` 订阅结果。
// 之所以不用 React state 传：附加项发生在「安装事件流已经结束」之后，
// 而那条事件流由 `useInstall` 管，不在这个组件的管辖范围内。

import { useSyncExternalStore } from 'react';

import { api, IS_TAURI } from './api';
import type { InstalledVersion, LoaderKind } from './types';

// ---------------------------------------------------------------- 协议类型
// 与 `crates/mcdl-core/src/extras/` 里的 serde 输出一一对应。

export type ExtraStatus = 'done' | 'skipped' | 'failed' | 'manual';

export interface ExtraNotice {
  /** `fabric_api` / `optifine` / `rename` */
  item: string;
  status: ExtraStatus;
  message: string;
}

export interface FabricApiFile {
  project: string;
  version: string;
  filename: string;
  url: string;
  size: number;
  sha1?: string | null;
  game_versions: string[];
  loaders: string[];
}

export interface FabricApiOutcome {
  status: ExtraStatus;
  message: string;
  file?: FabricApiFile | null;
  path?: string | null;
  bytes: number;
  already_present: boolean;
}

export type OptifineMode = 'mods_folder' | 'manual' | 'unsupported';

export interface OptifineBuild {
  mc_version: string;
  edition: string;
  filename: string;
  adloadx_url: string;
  download_page: string;
  pre: boolean;
}

export interface OptifinePlan {
  supported: boolean;
  mode: OptifineMode;
  reason: string;
  build?: OptifineBuild | null;
  download_page: string;
  manual_possible: boolean;
}

export interface OptifineOutcome {
  status: ExtraStatus;
  message: string;
  mode: OptifineMode;
  build?: OptifineBuild | null;
  installer_path?: string | null;
  mod_path?: string | null;
  bytes: number;
  sha1?: string | null;
  manual_hint?: string | null;
}

export interface RenameOutcome {
  from: string;
  to: string;
  version_json: string;
  jar_renamed: boolean;
}

export interface ExtrasPlan {
  fabric_api?: FabricApiFile | null;
  fabric_api_error?: string | null;
  optifine: OptifinePlan;
}

export interface ExtrasReport {
  version_id: string;
  notices: ExtraNotice[];
  fabric_api?: FabricApiOutcome | null;
  optifine?: OptifineOutcome | null;
  rename?: RenameOutcome | null;
}

export interface ExtrasRequest {
  game_dir: string;
  mc_version: string;
  loader?: LoaderKind | null;
  installed_version?: string | null;
  fabric_api: boolean;
  optifine: boolean;
  rename_to?: string | null;
  /** 版本隔离：Fabric API / OptiFine 进 `versions/<版本id>/mods/`（默认开）还是根 `mods/` */
  isolated?: boolean;
}

export interface NameCheck {
  ok: boolean;
  reason?: string | null;
}

/** 界面上的三个开关。 */
export interface ExtrasOptions {
  fabricApi: boolean;
  optifine: boolean;
  name: string;
}

export const DEFAULT_EXTRAS: ExtrasOptions = {
  fabricApi: true,
  optifine: false,
  name: '',
};

// ---------------------------------------------------------------- 调用封装

async function core() {
  return import('@tauri-apps/api/core');
}

/** 问一次「这个组合能补什么」。查不到不算失败，界面分开显示。 */
export async function resolveExtras(
  mcVersion: string,
  loader: LoaderKind | null,
): Promise<ExtrasPlan> {
  if (!IS_TAURI) return resolveExtrasMock(mcVersion, loader);
  const cached = planCache.get(cacheKey(mcVersion, loader));
  if (cached && Date.now() - cached.at < PLAN_TTL) return cached.value;
  const { invoke } = await core();
  const plan = await invoke<ExtrasPlan>('extras_resolve', {
    mcVersion,
    loader,
  });
  planCache.set(cacheKey(mcVersion, loader), { at: Date.now(), value: plan });
  return plan;
}

/** 真的补齐。安装结束之后调用，结果进模块级仓库。 */
export async function applyExtras(req: ExtrasRequest): Promise<ExtrasReport> {
  if (!IS_TAURI) return applyExtrasMock(req);
  const { invoke } = await core();
  return invoke<ExtrasReport>('extras_apply', { req });
}

/** 版本名校验。桌面端以引擎为准，浏览器里用同一套规则本地校验。 */
export async function validateVersionName(name: string): Promise<NameCheck> {
  if (!IS_TAURI) return localNameCheck(name);
  const { invoke } = await core();
  return invoke<NameCheck>('extras_validate_name', { name });
}

/**
 * 下载 OptiFine 官方安装器并打开它（Java GUI）。
 *
 * 这不是「自动安装」：OptiFine 的安装器没有可用的命令行模式，打开之后要
 * 用户自己在窗口里点 Install。界面上必须写清楚这一点。
 */
export async function openOptifineInstaller(
  gameDir: string,
  mcVersion: string,
): Promise<OptifineOutcome> {
  if (!IS_TAURI) return openOptifineInstallerMock(gameDir, mcVersion);
  const { invoke } = await core();
  return invoke<OptifineOutcome>('extras_optifine_installer', {
    gameDir,
    mcVersion,
  });
}

// 计划缓存的存活时间。optifine.net 的下载页有 200 KB，来回切加载器不该反复打。
const PLAN_TTL = 5 * 60 * 1000;
const planCache = new Map<string, { at: number; value: ExtrasPlan }>();

function cacheKey(mcVersion: string, loader: LoaderKind | null): string {
  return `${mcVersion}|${loader ?? 'vanilla'}`;
}

// ---------------------------------------------------------------- 本地校验
//
// 和引擎 `extras::rename::check_name` 保持同一套规则：桌面端仍以引擎返回为准，
// 这里只负责输入时的即时反馈（不用等 IPC）。

const MAX_NAME_CHARS = 96;
const FORBIDDEN = new Set(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);

export function localNameCheck(raw: string): NameCheck {
  const name = raw;
  if (!name) return { ok: false, reason: '版本名不能为空' };
  if (name.trim() !== name) return { ok: false, reason: '版本名前后不能有空格' };
  if ([...name].length > MAX_NAME_CHARS) {
    return { ok: false, reason: `版本名最多 ${MAX_NAME_CHARS} 个字符` };
  }
  if (name === '.' || name === '..') return { ok: false, reason: '版本名不能是 . 或 ..' };
  for (const ch of name) {
    if (FORBIDDEN.has(ch) || ch.charCodeAt(0) < 0x20) {
      return {
        ok: false,
        reason: `版本名里不能有 \`${ch}\`（不能含 / \\ : * ? " < > | 和控制字符）`,
      };
    }
  }
  if (name.endsWith('.')) return { ok: false, reason: '版本名不能以点结尾' };
  const stem = name.split('.')[0].toUpperCase();
  if (['CON', 'PRN', 'AUX', 'NUL'].includes(stem) || /^(COM|LPT)[1-9]$/.test(stem)) {
    return { ok: false, reason: `\`${name}\` 是 Windows 的保留设备名，换一个` };
  }
  return { ok: true };
}

// ---------------------------------------------------------------- 结论仓库

export type ExtrasPhase = 'idle' | 'running' | 'done' | 'failed';

export interface ExtrasState {
  phase: ExtrasPhase;
  /** 逐项结论 */
  items: ExtraNotice[];
  report: ExtrasReport | null;
  error: string | null;
  /** 浏览器预览：这些结论是演示数据，不是真的落了文件 */
  demo: boolean;
  /** 这一轮补的是哪个版本目录 */
  versionId: string | null;
  startedAt: number;
}

const INITIAL: ExtrasState = {
  phase: 'idle',
  items: [],
  report: null,
  error: null,
  demo: false,
  versionId: null,
  startedAt: 0,
};

let extrasState: ExtrasState = INITIAL;
const listeners = new Set<() => void>();

function setExtrasState(patch: Partial<ExtrasState>) {
  extrasState = { ...extrasState, ...patch };
  for (const listener of listeners) listener();
}

function subscribeExtras(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getExtrasSnapshot(): ExtrasState {
  return extrasState;
}

/** 订阅附加项结论。组件卸载后自动退订。 */
export function useExtrasState(): ExtrasState {
  return useSyncExternalStore(subscribeExtras, getExtrasSnapshot, getExtrasSnapshot);
}

export interface RunExtrasArgs {
  gameDir: string;
  mcVersion: string;
  loader: LoaderKind | null;
  options: ExtrasOptions;
  /** 这次安装是什么时候开始的（毫秒时间戳），用来认「刚装出来的版本」 */
  startedAt: number;
  /** 版本隔离（设置页的开关）：决定附加模组进版本目录还是根 mods/ */
  isolated: boolean;
}

/**
 * 「这次安装结束后要补什么」的待办。
 *
 * 放在模块作用域而不是组件里：安装期间用户切到别的分区再切回来时，
 * `InstallForm` 会重新挂载，组件内的 ref 就丢了；待办丢了等于附加项静默不执行。
 */
let pendingRun: RunExtrasArgs | null = null;

/** 提交安装时记下待办。 */
export function rememberExtrasRun(args: RunExtrasArgs): void {
  pendingRun = args;
}

/** 取走待办（只能取一次）。 */
export function takeExtrasRun(): RunExtrasArgs | null {
  const pending = pendingRun;
  pendingRun = null;
  return pending;
}

/** 有没有待办。组件挂载时用它判断「上一轮是不是在别处跑完了」。 */
export function hasExtrasRun(): boolean {
  return pendingRun !== null;
}

/**
 * 找出「这次安装产出的那个版本」。
 *
 * 不能只按 mtime 取最新：`Installer::run` 最后一步会把**原版**的版本 JSON
 * 写回 `versions/<mc>/`，所以原版目录的 mtime 反而比加载器目录新。装加载器时
 * 用户实际启动的是加载器那个目录，必须按加载器的命名特征去挑。
 */
function pickFreshVersion(
  installed: InstalledVersion[],
  mcVersion: string,
  loader: LoaderKind | null,
  startedAt: number,
): InstalledVersion | null {
  // 目录 mtime 是秒级；留 10 秒余量，避免刚好卡在边界上。
  const fresh = installed.filter((v) => v.modified * 1000 >= startedAt - 10_000);
  if (fresh.length === 0) return null;

  if (loader) {
    const needle =
      loader === 'fabric'
        ? 'fabric-loader'
        : loader === 'quilt'
          ? 'quilt-loader'
          : loader === 'forge'
            ? '-forge-'
            : 'neoforge';
    const match = fresh.find((v) => v.id.toLowerCase().includes(needle));
    if (match) return match;
    // 加载器目录没出现：这次安装多半没走到最后，别拿原版目录顶替。
    return null;
  }

  return fresh.find((v) => v.id === mcVersion) ?? null;
}

/** 每次补齐算一代；新一轮开始时旧一轮的异步结论要丢掉。 */
let extrasGeneration = 0;

/**
 * 安装结束之后补齐附加项。
 *
 * 由 `InstallForm` 在 `running` 从 true 变 false 时调用。它先确认这次安装
 * 真的产出了一个新版本目录（失败/取消时没有），再去调引擎——不能拿上次装好
 * 的老版本去改名。
 */
export async function runExtrasAfterInstall(args: RunExtrasArgs): Promise<void> {
  const { gameDir, mcVersion, loader, options, startedAt, isolated } = args;
  const renameTo = options.name.trim();
  const generation = ++extrasGeneration;

  if (!options.fabricApi && !options.optifine && !renameTo) {
    setExtrasState({ ...INITIAL });
    return;
  }

  setExtrasState({
    phase: 'running',
    items: [],
    report: null,
    error: null,
    demo: !IS_TAURI,
    versionId: null,
    startedAt: Date.now(),
  });

  try {
    let installedVersion: string | null;
    if (IS_TAURI) {
      const installed = await api.listInstalled(gameDir);
      if (generation !== extrasGeneration) return;
      const fresh = pickFreshVersion(installed, mcVersion, loader, startedAt);
      if (!fresh) {
        setExtrasState({
          phase: 'failed',
          error:
            '这次安装没有产出可用的版本目录（可能失败了，或者被取消），附加项已跳过。',
        });
        return;
      }
      installedVersion = fresh.id;
    } else {
      installedVersion = demoVersionName(mcVersion, loader);
    }

    const report = await applyExtras({
      game_dir: gameDir,
      mc_version: mcVersion,
      loader,
      installed_version: installedVersion,
      fabric_api: options.fabricApi,
      optifine: options.optifine,
      rename_to: renameTo || null,
      isolated,
    });
    if (generation !== extrasGeneration) return;

    setExtrasState({
      phase: 'done',
      items: report.notices,
      report,
      error: null,
      versionId: report.version_id || installedVersion,
    });
  } catch (error) {
    if (generation !== extrasGeneration) return;
    setExtrasState({
      phase: 'failed',
      error: error instanceof Error ? error.message : String(error),
    });
  }
}

/** 清掉上一轮的结论（界面在开始新一轮安装时用）。 */
export function resetExtras(): void {
  extrasGeneration += 1;
  setExtrasState({ ...INITIAL });
}

// ---------------------------------------------------------------- 演示实现

const MOCK_BUILD = (mcVersion: string): OptifineBuild => ({
  mc_version: mcVersion,
  edition: 'J1',
  filename: `OptiFine_${mcVersion}_HD_U_J1.jar`,
  adloadx_url: `https://optifine.net/adloadx?f=OptiFine_${mcVersion}_HD_U_J1.jar`,
  download_page: 'https://optifine.net/downloads',
  pre: false,
});

function mockSupport(loader: LoaderKind | null, mcVersion: string): OptifinePlan {
  const build = MOCK_BUILD(mcVersion);
  if (loader === 'fabric' || loader === 'quilt' || loader === 'neoforge') {
    return {
      supported: false,
      mode: 'unsupported',
      reason:
        loader === 'neoforge'
          ? 'OptiFine 不支持 NeoForge'
          : 'OptiFine 不能直接由 Fabric/Quilt 加载，需要第三方兼容层；本工具不自动装',
      build: null,
      download_page: build.download_page,
      manual_possible: false,
    };
  }
  const modern = loader === 'forge' && !/^1\.(?:[0-9]|1[0-2])(?:\.|$)/.test(mcVersion);
  return {
    supported: modern,
    mode: modern ? 'mods_folder' : 'manual',
    reason: modern
      ? 'Forge 已装：OptiFine 的官方 jar 本身就是 Forge mod，放进 mods/ 即可加载'
      : '原版需要 OptiFine 官方安装器 patch 游戏；可以下载后手动点 Install',
    build,
    download_page: build.download_page,
    manual_possible: true,
  };
}

function resolveExtrasMock(mcVersion: string, loader: LoaderKind | null): ExtrasPlan {
  const fabric =
    loader === 'fabric' || loader === 'quilt'
      ? {
          project: loader === 'quilt' ? 'qsl' : 'fabric-api',
          version: `0.116.17+${mcVersion}`,
          filename:
            loader === 'quilt'
              ? `qfapi-7.0.0+${mcVersion}.jar`
              : `fabric-api-0.116.17+${mcVersion}.jar`,
          url: 'https://cdn.modrinth.com/data/P7dR8mSH/versions/mock/mod.jar',
          size: 2_458_192,
          sha1: 'mock',
          game_versions: [mcVersion],
          loaders: [loader],
        }
      : null;

  return {
    fabric_api: fabric,
    fabric_api_error: null,
    optifine: mockSupport(loader, mcVersion),
  };
}

function applyExtrasMock(req: ExtrasRequest): ExtrasReport {
  const notices: ExtraNotice[] = [];
  let versionId = req.installed_version ?? req.mc_version;

  if (req.rename_to) {
    notices.push({
      item: 'rename',
      status: 'done',
      message: `（演示）版本已改名为 ${req.rename_to}`,
    });
    versionId = req.rename_to;
  }

  if (req.fabric_api) {
    if (req.loader === 'fabric' || req.loader === 'quilt') {
      const filename =
        req.loader === 'quilt'
          ? `qfapi-7.0.0+${req.mc_version}.jar`
          : `fabric-api-0.116.17+${req.mc_version}.jar`;
      notices.push({
        item: 'fabric_api',
        status: 'done',
        message: `（演示）已放入 mods/${filename}（2.3 MB）`,
      });
    } else {
      notices.push({
        item: 'fabric_api',
        status: 'skipped',
        message: '原版不需要 Fabric API',
      });
    }
  }

  if (req.optifine) {
    const plan = mockSupport(req.loader ?? null, req.mc_version);
    notices.push({
      item: 'optifine',
      status: plan.supported ? 'done' : plan.mode === 'manual' ? 'manual' : 'skipped',
      message: plan.supported
        ? `（演示）已放入 mods/${plan.build?.filename ?? 'OptiFine.jar'}`
        : plan.reason,
    });
  }

  return {
    version_id: versionId,
    notices,
    fabric_api: null,
    optifine: null,
    rename: req.rename_to
      ? {
          from: req.installed_version ?? req.mc_version,
          to: req.rename_to,
          version_json: `${req.game_dir}/versions/${req.rename_to}/${req.rename_to}.json`,
          jar_renamed: true,
        }
      : null,
  };
}

function openOptifineInstallerMock(gameDir: string, mcVersion: string): OptifineOutcome {
  const build = MOCK_BUILD(mcVersion);
  return {
    status: 'manual',
    message: `（演示）已下载 ${build.filename}；桌面应用里会同时打开官方安装器`,
    mode: 'manual',
    build,
    installer_path: `${gameDir}/.mcdl/optifine/${build.filename}`,
    mod_path: null,
    bytes: 7_322_249,
    sha1: 'mock',
    manual_hint: '（演示）桌面应用里这里会弹出 OptiFine 的安装窗口，需要你手动点 Install。',
  };
}

/** 演示用：安装器会生成什么样的目录名。 */
function demoVersionName(mcVersion: string, loader: LoaderKind | null): string {
  switch (loader) {
    case 'fabric':
      return `fabric-loader-0.16.9-${mcVersion}`;
    case 'quilt':
      return `quilt-loader-0.27.0-${mcVersion}`;
    case 'forge':
      return `${mcVersion}-forge-52.1.20`;
    case 'neoforge':
      return `neoforge-21.1.264`;
    default:
      return mcVersion;
  }
}

/** 给界面显示用：附加项名字。 */
export const EXTRA_LABEL: Record<string, string> = {
  fabric_api: 'Fabric API',
  optifine: 'OptiFine',
  rename: '版本名',
};

/** 给界面显示用：状态短标签。 */
export const EXTRA_STATUS_LABEL: Record<ExtraStatus, string> = {
  done: '已完成',
  skipped: '已跳过',
  failed: '失败',
  manual: '需要手动完成',
};
