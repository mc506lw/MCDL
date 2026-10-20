// 与 `crates/mcdl-core/src/types.rs` 一一对应的类型。
// 这个文件是前端侧的唯一协议来源：字段名、枚举值、大小写都照 serde 的输出写，
// 改动它等于改动协议，需要同步 docs/ARCHITECTURE.md。

/** 下载源。默认官方，镜像只能由用户显式选择。 */
export type Source = 'official' | 'bmclapi';

/** 模组加载器。 */
export type LoaderKind = 'fabric' | 'quilt' | 'forge' | 'neoforge';

/** 安装阶段，顺序就是引擎里的执行顺序。 */
export type StageKind = 'java' | 'meta' | 'libraries' | 'assets' | 'client' | 'loader';

export type LogLevel = 'info' | 'warn' | 'error';

export interface VersionSummary {
  id: string;
  /** release / snapshot / old_beta / old_alpha */
  type: string;
  url: string;
  sha1?: string | null;
  releaseTime: string;
}

export interface LatestVersions {
  release: string;
  snapshot: string;
}

export interface VersionManifest {
  latest: LatestVersions;
  versions: VersionSummary[];
}

export interface LoaderSpec {
  kind: LoaderKind;
  mc_version: string;
  loader_version?: string | null;
}

/** 一次安装请求，CLI 和 GUI 唯一的入口参数。 */
export interface InstallRequest {
  game_dir: string;
  mc_version: string;
  loader?: LoaderSpec | null;
  source: Source;
  concurrency: number;
  java?: string | null;
  skip_assets: boolean;
}

export interface InstallReport {
  version_id: string;
  game_dir: string;
  files: number;
  bytes: number;
  duration_ms: number;
  java?: string | null;
  installed_version: string;
}

export interface JavaRuntime {
  path: string;
  major: number;
  version: string;
  vendor: string;
  arch: string;
}

/**
 * 引擎向外抛的进度。
 *
 * 只描述已经发生的事实，不带百分比：不同阶段的总量是逐步确定的，
 * 百分比由界面自己算。
 */
export type Progress =
  | { type: 'stage'; kind: StageKind; index: number; total: number; label: string }
  | {
      type: 'files';
      kind: StageKind;
      done: number;
      total: number;
      bytes_done: number;
      bytes_total: number;
    }
  | { type: 'log'; level: LogLevel; message: string };

/** 命令层在进度之外补的三种收尾状态。 */
export type InstallEvent =
  | { type: 'progress'; progress: Progress }
  | { type: 'done'; report: InstallReport }
  | { type: 'failed'; message: string }
  | { type: 'cancelled' };

export const LOADER_LABEL: Record<LoaderKind, string> = {
  fabric: 'Fabric',
  quilt: 'Quilt',
  forge: 'Forge',
  neoforge: 'NeoForge',
};

export const SOURCE_LABEL: Record<Source, string> = {
  official: '官方源',
  bmclapi: 'BMCLAPI 第三方镜像',
};

/** 磁盘上的一个已装版本，对应引擎 `installed::InstalledVersion`。 */
export interface InstalledVersion {
  /** `versions/` 下的目录名，也是删除时用的标识 */
  id: string;
  /** release / snapshot / old_beta / old_alpha */
  kind: string;
  /** 认出来的模组加载器，原版是 null */
  loader: string | null;
  main_class: string;
  bytes: number;
  files: number;
  /** 目录最后修改时间（Unix 秒） */
  modified: number;
}
