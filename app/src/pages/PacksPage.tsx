// 整合包页：搜 → 选 → 装；本地文件导入走同一条安装流程。
//
// 版式：顶部一行搜索（平台切换 + 关键词 + 搜索），下面两栏
//   · 左：结果列表（每项两行：名称 / 作者·下载量·支持版本）
//   · 右：选中项详情 + 版本 + 安装按钮；安装时原地变成读数面板
//
// 右栏是 sticky 的：点「装到这个目录」和看进度是同一块地方。这不是装饰选择——
// 之前把读数面板放在列表上方，用户滚到详情点安装时面板在视野外 1300px，
// 引擎事件一直在来，界面却"什么都看不到"。
//
// 数据来自 `lib/packs-api.ts`：桌面应用走 Tauri 命令，浏览器里走同文件内的演示后端。

import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, FormEvent } from 'react';

import { DirPicker } from '../components/DirPicker';
import { T } from '../i18n';
import { api, IS_TAURI } from '../lib/api';
import {
  formatBytes,
  formatCount,
  formatEta,
  formatSpeed,
  pad2,
  padPercent,
} from '../lib/format';
import {
  CURSEFORGE_KEY_URL,
  PACK_SORTS,
  PACK_SOURCE_LABEL,
  loadCurseforgeKey,
  maskKey,
  packsApi,
  saveCurseforgeKey,
  type PackEvent,
  type PackHit,
  type PackInstallReport,
  type PackInstallRequest,
  type PackProgress,
  type PackProject,
  type PackSearchResult,
  type PackSort,
  type PackSource,
  type PackVersion,
} from '../lib/packs-api';
import { loadSettings } from '../lib/settings';
import { LOADER_LABEL, type LoaderKind, type LogLevel, type Source } from '../lib/types';

interface Props {
  /** 当前 `.minecraft` 目录 */
  gameDir: string;
  /** 记录过的目录，给 DirPicker 下拉用 */
  dirs: string[];
  onChangeDir: (dir: string) => void;
  onOpenSettings?: () => void;
  /** 装完之后通知外层（刷新版本页、记住目录），可选 */
  onInstalled?: (report: PackInstallReport) => void;
  /** 下面四项来自设置页；不传就用引擎默认值 */
  downloadSource?: Source;
  concurrency?: number;
  javaPath?: string;
  skipAssets?: boolean;
}

/** 搜索框里的加载器筛选项。 */
const LOADER_FILTERS: Array<{ value: '' | LoaderKind; label: string }> = [
  { value: '', label: T.PacksPage.loaderAny },
  { value: 'fabric', label: 'Fabric' },
  { value: 'neoforge', label: 'NeoForge' },
  { value: 'forge', label: 'Forge' },
  { value: 'quilt', label: 'Quilt' },
];

/** 速度取最近 1.6 秒的增量，和安装页的读数一致。 */
const SPEED_WINDOW = 1600;
/** 文件事件很密；状态最多每 100 ms 更新一次，避免一屏几十次重渲染。 */
const PUSH_INTERVAL = 100;
/** 读数面板里最多留几条日志。 */
const MAX_LOGS = 120;
const LOG_TAIL = 5;

interface Counts {
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
}

interface EngineView {
  index: number;
  total: number;
  label: string;
  files: Counts | null;
}

interface InstallView {
  /** 平台搜索来的，还是本地文件导入的 */
  mode: 'platform' | 'local';
  /** 包名 / 文件名 */
  label: string;
  /** 版本号（本地导入时一开始不知道） */
  version: string;
  /** 选中的版本要的 MC 版本；本地导入时装完才知道 */
  mc: string;
  status: 'running' | 'done' | 'failed' | 'cancelled';
  /** 三个阶段的名字，按序号落位 */
  phases: string[];
  phase: { index: number; total: number; label: string } | null;
  /** 当前阶段的文件进度（阶段 1、3 用它） */
  files: Counts | null;
  /** 阶段 2 是引擎在装原版 + 加载器，它自己报阶段 */
  engine: EngineView | null;
  /** 字节/秒，来自最近 1.6 秒的增量 */
  speed: number;
  logs: Array<{ level: LogLevel; message: string }>;
  report: PackInstallReport | null;
  error: string | null;
  startedAt: number;
}

/** 进度比例：有字节就按字节，没有就按文件数，都没有就是「不知道」。 */
function ratioOf(counts: Counts | null): number | null {
  if (!counts) return null;
  if (counts.bytesTotal > 0) return Math.min(1, counts.bytesDone / counts.bytesTotal);
  if (counts.total > 0) return Math.min(1, counts.done / counts.total);
  return null;
}

function pushLog(view: InstallView, level: LogLevel, message: string): InstallView {
  const logs = [...view.logs, { level, message }];
  return { ...view, logs: logs.slice(-MAX_LOGS) };
}

function applyProgress(view: InstallView, progress: PackProgress): InstallView {
  switch (progress.type) {
    case 'stage': {
      const phases = [...view.phases];
      phases[progress.index - 1] = progress.label;
      return {
        ...view,
        phases,
        phase: { index: progress.index, total: progress.total, label: progress.label },
        files: null,
        engine: null,
      };
    }
    case 'files':
      return {
        ...view,
        files: {
          done: progress.done,
          total: progress.total,
          bytesDone: progress.bytes_done,
          bytesTotal: progress.bytes_total,
        },
      };
    case 'log':
      return pushLog(view, progress.level, progress.message);
    case 'engine': {
      const inner = progress.progress;
      if (inner.type === 'stage') {
        return {
          ...view,
          engine: {
            index: inner.index,
            total: inner.total,
            label: inner.label,
            files: null,
          },
        };
      }
      if (inner.type === 'files') {
        const base: EngineView = view.engine ?? {
          index: 1,
          total: 1,
          label: T.PacksPage.engineFallback,
          files: null,
        };
        return {
          ...view,
          engine: {
            ...base,
            files: {
              done: inner.done,
              total: inner.total,
              bytesDone: inner.bytes_done,
              bytesTotal: inner.bytes_total,
            },
          },
        };
      }
      return pushLog(view, inner.level, inner.message);
    }
  }
}

/** 阶段状态：已过 / 当前 / 停住 / 还没到。 */
function phaseState(
  view: InstallView,
  index: number,
): 'is-done' | 'is-now' | 'is-stop' | 'is-wait' {
  const current = view.phase?.index ?? 0;
  const finished = view.status !== 'running';
  if (!finished && index === current) return 'is-now';
  if (finished && view.status !== 'done' && index === current) return 'is-stop';
  if (view.status === 'done' || index < current) return 'is-done';
  return 'is-wait';
}

export function PacksPage({
  gameDir,
  dirs,
  onChangeDir,
  onOpenSettings,
  onInstalled,
  downloadSource = 'official',
  concurrency = 16,
  javaPath = '',
  skipAssets = false,
}: Props) {
  // ------------------------------------------------------------ 搜索
  const [source, setSource] = useState<PackSource>('modrinth');
  const [query, setQuery] = useState('');
  const [mcVersion, setMcVersion] = useState('');
  const [loader, setLoader] = useState<'' | LoaderKind>('');
  const [sort, setSort] = useState<PackSort>('relevance');
  const [result, setResult] = useState<PackSearchResult | null>(null);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState<string | null>(null);

  // ------------------------------------------------------------ 详情
  const [selected, setSelected] = useState<PackHit | null>(null);
  const [project, setProject] = useState<PackProject | null>(null);
  const [projectLoading, setProjectLoading] = useState(false);
  const [versions, setVersions] = useState<PackVersion[] | null>(null);
  const [versionsLoading, setVersionsLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [versionId, setVersionId] = useState('');

  // ------------------------------------------------------------ CurseForge key
  const [cfKey, setCfKey] = useState(loadCurseforgeKey);
  const [cfDraft, setCfDraft] = useState('');

  // ------------------------------------------------------------ 安装 / 导入
  const [install, setInstall] = useState<InstallView | null>(null);
  const [dragging, setDragging] = useState(false);

  const sideRef = useRef<HTMLElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  /** 速度采样：阶段一变就清空，否则跨阶段的字节差会算出假速度。 */
  const meterRef = useRef<{ key: string; samples: Array<{ t: number; b: number }> }>({
    key: '',
    samples: [],
  });
  const lastPushRef = useRef(0);

  const running = install?.status === 'running';
  const needCfKey = source === 'curseforge' && !cfKey;

  const runSearch = useCallback(
    async (offset = 0) => {
      setSearching(true);
      setSearchError(null);
      try {
        const page = await packsApi.search({
          query: query.trim(),
          source,
          limit: 20,
          offset,
          mc_version: mcVersion.trim() || null,
          loader: loader || null,
          sort,
          api_key: source === 'curseforge' ? cfKey || null : null,
        });
        setResult(page);
      } catch (error) {
        setResult(null);
        setSearchError(error instanceof Error ? error.message : String(error));
      } finally {
        setSearching(false);
      }
    },
    [query, source, mcVersion, loader, sort, cfKey],
  );

  // 进页面先给一批「热门整合包」，不然是一张空列表。
  useEffect(() => {
    void runSearch(0);
    // 只在首次挂载时拉一次；之后由「搜索」按钮触发
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function openPack(hit: PackHit) {
    setSelected(hit);
    setProject(null);
    setVersions(null);
    setVersionId('');
    setDetailError(null);

    if (hit.source === 'curseforge' && !cfKey) {
      setDetailError(T.PacksPage.cfKeyMissingDetail);
      return;
    }

    setProjectLoading(true);
    setVersionsLoading(true);
    const key = hit.source === 'curseforge' ? cfKey : null;

    packsApi
      .project(hit.source, hit.id, key)
      .then(setProject)
      .catch((e: unknown) => setDetailError(e instanceof Error ? e.message : String(e)))
      .finally(() => setProjectLoading(false));

    packsApi
      .versions(hit.source, hit.id, key)
      .then((list) => {
        setVersions(list);
        setVersionId(list[0]?.id ?? '');
      })
      .catch((e: unknown) => setDetailError(e instanceof Error ? e.message : String(e)))
      .finally(() => setVersionsLoading(false));
  }

  // ------------------------------------------------------------ 安装

  /** 最近 1.6 秒的平均速度；样本不够就返回 0。 */
  const pushBytes = (bytes: number): number => {
    const now = performance.now();
    const meter = meterRef.current;
    const samples = [...meter.samples, { t: now, b: bytes }].filter(
      (s) => now - s.t <= SPEED_WINDOW,
    );
    meter.samples = samples;
    if (samples.length < 2) return 0;
    const first = samples[0];
    const last = samples[samples.length - 1];
    const dt = (last.t - first.t) / 1000;
    return dt > 0 ? Math.max(0, (last.b - first.b) / dt) : 0;
  };

  const onEvent = useCallback(
    (event: PackEvent) => {
      // 先更新速度采样（只动 ref，不触发渲染）
      let speed: number | null = null;
      let filesEvent = false;
      if (event.type === 'progress') {
        const p = event.progress;
        if (p.type === 'stage') {
          meterRef.current = { key: `p${p.index}`, samples: [] };
          speed = 0;
        } else if (p.type === 'files') {
          filesEvent = true;
          speed = pushBytes(p.bytes_done);
        } else if (p.type === 'engine' && p.progress.type === 'stage') {
          meterRef.current = { key: `e${p.progress.index}`, samples: [] };
          speed = 0;
        } else if (p.type === 'engine' && p.progress.type === 'files') {
          filesEvent = true;
          speed = pushBytes(p.progress.bytes_done);
        }
      }

      // 文件事件最多 10 次/秒更新状态；阶段、日志、收尾事件一律立即更新。
      const now = performance.now();
      if (filesEvent && now - lastPushRef.current < PUSH_INTERVAL) return;
      lastPushRef.current = now;

      setInstall((prev) => {
        if (!prev) return prev;
        switch (event.type) {
          case 'progress': {
            const next = applyProgress(prev, event.progress);
            return speed === null ? next : { ...next, speed };
          }
          case 'done':
            // 保留最后一次 files 事件：结束态里文件数/字节数比一排「—」有用
            return { ...prev, status: 'done', report: event.report, speed: 0 };
          case 'failed':
            return { ...prev, status: 'failed', error: event.message, speed: 0 };
          case 'cancelled':
            return { ...prev, status: 'cancelled', speed: 0 };
        }
      });
      if (event.type === 'done') onInstalled?.(event.report);
    },
    [onInstalled],
  );

  /** 右栏在窄屏时在列表下面：开始装之后把它滚进视野。 */
  function revealReadout() {
    window.requestAnimationFrame(() => {
      sideRef.current?.scrollIntoView({ block: 'nearest' });
    });
  }

  function buildRequest(extra: Partial<PackInstallRequest>): PackInstallRequest {
    return {
      game_dir: gameDir.trim(),
      source: selected?.source ?? 'modrinth',
      project_id: selected?.id ?? '',
      version_id: versionId || null,
      project_title: project?.title ?? selected?.title ?? null,
      api_key: cfKey || null,
      download_source: downloadSource,
      concurrency,
      java: javaPath.trim() || null,
      skip_assets: skipAssets,
      // 版本隔离是全局设置：决定模组进 versions/<版本id>/mods/ 还是根 mods/。
      isolated: loadSettings().isolated,
      ...extra,
    };
  }

  function beginInstall(init: {
    mode: 'platform' | 'local';
    label: string;
    version: string;
    mc: string;
  }) {
    meterRef.current = { key: '', samples: [] };
    lastPushRef.current = 0;
    setInstall({
      mode: init.mode,
      label: init.label,
      version: init.version,
      mc: init.mc,
      status: 'running',
      phases: [],
      phase: null,
      files: null,
      engine: null,
      speed: 0,
      logs: [],
      report: null,
      error: null,
      startedAt: Date.now(),
    });
    revealReadout();
  }

  async function startInstall() {
    if (!selected) return;
    if (!gameDir.trim()) {
      setDetailError(T.PacksPage.noGameDir);
      return;
    }
    setDetailError(null);
    beginInstall({
      mode: 'platform',
      label: project?.title ?? selected.title,
      version: chosenVersion?.version_number ?? '',
      mc: chosenVersion?.game_versions[0] ?? '',
    });

    try {
      await packsApi.install(buildRequest({}), onEvent);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setInstall((prev) =>
        prev ? { ...prev, status: 'failed', error: message, speed: 0 } : prev,
      );
    }
  }

  /** 本地文件导入：格式由引擎按文件内容判断，界面只管把路径递过去。 */
  async function startImport(archive: string) {
    if (running) return;
    if (!gameDir.trim()) {
      setDetailError(T.PacksPage.noGameDir);
      return;
    }
    setDetailError(null);
    const name = archive.split(/[\\/]/).pop() || archive;
    beginInstall({ mode: 'local', label: name, version: '', mc: '' });

    try {
      await packsApi.install(
        // 本地文件不需要 project_id / version_id：格式和版本都从文件里读
        buildRequest({ archive, project_id: '', version_id: null, project_title: null }),
        onEvent,
      );
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setInstall((prev) =>
        prev ? { ...prev, status: 'failed', error: message, speed: 0 } : prev,
      );
    }
  }

  // 拖放要拿到最新的 startImport，而监听只注册一次
  const startImportRef = useRef(startImport);
  startImportRef.current = startImport;

  async function pickArchive() {
    if (running) return;
    if (!IS_TAURI) {
      // 浏览器拿不到真实路径，用文件选择框拿到文件名走演示后端
      fileInputRef.current?.click();
      return;
    }
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({
        multiple: false,
        directory: false,
        title: T.PacksPage.importDialogTitle,
        filters: [{ name: T.PacksPage.importFilterName, extensions: ['mrpack', 'zip'] }],
      });
      if (typeof picked === 'string' && picked) await startImport(picked);
    } catch {
      // 用户取消，或对话框不可用
    }
  }

  // Tauri 的原生拖放：WebView 里不会触发 HTML5 的 drop 事件
  useEffect(() => {
    if (!IS_TAURI) return;
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void (async () => {
      try {
        const { getCurrentWebview } = await import('@tauri-apps/api/webview');
        const un = await getCurrentWebview().onDragDropEvent((event) => {
          const payload = event.payload;
          if (payload.type === 'over') {
            setDragging(true);
          } else if (payload.type === 'drop') {
            setDragging(false);
            const path = payload.paths?.[0];
            if (path) void startImportRef.current(path);
          } else {
            setDragging(false);
          }
        });
        if (disposed) un();
        else unlisten = un;
      } catch {
        // 老版本 webview 没有这个 API：仍然可以用「导入文件」按钮
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  async function cancelInstall() {
    try {
      await packsApi.cancel();
    } catch {
      // 取消失败不该再弹一个错误：状态机自己会收到 cancelled 事件
    }
  }

  // ------------------------------------------------------------ key

  /** 换平台：旧结果和旧选中都不属于这个平台了，一起清掉。 */
  function switchSource(next: PackSource) {
    if (next === source) return;
    setSource(next);
    setResult(null);
    setSelected(null);
    setProject(null);
    setVersions(null);
    setDetailError(null);
  }

  function saveKey(event: FormEvent) {
    event.preventDefault();
    const next = cfDraft.trim();
    saveCurseforgeKey(next);
    setCfKey(next);
    setCfDraft('');
  }

  function clearKey() {
    saveCurseforgeKey('');
    setCfKey('');
    setCfDraft('');
  }

  function clearFilters() {
    setQuery('');
    setMcVersion('');
    setLoader('');
    setSort('relevance');
    void packsApi
      .search({ query: '', source, limit: 20, offset: 0, sort: 'relevance', api_key: source === 'curseforge' ? cfKey || null : null })
      .then(setResult)
      .catch(() => setResult(null));
  }

  // ------------------------------------------------------------ 渲染

  const chosenVersion = versions?.find((v) => v.id === versionId) ?? versions?.[0] ?? null;
  const activeCounts =
    install?.phase?.index === 2 ? (install.engine?.files ?? null) : (install?.files ?? null);
  // 装完了就是 100%：这时没有「本阶段」，不能退化成不知道进度的走格条。
  const ratio = install?.status === 'done' ? 1 : ratioOf(activeCounts);
  const percent = ratio === null ? '——' : padPercent(ratio);
  const hits = result?.hits ?? [];
  const speed = install?.speed ?? 0;
  const speedText = formatSpeed(speed);
  const remaining =
    activeCounts && activeCounts.bytesTotal > activeCounts.bytesDone && speed > 0
      ? (activeCounts.bytesTotal - activeCounts.bytesDone) / speed
      : null;

  return (
    <div
      className="wrap packs-page"
      onDragOver={(event) => {
        if (IS_TAURI) return;
        event.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => {
        if (!IS_TAURI) setDragging(false);
      }}
      onDrop={(event) => {
        if (IS_TAURI) return;
        event.preventDefault();
        setDragging(false);
        const file = event.dataTransfer.files?.[0];
        if (file) void startImport(file.name);
      }}
    >
      <header className="ef-head">
        <h1>
          {T.nav.packs} <small>Modpacks</small>
        </h1>
        <p>{T.PacksPage.lede}</p>
      </header>

      {!IS_TAURI && (
        <p className="ef-notice" data-flag={T.common.demoData}>
          {T.PacksPage.demoNote}
        </p>
      )}

      <section className="ef-panel packs-search">
        <form
          className="packs-bar"
          onSubmit={(event) => {
            event.preventDefault();
            void runSearch(0);
          }}
        >
          <div className="packs-bar-main">
            <div className="ef-seg" role="radiogroup" aria-label={T.PacksPage.sourceAria}>
              <label>
                <input
                  type="radio"
                  name="pack-source"
                  checked={source === 'modrinth'}
                  onChange={() => switchSource('modrinth')}
                />
                Modrinth
              </label>
              <label>
                <input
                  type="radio"
                  name="pack-source"
                  checked={source === 'curseforge'}
                  onChange={() => switchSource('curseforge')}
                />
                CurseForge
              </label>
            </div>

            <div className="field">
              <span id="pack-query-label">{T.PacksPage.queryLabel}</span>
              <input
                aria-labelledby="pack-query-label"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={T.PacksPage.queryPlaceholder}
                spellCheck={false}
              />
            </div>
          </div>

          <button className="ef-btn go" type="submit" disabled={searching || needCfKey}>
            {searching ? T.PacksPage.searching : T.PacksPage.search}
          </button>
        </form>

        <div className="packs-filters">
          <div className="field">
            <span id="pack-mc-label">{T.PacksPage.mcLabel}</span>
            <input
              aria-labelledby="pack-mc-label"
              value={mcVersion}
              onChange={(event) => setMcVersion(event.target.value)}
              placeholder={T.PacksPage.mcPlaceholder}
              spellCheck={false}
            />
          </div>

          <div className="field">
            <span id="pack-loader-label">{T.PacksPage.loaderLabel}</span>
            <div className="select-wrap">
              <select
                aria-labelledby="pack-loader-label"
                value={loader}
                onChange={(event) => setLoader(event.target.value as '' | LoaderKind)}
              >
                {LOADER_FILTERS.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <div className="field">
            <span id="pack-sort-label">{T.PacksPage.sortLabel}</span>
            <div className="select-wrap">
              <select
                aria-labelledby="pack-sort-label"
                value={sort}
                onChange={(event) => setSort(event.target.value as PackSort)}
              >
                {PACK_SORTS.map((item) => (
                  <option key={item.value} value={item.value}>
                    {item.label}
                  </option>
                ))}
              </select>
            </div>
          </div>

          <button
            className="ef-btn small"
            type="button"
            onClick={() => void pickArchive()}
            disabled={running}
            title={T.PacksPage.importHint}
          >
            {T.PacksPage.importFile}
          </button>
        </div>

        <p className="ef-note">
          {needCfKey
            ? T.PacksPage.cfKeyMissing
            : source === 'modrinth'
              ? T.PacksPage.sourceHintModrinth
              : T.PacksPage.sourceHintCf}
        </p>

        {searchError && (
          <p className="ef-notice" data-flag={T.PacksPage.searchErrorFlag}>
            {searchError}
          </p>
        )}

        {detailError && !selected && (
          <p className="ef-notice" data-flag={T.PacksPage.importFailedFlag}>
            {detailError}
          </p>
        )}

        <input
          ref={fileInputRef}
          type="file"
          accept=".mrpack,.zip"
          hidden
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = '';
            if (file) void startImport(file.name);
          }}
        />
      </section>

      <div className="packs-work">
        <section className="ef-panel packs-list">
          <h2>
            {T.PacksPage.listHeading}{' '}
            <small>
              {searching
                ? T.PacksPage.searching
                : result
                  ? T.PacksPage.resultCount(formatCount(result.total))
                  : T.common.none}
            </small>
          </h2>

          {searching && <span className="ef-skeleton line" aria-label={T.PacksPage.searchingAria} />}

          {!searching && hits.length === 0 && (
            <div className="packs-empty">
              <b>{T.PacksPage.listEmpty}</b>
              <div className="ef-actions">
                <button className="ef-btn small" type="button" onClick={clearFilters}>
                  {T.PacksPage.clearFilters}
                </button>
                <button className="ef-btn small" type="button" onClick={() => void pickArchive()}>
                  {T.PacksPage.importFile}
                </button>
              </div>
            </div>
          )}

          {!searching && hits.length > 0 && (
            <ul className="ef-rows packs-rows">
              {hits.map((hit) => (
                <li key={`${hit.source}-${hit.id}`} aria-current={selected?.id === hit.id}>
                  <div>
                    <div className="name">
                      <b>{hit.title}</b>
                      {hit.loaders.length === 0 ? (
                        <span className="ef-chip">{T.common.vanilla}</span>
                      ) : (
                        hit.loaders.map((name) => (
                          <span key={name} className="ef-chip">
                            {LOADER_LABEL[name as LoaderKind] ?? name}
                          </span>
                        ))
                      )}
                    </div>
                    <div className="meta">
                      <span>{hit.author ?? T.PacksPage.authorUnknown}</span>
                      <span>{T.PacksPage.downloads(formatCount(hit.downloads))}</span>
                      <span>
                        {hit.game_versions.slice(0, 3).join(' / ')}
                        {hit.game_versions.length > 3 ? ' …' : ''}
                      </span>
                    </div>
                  </div>
                  <div className="acts">
                    <button
                      className="ef-btn small"
                      type="button"
                      aria-pressed={selected?.id === hit.id}
                      onClick={() => void openPack(hit)}
                    >
                      {selected?.id === hit.id ? T.PacksPage.selected : T.PacksPage.view}
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>

        <aside className="packs-side" ref={sideRef} aria-live={install ? 'polite' : undefined}>
          {install ? (
            <section className="ef-panel packs-readout hero">
              <h2>
                {T.PacksPage.installHeading}{' '}
                <small>
                  {install.status === 'running'
                    ? 'Running'
                    : install.status === 'done'
                      ? 'Done'
                      : install.status === 'cancelled'
                        ? 'Cancelled'
                        : 'Failed'}
                </small>
              </h2>

              <ul className="ef-facts ef-field">
                <li title={install.label}>{install.label}</li>
                <li>
                  {install.report?.pack_version ??
                    (install.version || (install.mode === 'local' ? T.PacksPage.localFile : '—'))}
                </li>
                <li>{install.report?.mc_version ?? (install.mc || '—')}</li>
                <li className="path" title={gameDir}>
                  {gameDir || T.common.dirUnset}
                </li>
              </ul>

              <div className="ef-figs">
                <div>
                  <b>{install.status === 'running' && speed > 0 ? speedText.value : '—'}</b>
                  <small>
                    {install.status === 'running' && speed > 0
                      ? T.PacksPage.speed(speedText.unit)
                      : T.PacksPage.speedPlain}
                  </small>
                </div>
                <div>
                  <b>{remaining === null ? '—' : formatEta(remaining)}</b>
                  <small>{T.PacksPage.eta}</small>
                </div>
                <div>
                  <b>
                    {activeCounts && activeCounts.total > 0
                      ? formatCount(activeCounts.done)
                      : activeCounts
                        ? formatBytes(activeCounts.bytesDone)
                        : '—'}
                  </b>
                  <small>
                    {activeCounts && activeCounts.total > 0
                      ? T.PacksPage.filesOf(
                          formatCount(activeCounts.done),
                          formatCount(activeCounts.total),
                        )
                      : T.PacksPage.filesPlain}
                  </small>
                </div>
              </div>

              <div
                className={`ef-progress${ratio === null ? ' is-unknown' : ''}`}
                role="progressbar"
                aria-label={T.PacksPage.installProgressAria}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={ratio === null ? undefined : Math.round(ratio * 100)}
                style={ratio === null ? undefined : ({ '--p': ratio } as CSSProperties)}
              >
                <span className="ef-eyebrow">
                  {install.phase
                    ? T.common.stageLine(
                        pad2(install.phase.index),
                        pad2(install.phase.total),
                        install.phase.label,
                      )
                    : T.common.preparing}
                </span>
                <span className="ef-field">{percent}</span>
                <i />
              </div>

              {install.engine && (
                <p className="ef-note">
                  {T.PacksPage.engineLine(
                    pad2(install.engine.index),
                    pad2(install.engine.total),
                    install.engine.label,
                  )}
                  {install.engine.files && install.engine.files.total > 0
                    ? T.PacksPage.engineFiles(
                        formatCount(install.engine.files.done),
                        formatCount(install.engine.files.total),
                      )
                    : ''}
                </p>
              )}

              {install.phase && install.phase.total > 0 && (
                <ol className="packs-rail" aria-label={T.PacksPage.phaseRailAria}>
                  {Array.from({ length: install.phase.total }, (_, i) => {
                    const state = phaseState(install, i + 1);
                    return (
                      <li
                        key={i}
                        className={state}
                        aria-current={state === 'is-now' ? 'step' : undefined}
                      >
                        <span className="no">{pad2(i + 1)}</span>
                        <span className="zh">
                          {install.phases[i] ?? T.PacksPage.phaseWaiting}
                        </span>
                      </li>
                    );
                  })}
                </ol>
              )}

              <div className="ef-actions">
                {install.status === 'running' ? (
                  <button
                    className="ef-btn small danger"
                    type="button"
                    onClick={() => void cancelInstall()}
                  >
                    {T.PacksPage.cancelInstall}
                  </button>
                ) : (
                  <button
                    className="ef-btn small"
                    type="button"
                    onClick={() => setInstall(null)}
                  >
                    {T.PacksPage.backToDetail}
                  </button>
                )}
                {install.report && IS_TAURI && (
                  <button
                    className="ef-btn small"
                    type="button"
                    onClick={() =>
                      void api
                        .openGameDir(install.report?.game_dir ?? gameDir)
                        .catch(() => undefined)
                    }
                  >
                    {T.PacksPage.openGameDir}
                  </button>
                )}
              </div>

              <ul className="ef-ledger">
                <li>
                  <span>{T.PacksPage.transferred}</span>
                  <span className="ef-field">
                    {activeCounts ? formatBytes(activeCounts.bytesDone) : '—'}
                  </span>
                </li>
                <li>
                  <span>{T.common.elapsed}</span>
                  <span className="ef-field">
                    {install.status === 'running'
                      ? `${((Date.now() - install.startedAt) / 1000).toFixed(1)} s`
                      : install.report
                        ? `${(install.report.duration_ms / 1000).toFixed(1)} s`
                        : '—'}
                  </span>
                </li>
                <li>
                  <span>{T.common.status}</span>
                  <span className="ef-field">
                    {install.status === 'running'
                      ? T.common.statusRunning
                      : install.status === 'done'
                        ? T.common.statusDone
                        : install.status === 'cancelled'
                          ? T.common.statusCancelled
                          : T.common.statusFailed}
                  </span>
                </li>
                {install.report && (
                  <>
                    <li>
                      <span>{T.PacksPage.installedVersion}</span>
                      <span className="ef-field">{install.report.installed_version}</span>
                    </li>
                    <li>
                      <span>{T.PacksPage.packFiles}</span>
                      <span className="ef-field">
                        {T.PacksPage.packFilesValue(
                          formatCount(install.report.pack_files),
                          formatBytes(install.report.pack_bytes),
                        )}
                      </span>
                    </li>
                    <li>
                      <span>{T.PacksPage.modsDir}</span>
                      <span className="ef-field path" title={install.report.mods_dir}>
                        {install.report.mods_dir || '—'}
                      </span>
                    </li>
                    <li>
                      <span>{T.PacksPage.overridesSkipped}</span>
                      <span className="ef-field">
                        {install.report.overrides} / {install.report.skipped}
                      </span>
                    </li>
                  </>
                )}
              </ul>

              {install.error && (
                <p className="ef-notice" data-flag={T.PacksPage.installFailedFlag}>
                  {install.error}
                </p>
              )}

              {install.report && install.report.warnings.length > 0 && (
                <div className="ef-notice" data-flag={T.PacksPage.warningFlag}>
                  <ul style={{ margin: 0, paddingLeft: '1.1rem' }}>
                    {install.report.warnings.map((warning) => (
                      <li key={warning}>{warning}</li>
                    ))}
                  </ul>
                </div>
              )}

              {install.logs.length > 0 && (
                <pre className="packs-log mono">
                  {install.logs
                    .slice(-LOG_TAIL)
                    .map(
                      (line) =>
                        `${line.level === 'warn' ? '! ' : line.level === 'error' ? '× ' : '· '}${line.message}`,
                    )
                    .join('\n')}
                </pre>
              )}
            </section>
          ) : selected ? (
            <section className="ef-panel packs-detail">
              <h2>
                {T.PacksPage.detailHeading}{' '}
                <small>{PACK_SOURCE_LABEL[selected.source]}</small>
              </h2>

              <div>
                <p className="packs-title">{project?.title ?? selected.title}</p>
                <p className="packs-sub">
                  {project?.author ?? selected.author ?? T.PacksPage.authorUnknown}
                  {' · '}
                  {T.PacksPage.downloads(formatCount(project?.downloads ?? selected.downloads))}
                  {' · '}
                  {project?.license ?? T.PacksPage.licenseUnset}
                </p>
              </div>

              <p className="packs-desc">{project?.description ?? selected.description}</p>

              {detailError && (
                <p className="ef-notice" data-flag={T.PacksPage.detailFlag}>
                  {detailError}
                </p>
              )}

              {projectLoading || versionsLoading ? (
                <span className="ef-skeleton line" aria-label={T.PacksPage.detailLoading} />
              ) : (
                <ul className="ef-ledger">
                  <li>
                    <span>{T.PacksPage.supportedMc}</span>
                    <span className="ef-field">
                      {(project?.game_versions ?? selected.game_versions).slice(0, 4).join(' / ') ||
                        '—'}
                    </span>
                  </li>
                  <li>
                    <span>{T.PacksPage.loadersLabel}</span>
                    <span className="ef-field">
                      {(project?.loaders ?? selected.loaders)
                        .map((name) => LOADER_LABEL[name as LoaderKind] ?? name)
                        .join(' / ') || T.common.vanilla}
                    </span>
                  </li>
                  <li>
                    <span>{T.PacksPage.versionCount}</span>
                    <span className="ef-field">{versions?.length ?? '—'}</span>
                  </li>
                  <li>
                    <span>{T.PacksPage.sourceUrl}</span>
                    <span className="ef-field path" title={project?.source_url ?? ''}>
                      {project?.source_url ?? '—'}
                    </span>
                  </li>
                </ul>
              )}

              {versions && versions.length > 0 && (
                <>
                  <div className="field">
                    <span id="pack-version-label">{T.PacksPage.pickVersion}</span>
                    <div className="select-wrap">
                      <select
                        aria-labelledby="pack-version-label"
                        value={versionId}
                        onChange={(event) => setVersionId(event.target.value)}
                        disabled={running}
                      >
                        {versions.map((version) => (
                          <option key={version.id} value={version.id}>
                            {version.version_number} · {version.game_versions.join('/')}
                            {version.version_type !== 'release' ? ` · ${version.version_type}` : ''}
                          </option>
                        ))}
                      </select>
                    </div>
                    <small>{T.PacksPage.versionHint}</small>
                  </div>

                  <div className="packs-actions">
                    <button
                      className="ef-btn go"
                      type="button"
                      onClick={() => void startInstall()}
                      disabled={running || !gameDir.trim()}
                    >
                      {running ? T.PacksPage.installing : T.PacksPage.installHere}
                    </button>
                  </div>
                  {chosenVersion && (
                    <span className="packs-file">
                      {chosenVersion.files.find((f) => f.primary)?.filename ??
                        chosenVersion.version_number}
                    </span>
                  )}
                </>
              )}

              {versions && versions.length === 0 && (
                <p className="ef-empty">{T.PacksPage.noVersions}</p>
              )}

              {project?.body && (
                <details>
                  <summary style={{ cursor: 'pointer', fontWeight: 600 }}>
                    {T.PacksPage.bodySummary}
                  </summary>
                  <p
                    className="ef-note"
                    style={{ whiteSpace: 'pre-wrap', maxHeight: '12rem', overflow: 'auto' }}
                  >
                    {project.body}
                  </p>
                </details>
              )}
            </section>
          ) : (
            <section className="ef-panel">
              <div className="packs-empty">
                <b>{T.PacksPage.sideEmptyTitle}</b>
                <p>{T.PacksPage.sideEmptyBody}</p>
                <div className="ef-actions">
                  <button className="ef-btn small" type="button" onClick={() => void pickArchive()}>
                    {T.PacksPage.importFile}
                  </button>
                </div>
                <p className="ef-note">{T.PacksPage.importHint}</p>
              </div>
            </section>
          )}
        </aside>
      </div>

      <section className="ef-panel">
        <h2>
          {T.PacksPage.targetHeading} <small>Target</small>
        </h2>
        <DirPicker value={gameDir} dirs={dirs} onChange={onChangeDir} label={T.common.gameDir} />
        <p className="ef-note">{T.PacksPage.targetNote}</p>
        {onOpenSettings && (
          <div className="ef-actions">
            <button className="ef-btn small" type="button" onClick={onOpenSettings}>
              {T.common.manageDirs}
            </button>
          </div>
        )}
      </section>

      <section className="ef-panel">
        <h2>
          {T.PacksPage.keyHeading} <small>Key</small>
        </h2>
        <p className="ef-note">
          {T.PacksPage.keyNote}
          <a href={CURSEFORGE_KEY_URL} target="_blank" rel="noreferrer">
            {CURSEFORGE_KEY_URL}
          </a>
        </p>

        <form className="ef-form" onSubmit={saveKey}>
          <div className="field">
            <span id="cf-key-label">API key</span>
            <input
              aria-labelledby="cf-key-label"
              type="password"
              value={cfDraft}
              onChange={(event) => setCfDraft(event.target.value)}
              placeholder={T.PacksPage.keyPlaceholder}
              autoComplete="off"
              spellCheck={false}
            />
            <small>{cfKey ? T.PacksPage.keySaved(maskKey(cfKey)) : T.PacksPage.keyEmpty}</small>
          </div>
          <div className="ef-actions">
            <button className="ef-btn small" type="submit" disabled={!cfDraft.trim()}>
              {T.PacksPage.keySave}
            </button>
            {cfKey && (
              <button className="ef-btn small danger" type="button" onClick={clearKey}>
                {T.PacksPage.keyClear}
              </button>
            )}
          </div>
        </form>
      </section>

      {dragging && (
        <div className="packs-drop" role="presentation" aria-label={T.PacksPage.importDropAria}>
          <b>{T.PacksPage.importDrop}</b>
        </div>
      )}
    </div>
  );
}
