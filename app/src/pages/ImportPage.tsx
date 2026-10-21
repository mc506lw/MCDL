// 导入页：把别的启动器里已经装好的东西搬过来。
//
// 数据全部来自磁盘扫描（`import::scan`），不是我们自己的记账：PCL / HMCL /
// BakaXL / Prism / MultiMC / 官方启动器留下的目录都会被找出来。
// 「扫描」只读配置和目录名，体积是拿到列表之后逐项懒加载的——大目录不会拖慢首屏。
//
// 风格沿用终末地那套：圆角 0、单一强调色只做填充、切角表示可点击、细线分区。
// 页面自己的版式用内联样式写，不去动 `app.css` / `endfield.css`（那里是共用的）。

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';

import { DirPicker } from '../components/DirPicker';
import { IS_TAURI } from '../lib/api';
import { formatBytes, formatCount, formatDuration, pad2, padPercent } from '../lib/format';
import {
  ITEM_KIND_LABEL,
  LAUNCHER_LABEL,
  importApi,
  makeImportRequest,
  type DirSize,
  type ImportItem,
  type ImportProgress,
  type ImportReport,
  type ImportRequest,
  type ImportScan,
} from '../lib/import-api';

interface Props {
  gameDir: string;
  dirs: string[];
  onChangeDir: (dir: string) => void;
  onOpenSettings: () => void;
}

const S = {
  row: { display: 'grid', gap: '0.4rem', flex: 1, minWidth: 0 } as CSSProperties,
  head: {
    display: 'flex',
    alignItems: 'baseline',
    flexWrap: 'wrap',
    gap: '0.45rem',
  } as CSSProperties,
  chips: { display: 'flex', flexWrap: 'wrap', gap: '0.35rem' } as CSSProperties,
  sub: {
    color: 'var(--ef-ink-2)',
    fontSize: 'var(--ef-t-field)',
    fontStretch: 'var(--ef-wide)',
    fontVariantNumeric: 'tabular-nums',
  } as CSSProperties,
  options: { display: 'grid', gap: '0.1rem' } as CSSProperties,
  logs: {
    margin: '0.6rem 0 0',
    padding: 0,
    listStyle: 'none',
    fontFamily: 'var(--ef-font-mono)',
    fontSize: '0.78rem',
    lineHeight: 1.7,
    color: 'var(--ef-ink-2)',
    maxHeight: '11rem',
    overflowY: 'auto',
  } as CSSProperties,
  probe: {
    display: 'grid',
    gridTemplateColumns: 'auto minmax(0, 1fr)',
    gap: '0.2rem 0.6rem',
    padding: '0.35rem 0',
    borderBottom: '1px solid var(--ef-rule)',
  } as CSSProperties,
};

export function ImportPage({ gameDir, dirs, onChangeDir, onOpenSettings }: Props) {
  const [scan, setScan] = useState<ImportScan | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [versionSel, setVersionSel] = useState<Record<string, Set<string>>>({});
  const [sizes, setSizes] = useState<Record<string, DirSize | undefined>>({});

  const [opts, setOpts] = useState({
    mods: true,
    config: true,
    saves: false,
    overwrite: false,
    hardlink: false,
  });

  const [running, setRunning] = useState(false);
  const [stage, setStage] = useState<{ index: number; total: number; label: string } | null>(null);
  const [files, setFiles] = useState<{
    done: number;
    total: number;
    bytes_done: number;
    bytes_total: number;
  } | null>(null);
  const [logs, setLogs] = useState<{ level: string; message: string }[]>([]);
  const [reports, setReports] = useState<ImportReport[]>([]);
  const [cancelled, setCancelled] = useState(false);
  const cancelRef = useRef(false);

  // ------------------------------------------------------------ 扫描

  const runScan = useCallback(async () => {
    setScanning(true);
    setError(null);
    try {
      const result = await importApi.scan();
      setScan(result);
      setSelected(new Set());
      setVersionSel({});
      setSizes({});
      // 体积懒加载：逐项补，界面先出列表
      for (const item of result.items) {
        importApi
          .measure(item)
          .then((size) => setSizes((prev) => ({ ...prev, [item.key]: size })))
          .catch(() => undefined);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setScan(null);
    } finally {
      setScanning(false);
    }
  }, []);

  useEffect(() => {
    void runScan();
  }, [runScan]);

  // ------------------------------------------------------------ 选择

  const versionsOf = useCallback(
    (item: ImportItem): Set<string> =>
      versionSel[item.key] ??
      new Set(item.versions.filter((v) => v.importable).map((v) => v.id)),
    [versionSel],
  );

  function toggleItem(key: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function toggleVersion(item: ImportItem, id: string) {
    setVersionSel((prev) => {
      const current =
        prev[item.key] ?? new Set(item.versions.filter((v) => v.importable).map((v) => v.id));
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return { ...prev, [item.key]: next };
    });
  }

  const chosen = useMemo(
    () => (scan?.items ?? []).filter((i) => selected.has(i.key)),
    [scan, selected],
  );

  // ------------------------------------------------------------ 导入

  function applyProgress(p: ImportProgress) {
    if (p.type === 'stage') {
      setStage({ index: p.index, total: p.total, label: p.label });
      setFiles(null);
    } else if (p.type === 'files') {
      setFiles({
        done: p.done,
        total: p.total,
        bytes_done: p.bytes_done,
        bytes_total: p.bytes_total,
      });
    } else {
      setLogs((prev) => [...prev.slice(-60), { level: p.level, message: p.message }]);
    }
  }

  function runOne(req: ImportRequest): Promise<ImportReport | null> {
    return new Promise((resolve, reject) => {
      importApi
        .start(req, (ev) => {
          if (ev.type === 'progress') applyProgress(ev.progress);
          else if (ev.type === 'done') resolve(ev.report);
          else if (ev.type === 'failed') reject(new Error(ev.message));
          else {
            cancelRef.current = true;
            setCancelled(true);
            resolve(null);
          }
        })
        .catch(reject);
    });
  }

  async function runImport() {
    if (!gameDir) {
      setError('先选一个目标游戏目录。');
      return;
    }
    if (chosen.length === 0) {
      setError('先勾选要导入的实例。');
      return;
    }
    setRunning(true);
    setError(null);
    setReports([]);
    setLogs([]);
    setStage(null);
    setFiles(null);
    setCancelled(false);
    cancelRef.current = false;

    const acc: ImportReport[] = [];
    try {
      for (const item of chosen) {
        if (cancelRef.current) break;
        const req: ImportRequest = {
          ...makeImportRequest(item.path, gameDir, [...versionsOf(item)]),
          include_mods: opts.mods,
          include_config: opts.config,
          include_saves: opts.saves,
          overwrite: opts.overwrite,
          hardlink: opts.hardlink,
        };
        const report = await runOne(req);
        if (!report) break;
        acc.push(report);
        setReports([...acc]);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunning(false);
      importApi.releaseChannel();
    }
  }

  async function cancel() {
    cancelRef.current = true;
    try {
      await importApi.cancel();
    } catch {
      // 取消失败不影响界面：引擎那边会在下一个文件边界停下
    }
  }

  // ------------------------------------------------------------ 汇总

  const summary = useMemo(() => {
    if (reports.length === 0) return null;
    return reports.reduce(
      (acc, r) => ({
        files: acc.files + r.files,
        bytes: acc.bytes + r.bytes,
        versions: [...acc.versions, ...r.versions],
        skipped: [...acc.skipped, ...r.skipped_versions],
        links: acc.links + r.links,
        failed: acc.failed + r.failed,
        duration: acc.duration + r.duration_ms,
      }),
      {
        files: 0,
        bytes: 0,
        versions: [] as string[],
        skipped: [] as string[],
        links: 0,
        failed: 0,
        duration: 0,
      },
    );
  }, [reports]);

  const ratio =
    files && files.bytes_total > 0
      ? Math.max(0, Math.min(1, files.bytes_done / files.bytes_total))
      : null;

  const versionCount = scan?.items.reduce((n, i) => n + i.versions.length, 0) ?? 0;
  const foundLaunchers = useMemo(() => {
    const set = new Set<string>();
    for (const p of scan?.probes ?? []) if (p.found) set.add(LAUNCHER_LABEL[p.launcher]);
    return [...set];
  }, [scan]);

  // ------------------------------------------------------------ 视图

  return (
    <div className="wrap">
      <header className="ef-head">
        <h1>
          导入 <small>Import</small>
        </h1>
        <p>
          把本机其它启动器里已经装好的版本搬过来。扫描只读配置和目录名，不动源目录；
          导入时同名版本会跳过，不覆盖。
        </p>
      </header>

      {!IS_TAURI && (
        <p className="ef-notice" data-flag="演示数据">
          浏览器预览：列表和进度都来自内置的假数据，不是本机磁盘。装成桌面应用后，
          这里显示的是真实的启动器目录。
        </p>
      )}

      {error && (
        <p className="ef-notice" data-flag="出错">
          {error}
        </p>
      )}

      <section className="ef-panel" aria-labelledby="target-h">
        <h2 id="target-h">
          目标目录 <small>Target</small>
        </h2>
        <DirPicker value={gameDir} dirs={dirs} onChange={onChangeDir} label="目标游戏目录" />
        <div className="ef-actions" style={{ marginTop: '0.7rem' }}>
          <button
            className="ef-btn"
            type="button"
            onClick={() => void runScan()}
            disabled={scanning || running}
          >
            {scanning ? '正在扫描…' : '扫描本机启动器'}
          </button>
          <button className="ef-btn small" type="button" onClick={onOpenSettings}>
            管理目录
          </button>
        </div>
        {scan && (
          <p style={{ ...S.sub, margin: '0.6rem 0 0' }}>
            扫了 {scan.scanned} 个候选路径，用时 {scan.duration_ms} ms
            {foundLaunchers.length > 0 ? ` │ 找到：${foundLaunchers.join('、')}` : ' │ 一个也没找到'}
          </p>
        )}
      </section>

      <section className="ef-panel" aria-labelledby="found-h">
        <h2 id="found-h">
          找到的实例{' '}
          <small>
            {scan === null ? '扫描中' : `${scan.items.length} 项 · ${versionCount} 个版本`}
          </small>
        </h2>

        {scan !== null && scan.items.length === 0 && (
          <p className="ef-empty">
            本机没有扫到可以导入的东西。常见原因：这台机器上只装过 MCDL 自己，或者其它启动器把
            游戏目录放在了非默认位置。下面的「扫描明细」会逐条说明每个位置为什么没找到。
          </p>
        )}

        {scan !== null && scan.items.length > 0 && (
          <ul className="ef-ledger">
            {scan.items.map((item) => {
              const on = selected.has(item.key);
              const picked = versionsOf(item);
              const size = sizes[item.key];
              const importable = item.versions.filter((v) => v.importable);
              return (
                <li key={item.key}>
                  <div style={S.row}>
                    <div style={S.head}>
                      <button
                        className="ef-chip"
                        type="button"
                        aria-pressed={on}
                        onClick={() => toggleItem(item.key)}
                      >
                        {on ? '已选' : '选择'}
                      </button>
                      <span className="version-id">{item.name}</span>
                      <span className="ef-tag">{ITEM_KIND_LABEL[item.kind]}</span>
                      {item.launchers.map((l) => (
                        <span className="ef-chip" key={l}>
                          {LAUNCHER_LABEL[l]}
                        </span>
                      ))}
                      <span style={S.sub}>
                        {size === undefined
                          ? '体积统计中…'
                          : `${formatCount(size.files)} 个文件 │ ${formatBytes(size.bytes)}`}
                      </span>
                    </div>

                    <div className="path" style={S.sub} title={item.path}>
                      {item.path}
                    </div>

                    {item.versions.length > 0 && (
                      <div style={S.chips}>
                        {item.versions.map((v) => {
                          const checked = picked.has(v.id);
                          return (
                            <button
                              key={v.id}
                              className="ef-chip"
                              type="button"
                              aria-pressed={checked}
                              disabled={!v.importable}
                              title={
                                v.importable
                                  ? '这个版本目录会被复制过去'
                                  : '实例没有独立的 versions 目录，版本要在「安装」页重装'
                              }
                              onClick={() => {
                                // 点版本就算选中这一项：少一次点击
                                if (!on) toggleItem(item.key);
                                toggleVersion(item, v.id);
                              }}
                            >
                              {v.id}
                              <span className="n">{v.loader ?? '原版'}</span>
                            </button>
                          );
                        })}
                      </div>
                    )}

                    {importable.length === 0 && item.kind !== 'game_dir' && (
                      <p style={{ ...S.sub, margin: 0 }}>
                        实例目录里没有独立的版本目录，导入只带 mods / config；版本本身要用「安装」页重装。
                      </p>
                    )}

                    {item.evidence.length > 0 && (
                      <ul className="ef-ledger" style={{ margin: 0 }}>
                        {item.evidence.map((e) => (
                          <li key={e} style={{ ...S.sub, borderBottom: 'none', padding: '0.15rem 0' }}>
                            {e}
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <section className="ef-panel" aria-labelledby="opts-h">
        <h2 id="opts-h">
          导入内容 <small>Options</small>
        </h2>
        <div style={S.options}>
          <label className="ef-setting">
            <span>
              <b>带上 mods</b>
              <small>复制源目录的 mods/</small>
            </span>
            <input
              className="ef-switch"
              type="checkbox"
              checked={opts.mods}
              disabled={running}
              onChange={(e) => setOpts({ ...opts, mods: e.target.checked })}
            />
          </label>
          <label className="ef-setting">
            <span>
              <b>带上 config</b>
              <small>复制源目录的 config/（模组配置）</small>
            </span>
            <input
              className="ef-switch"
              type="checkbox"
              checked={opts.config}
              disabled={running}
              onChange={(e) => setOpts({ ...opts, config: e.target.checked })}
            />
          </label>
          <label className="ef-setting">
            <span>
              <b>带上存档</b>
              <small>复制源目录的 saves/（存档体积通常很大）</small>
            </span>
            <input
              className="ef-switch"
              type="checkbox"
              checked={opts.saves}
              disabled={running}
              onChange={(e) => setOpts({ ...opts, saves: e.target.checked })}
            />
          </label>
          <label className="ef-setting">
            <span>
              <b>覆盖目标里的同名文件</b>
              <small>默认关：目标已有同名版本就跳过，源目录永远不改</small>
            </span>
            <input
              className="ef-switch"
              type="checkbox"
              checked={opts.overwrite}
              disabled={running}
              onChange={(e) => setOpts({ ...opts, overwrite: e.target.checked })}
            />
          </label>
          <label className="ef-setting">
            <span>
              <b>同盘用硬链</b>
              <small>省空间；但之后改目标里的文件会同时改到源目录</small>
            </span>
            <input
              className="ef-switch"
              type="checkbox"
              checked={opts.hardlink}
              disabled={running}
              onChange={(e) => setOpts({ ...opts, hardlink: e.target.checked })}
            />
          </label>
        </div>

        <div className="ef-actions" style={{ marginTop: '0.9rem' }}>
          <button
            className="ef-btn go"
            type="button"
            onClick={() => void runImport()}
            disabled={running || chosen.length === 0 || !gameDir}
          >
            导入选中项
            {chosen.length > 0 && <span className="sub">{chosen.length} 项</span>}
          </button>
          <button
            className="ef-btn small"
            type="button"
            onClick={() => void cancel()}
            disabled={!running}
          >
            取消
          </button>
        </div>
      </section>

      <section className="ef-panel" aria-labelledby="run-h">
        <h2 id="run-h">
          进度 <small>Progress</small>
        </h2>

        <div
          className={`ef-progress${ratio === null ? ' is-unknown' : ''}`}
          role="progressbar"
          aria-label="导入进度"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={ratio === null ? undefined : Math.round(ratio * 100)}
          style={ratio === null ? undefined : ({ '--p': ratio } as CSSProperties)}
        >
          <span className="ef-eyebrow">
            {running
              ? stage
                ? `${pad2(stage.index)} / ${pad2(stage.total)} · ${stage.label}`
                : '准备中'
              : summary
                ? '已完成'
                : '还没开始'}
          </span>
          <span className="ef-field">{ratio === null ? '——' : padPercent(ratio)}</span>
          <i />
        </div>

        <ul className="ef-ledger" style={{ marginTop: '0.8rem' }}>
          <li>
            <span>已处理文件</span>
            <span className="ef-field">
              {files ? `${formatCount(files.done)} / ${formatCount(files.total)}` : '—'}
            </span>
          </li>
          <li>
            <span>已传输</span>
            <span className="ef-field">
              {files
                ? `${formatBytes(files.bytes_done)} / ${formatBytes(files.bytes_total)}`
                : '—'}
            </span>
          </li>
          <li>
            <span>状态</span>
            <span className="ef-field">
              {running ? '导入中' : cancelled ? '已取消' : summary ? '完成' : '空闲'}
            </span>
          </li>
        </ul>

        {logs.length > 0 && (
          <ul style={S.logs}>
            {logs.map((l, i) => (
              <li key={`${i}-${l.message}`}>
                <span style={{ color: 'var(--ef-ink)' }}>[{l.level}]</span> {l.message}
              </li>
            ))}
          </ul>
        )}

        {summary && (
          <>
            <ul className="ef-ledger" style={{ marginTop: '0.8rem' }}>
              <li>
                <span>导入的版本</span>
                <span className="ef-field">
                  {summary.versions.length > 0 ? summary.versions.join('、') : '无'}
                </span>
              </li>
              <li>
                <span>新增文件</span>
                <span className="ef-field">
                  {formatCount(summary.files)} 个 │ {formatBytes(summary.bytes)}
                </span>
              </li>
              <li>
                <span>跳过（已存在）</span>
                <span className="ef-field">
                  {summary.skipped.length > 0 ? summary.skipped.join('、') : '无'}
                </span>
              </li>
              <li>
                <span>硬链 / 失败</span>
                <span className="ef-field">
                  {summary.links} / {summary.failed}
                </span>
              </li>
              <li>
                <span>用时</span>
                <span className="ef-field">{formatDuration(summary.duration)}</span>
              </li>
            </ul>
            {summary.skipped.length > 0 && (
              <p className="ef-notice" data-flag="已存在" style={{ marginTop: '0.8rem' }}>
                目标目录里已经有这些版本，按规则跳过、没有覆盖：
                {summary.skipped.join('、')}。确实要替换的话，勾上「覆盖目标里的同名文件」再导一次。
              </p>
            )}
            {summary.failed > 0 && (
              <p className="ef-notice" data-flag="有失败" style={{ marginTop: '0.8rem' }}>
                有 {summary.failed} 个文件没复制成功（多半是被占用或权限不足），其余文件已经就位。
              </p>
            )}
          </>
        )}
      </section>

      <section className="ef-panel" aria-labelledby="probe-h">
        <h2 id="probe-h">
          扫描明细 <small>Probes</small>
        </h2>
        <p style={{ ...S.sub, margin: '0 0 0.6rem' }}>
          每一个找过的位置都在这里：找到了写证据，没找到写原因。
        </p>
        {scan === null && <p className="ef-empty">还没有扫描结果。</p>}
        {scan !== null && (
          <div>
            {scan.probes.map((p) => (
              <div key={`${p.launcher}-${p.path}`} style={S.probe}>
                <span className="ef-tag" style={p.found ? undefined : { opacity: 0.55 }}>
                  {p.found ? '命中' : '没有'}
                </span>
                <div style={{ minWidth: 0 }}>
                  <div className="path" style={{ ...S.sub, color: 'var(--ef-ink)' }} title={p.path}>
                    {LAUNCHER_LABEL[p.launcher]} │ {p.path}
                  </div>
                  <div style={S.sub}>{p.note}</div>
                </div>
              </div>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
