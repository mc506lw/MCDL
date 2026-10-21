// 整合包页：搜 → 看详情 → 一键装到当前游戏目录。
//
// 风格照 `VersionsPage.tsx`：`wrap` / `ef-head` / `ef-panel` / `ef-ledger` / `ef-rows`。
// 圆角 0、单一强调色只做填充、切角只给按钮、细线分区、没有投影和渐变。
//
// 数据来自 `lib/packs-api.ts`：桌面应用走 Tauri 命令，浏览器里走同文件内的演示后端，
// 所以 `pnpm dev` 打开就能把这一页走完整（搜索 → 选版本 → 装 → 看进度）。
//
// 页面**自己管安装状态**：整合包安装和「安装」页的安装是两个独立任务，
// 不挤进 `useInstall` 那台状态机里。

import { useCallback, useEffect, useState } from 'react';
import type { CSSProperties, FormEvent } from 'react';

import { DirPicker } from '../components/DirPicker';
import { T } from '../i18n';
import { api, IS_TAURI } from '../lib/api';
import { formatBytes, formatCount, formatDate, pad2, padPercent } from '../lib/format';
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
  status: 'running' | 'done' | 'failed' | 'cancelled';
  /** 整合包自己的阶段：共 3 步 */
  phase: { index: number; total: number; label: string } | null;
  /** 当前阶段的文件进度（第 1、3 步用它） */
  files: Counts | null;
  /** 第 2 步是引擎在装原版 + 加载器，它自己会报阶段 */
  engine: EngineView | null;
  logs: Array<{ level: LogLevel; message: string; at: number }>;
  report: PackInstallReport | null;
  error: string | null;
  startedAt: number;
}

const MAX_LOGS = 200;

/** 进度比例：有字节就按字节，没有就按文件数，都没有就是「不知道」。 */
function ratioOf(counts: Counts | null): number | null {
  if (!counts) return null;
  if (counts.bytesTotal > 0) return Math.min(1, counts.bytesDone / counts.bytesTotal);
  if (counts.total > 0) return Math.min(1, counts.done / counts.total);
  return null;
}

function pushLog(
  view: InstallView,
  level: LogLevel,
  message: string,
): InstallView {
  const logs = [...view.logs, { level, message, at: Date.now() }];
  return { ...view, logs: logs.slice(-MAX_LOGS) };
}

function applyProgress(view: InstallView, progress: PackProgress): InstallView {
  switch (progress.type) {
    case 'stage':
      return {
        ...view,
        phase: { index: progress.index, total: progress.total, label: progress.label },
        files: null,
        engine: null,
      };
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

  // ------------------------------------------------------------ 安装
  const [install, setInstall] = useState<InstallView | null>(null);

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

  const onEvent = useCallback(
    (event: PackEvent) => {
      setInstall((prev) => {
        if (!prev) return prev;
        switch (event.type) {
          case 'progress':
            return applyProgress(prev, event.progress);
          case 'done':
            return { ...prev, status: 'done', report: event.report, files: null };
          case 'failed':
            return { ...prev, status: 'failed', error: event.message };
          case 'cancelled':
            return { ...prev, status: 'cancelled' };
        }
      });
      if (event.type === 'done') onInstalled?.(event.report);
    },
    [onInstalled],
  );

  async function startInstall() {
    if (!selected) return;
    if (!gameDir.trim()) {
      setDetailError(T.PacksPage.noGameDir);
      return;
    }
    setDetailError(null);
    setInstall({
      status: 'running',
      phase: null,
      files: null,
      engine: null,
      logs: [],
      report: null,
      error: null,
      startedAt: Date.now(),
    });

    const req = {
      game_dir: gameDir.trim(),
      source: selected.source,
      project_id: selected.id,
      version_id: versionId || null,
      project_title: project?.title ?? selected.title,
      api_key: selected.source === 'curseforge' ? cfKey || null : null,
      download_source: downloadSource,
      concurrency,
      java: javaPath.trim() || null,
      skip_assets: skipAssets,
      // 版本隔离是全局设置（设置页），这里读已保存的值：决定模组进
      // versions/<版本id>/mods/ 还是根 mods/。
      isolated: loadSettings().isolated,
    };

    try {
      await packsApi.install(req, onEvent);
    } catch (error) {
      setInstall((prev) =>
        prev
          ? {
              ...prev,
              status: 'failed',
              error: error instanceof Error ? error.message : String(error),
            }
          : prev,
      );
    }
  }

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

  // ------------------------------------------------------------ 渲染

  const chosenVersion = versions?.find((v) => v.id === versionId) ?? versions?.[0] ?? null;
  const activeCounts =
    install?.phase?.index === 2 ? (install.engine?.files ?? null) : (install?.files ?? null);
  // 装完了就是 100%：这时候没有「本阶段」了，不能退化成不知道进度的走格条。
  const ratio = install?.status === 'done' ? 1 : ratioOf(activeCounts);
  const percent = ratio === null ? '——' : padPercent(ratio);
  const total = result?.total ?? 0;

  return (
    <div className="wrap">
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

      {install && (
        <section className="ef-panel hero">
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
            <li>{install.report?.pack_name ?? selected?.title ?? T.PacksPage.packFallback}</li>
            <li>{install.report?.pack_version ?? chosenVersion?.version_number ?? '—'}</li>
            <li>{install.report?.mc_version ?? chosenVersion?.game_versions[0] ?? '—'}</li>
            <li className="path" title={gameDir}>
              {gameDir || T.common.dirUnset}
            </li>
          </ul>

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
            <p className="ef-note" style={{ marginTop: '0.5rem' }}>
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

          {(install.status === 'running' || (install.report && IS_TAURI)) && (
            <div className="ef-actions">
              {install.status === 'running' && (
                <button
                  className="ef-btn small danger"
                  type="button"
                  onClick={() => void cancelInstall()}
                >
                  {T.PacksPage.cancelInstall}
                </button>
              )}
              {install.report && IS_TAURI && (
                <button
                  className="ef-btn small"
                  type="button"
                  onClick={() =>
                    void api.openGameDir(install.report?.game_dir ?? gameDir).catch(() => undefined)
                  }
                >
                  {T.PacksPage.openGameDir}
                </button>
              )}
            </div>
          )}

          {install.logs.length > 0 && (
            <pre
              className="mono"
              style={{
                margin: '0.7rem 0 0',
                maxHeight: '12rem',
                overflow: 'auto',
                fontSize: '0.75rem',
                whiteSpace: 'pre-wrap',
                borderTop: '1px solid var(--ef-rule)',
                paddingTop: '0.5rem',
              }}
            >
              {install.logs
                .slice(-16)
                .map((line) => `${line.level === 'warn' ? '! ' : line.level === 'error' ? '× ' : '· '}${line.message}`)
                .join('\n')}
            </pre>
          )}
        </section>
      )}

      <section className="ef-panel">
        <h2>
          {T.PacksPage.searchHeading} <small>Search</small>
        </h2>

        <form
          className="ef-form"
          onSubmit={(event) => {
            event.preventDefault();
            void runSearch(0);
          }}
        >
          <div className="field">
            <span>{T.PacksPage.sourceLabel}</span>
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
            <small>
              {source === 'modrinth'
                ? T.PacksPage.sourceHintModrinth
                : T.PacksPage.sourceHintCf}
            </small>
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

          <div className="ef-actions">
            <button className="ef-btn go" type="submit" disabled={searching || needCfKey}>
              {searching ? T.PacksPage.searching : T.PacksPage.search}
            </button>
            {result && (
              <span className="ef-field" style={{ alignSelf: 'center', fontSize: '0.8rem' }}>
                {T.PacksPage.resultCount(formatCount(total))}
              </span>
            )}
          </div>
        </form>

        {needCfKey && (
          <p className="ef-notice" data-flag={T.PacksPage.needKeyFlag}>
            {T.PacksPage.cfKeyMissing}
          </p>
        )}

        {searchError && (
          <p className="ef-notice" data-flag={T.PacksPage.searchErrorFlag}>
            {searchError}
          </p>
        )}

        {searching && <span className="ef-skeleton line" aria-label={T.PacksPage.searchingAria} />}

        {!searching && result && result.hits.length === 0 && (
          <p className="ef-empty">{T.PacksPage.empty}</p>
        )}

        {!searching && result && result.hits.length > 0 && (
          <ul className="ef-rows">
            {result.hits.map((hit) => (
              <li key={`${hit.source}-${hit.id}`}>
                <div>
                  <div
                    style={{
                      display: 'flex',
                      alignItems: 'baseline',
                      gap: '0.5rem',
                      flexWrap: 'wrap',
                    }}
                  >
                    <b style={{ fontSize: '1rem' }}>{hit.title}</b>
                    <span className="ef-chip">{hit.author ?? T.PacksPage.authorUnknown}</span>
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
                  <p
                    style={{
                      margin: '0.35rem 0 0',
                      color: 'var(--ef-ink-2)',
                      fontSize: '0.85rem',
                    }}
                  >
                    {hit.description}
                  </p>
                  <div className="meta">
                    <span>{T.PacksPage.downloads(formatCount(hit.downloads))}</span>
                    <span>
                      {hit.game_versions.slice(0, 4).join(' / ')}
                      {hit.game_versions.length > 4 ? ' …' : ''}
                    </span>
                    <span>{formatDate(hit.date_modified)}</span>
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

      {selected && (
        <section className="ef-panel">
          <h2>
            {project?.title ?? selected.title} <small>{PACK_SOURCE_LABEL[selected.source]}</small>
          </h2>

          <ul className="ef-facts ef-field">
            <li>{project?.author ?? selected.author ?? T.PacksPage.authorUnknown}</li>
            <li>{T.PacksPage.downloads(formatCount(project?.downloads ?? selected.downloads))}</li>
            <li>{project?.license ?? T.PacksPage.licenseUnset}</li>
            <li>{formatDate(project?.updated ?? selected.date_modified) || '—'}</li>
          </ul>

          <p style={{ margin: '0.6rem 0 0' }}>
            {project?.description ?? selected.description}
          </p>

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
                  {(project?.game_versions ?? selected.game_versions).slice(0, 6).join(' / ') || '—'}
                </span>
              </li>
              <li>
                <span>{T.PacksPage.loadersLabel}</span>
                <span className="ef-field">
                  {(project?.loaders ?? selected.loaders).map(
                    (name) => LOADER_LABEL[name as LoaderKind] ?? name,
                  ).join(' / ') || T.common.vanilla}
                </span>
              </li>
              <li>
                <span>{T.PacksPage.versionCount}</span>
                <span className="ef-field">{versions?.length ?? '—'}</span>
              </li>
              {project?.source_url && (
                <li>
                  <span>{T.PacksPage.sourceUrl}</span>
                  <span className="ef-field path" title={project.source_url}>
                    {project.source_url}
                  </span>
                </li>
              )}
            </ul>
          )}

          {versions && versions.length > 0 && (
            <>
              <div className="ef-form" style={{ marginTop: '0.9rem' }}>
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
                  <small>
                    {T.PacksPage.versionHint}
                  </small>
                </div>
              </div>

              <div className="ef-actions">
                <button
                  className="ef-btn go"
                  type="button"
                  onClick={() => void startInstall()}
                  disabled={running || !gameDir.trim()}
                >
                  {running ? T.PacksPage.installing : T.PacksPage.installHere}
                </button>
                {running && (
                  <button className="ef-btn small danger" type="button" onClick={() => void cancelInstall()}>
                    {T.common.cancel}
                  </button>
                )}
                {chosenVersion && (
                  <span className="ef-field" style={{ alignSelf: 'center', fontSize: '0.8rem' }}>
                    {chosenVersion.files.find((f) => f.primary)?.filename ?? chosenVersion.version_number}
                  </span>
                )}
              </div>
            </>
          )}

          {versions && versions.length === 0 && (
            <p className="ef-empty">{T.PacksPage.noVersions}</p>
          )}

          {project?.body && (
            <details style={{ marginTop: '0.8rem' }}>
              <summary style={{ cursor: 'pointer', fontWeight: 600 }}>{T.PacksPage.bodySummary}</summary>
              <p
                className="ef-note"
                style={{ whiteSpace: 'pre-wrap', maxHeight: '14rem', overflow: 'auto' }}
              >
                {project.body}
              </p>
            </details>
          )}
        </section>
      )}

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
            <small>
              {cfKey ? T.PacksPage.keySaved(maskKey(cfKey)) : T.PacksPage.keyEmpty}
            </small>
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
    </div>
  );
}
