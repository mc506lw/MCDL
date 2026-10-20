// 前端与引擎之间唯一的门。
//
// 桌面应用里走 `invoke`；在浏览器里（`pnpm dev`，没有 Tauri 注入）自动切到
// `mock.ts` 的演示后端。两边的函数签名、事件结构完全一致，界面代码不感知差别。

import type {
  InstallEvent,
  InstallReport,
  InstallRequest,
  JavaRuntime,
  LoaderKind,
  VersionManifest,
} from './types';
import {
  cancelMock,
  defaultGameDirMock,
  detectJavaMock,
  installMock,
  listLoadersMock,
  listRuntimesMock,
  listVersionsMock,
  openGameDirMock,
} from './mock';

/** 是否跑在 Tauri 窗口里。 */
export const IS_TAURI =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

/** 通道要活到安装结束，放在模块作用域里。 */
let liveChannel: { onmessage: ((event: InstallEvent) => void) | null } | null = null;
async function core() {
  return import('@tauri-apps/api/core');
}

export const api = {
  /** 版本清单。永远走官方接口。 */
  async listVersions(): Promise<VersionManifest> {
    if (!IS_TAURI) return listVersionsMock();
    const { invoke } = await core();
    return invoke<VersionManifest>('list_versions');
  },

  /** 开始一次安装。立刻返回，进度与结果都从 `onEvent` 来。 */
  async install(
    req: InstallRequest,
    onEvent: (event: InstallEvent) => void,
  ): Promise<void> {
    if (!IS_TAURI) return installMock(req, onEvent);
    const { invoke, Channel } = await core();
    // 上一轮如果还挂着，先摘掉它的回调
    if (liveChannel) liveChannel.onmessage = null;
    const channel = new Channel<InstallEvent>();
    channel.onmessage = onEvent;
    liveChannel = channel;
    await invoke('install', { req, onEvent: channel });
  },

  /** 取消正在跑的安装。返回是否真的取消了。 */
  async cancel(): Promise<boolean> {
    if (!IS_TAURI) return cancelMock();
    const { invoke } = await core();
    return invoke<boolean>('cancel');
  },

  async detectJava(): Promise<JavaRuntime[]> {
    if (!IS_TAURI) return detectJavaMock();
    const { invoke } = await core();
    return invoke<JavaRuntime[]>('detect_java');
  },

  /**
   * 某个加载器对某个 MC 版本可用的版本列表。
   *
   * 引擎侧最多 20 条、从新到旧、5 分钟缓存；单次查询 ≤10 秒，超时会 reject，
   * 调用方要能降级成「用最新版」。
   */
  async listLoaders(kind: LoaderKind, mcVersion: string): Promise<string[]> {
    if (!IS_TAURI) return listLoadersMock(kind);
    const { invoke } = await core();
    return invoke<string[]>('list_loaders', { kind, mcVersion });
  },

  /** 游戏目录下已经落盘的 JRE：`runtime/jre-<major>/bin/java.exe`。 */
  async listRuntimes(gameDir: string): Promise<string[]> {
    if (!IS_TAURI) return listRuntimesMock();
    const { invoke } = await core();
    return invoke<string[]>('list_runtimes', { gameDir });
  },

  async defaultGameDir(): Promise<string> {
    if (!IS_TAURI) return defaultGameDirMock();
    const { invoke } = await core();
    return invoke<string>('default_game_dir');
  },

  async openGameDir(path: string): Promise<void> {
    if (!IS_TAURI) return openGameDirMock();
    const { invoke } = await core();
    await invoke('open_game_dir', { path });
  },

  /** 装完之后释放通道。 */
  releaseChannel() {
    liveChannel = null;
  },
};

export type { InstallEvent, InstallReport, InstallRequest, JavaRuntime, VersionManifest };