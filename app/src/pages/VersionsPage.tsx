// 版本页：这个游戏目录里已经装了什么。
//
// 数据全部来自磁盘扫描（`installed::list`），不是我们自己的记账——
// 别的启动器装的版本也会出现在这里，因为那本来就是同一个 `versions/` 目录。

import { useCallback, useEffect, useState } from 'react';

import { T } from '../i18n';
import { api, IS_TAURI } from '../lib/api';
import { DirPicker } from '../components/DirPicker';
import { formatBytes, formatCount, formatDate } from '../lib/format';
import { ISOLATION_NOTICE, modsDir, modsLabel } from '../lib/isolate';
import { loadSettings } from '../lib/settings';
import type { InstalledVersion } from '../lib/types';

interface Props {
  gameDir: string;
  dirs: string[];
  onChangeDir: (dir: string) => void;
  onOpenSettings: () => void;
}

export function VersionsPage({ gameDir, dirs, onChangeDir, onOpenSettings }: Props) {
  const [items, setItems] = useState<InstalledVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

  // 版本隔离是全局设置：这里只**显示**它把模组放在哪，不提供自动迁移
  // （把根 mods/ 里属于某个版本的模组猜出来再搬走，误伤概率太高）。
  const [isolated] = useState(() => loadSettings().isolated);

  /** 某个版本的模组落点；名字异常时返回 null，界面就不显示这一行。 */
  function modsTargetOf(id: string): string | null {
    if (!gameDir) return null;
    try {
      return modsLabel(modsDir(gameDir, id, isolated));
    } catch {
      return null;
    }
  }

  const reload = useCallback(() => {
    if (!gameDir) {
      setItems([]);
      return;
    }
    setError(null);
    api
      .listInstalled(gameDir)
      .then(setItems)
      .catch((e: unknown) => {
        setItems([]);
        setError(e instanceof Error ? e.message : String(e));
      });
  }, [gameDir]);

  useEffect(reload, [reload]);

  async function remove(id: string) {
    setBusy(id);
    setError(null);
    try {
      await api.removeVersion(gameDir, id);
      setConfirming(null);
      reload();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const total = items?.reduce((sum, v) => sum + v.bytes, 0) ?? 0;
  const loaderCount = items?.filter((v) => v.loader).length ?? 0;

  return (
    <div className="wrap">
      <header className="ef-head">
        <h1>
          {T.VersionsPage.title} <small>Versions</small>
        </h1>
      </header>

      <div className="versions-bar">
        <DirPicker value={gameDir} dirs={dirs} onChange={onChangeDir} label={T.common.gameDir} />
        <button className="ef-btn small" type="button" onClick={reload}>
          {T.VersionsPage.rescan}
        </button>
        <button className="ef-btn small" type="button" onClick={onOpenSettings}>
          {T.common.manageDirs}
        </button>
      </div>

      {error && (
        <p className="ef-notice" data-flag={T.VersionsPage.errorFlag}>
          {error}
        </p>
      )}

      <section className="ef-panel">
        <h2>
          {T.VersionsPage.installedHeading}{' '}
          <small>
            {items === null
              ? T.VersionsPage.scanning
              : T.VersionsPage.count(items.length, formatBytes(total))}
          </small>
        </h2>

        {items !== null && items.length === 0 && (
          <p className="ef-empty">
            {T.VersionsPage.empty}
          </p>
        )}

        {items !== null && items.length > 0 && (
          <ul className="ef-ledger version-list">
            {items.map((v) => (
              <li key={v.id}>
                <div className="version-main">
                  <span className="version-id">{v.id}</span>
                  <span className="ef-chip">{v.loader ?? T.common.vanilla}</span>
                  <span className="ef-chip" title={ISOLATION_NOTICE}>
                    {isolated ? T.common.isolated : T.common.shared}
                  </span>
                  <span className="version-meta">
                    {T.VersionsPage.versionMeta(
                      v.kind,
                      formatCount(v.files),
                      formatBytes(v.bytes),
                      v.modified ? formatDate(new Date(v.modified * 1000).toISOString()) : '—',
                    )}
                    {modsTargetOf(v.id) && T.VersionsPage.modsSuffix(modsTargetOf(v.id) ?? '')}
                  </span>
                </div>

                <div className="version-actions">
                  {confirming === v.id ? (
                    <>
                      <span className="version-warn">{T.VersionsPage.deleteAsk}</span>
                      <button
                        className="ef-btn small danger"
                        type="button"
                        onClick={() => remove(v.id)}
                        disabled={busy === v.id}
                      >
                        {busy === v.id ? T.VersionsPage.deleting : T.VersionsPage.confirmDelete}
                      </button>
                      <button
                        className="ef-btn small"
                        type="button"
                        onClick={() => setConfirming(null)}
                      >
                        {T.common.cancel}
                      </button>
                    </>
                  ) : (
                    <button
                      className="ef-btn small"
                      type="button"
                      onClick={() => setConfirming(v.id)}
                    >
                      {T.VersionsPage.delete}
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}

        {!IS_TAURI && (
          <p className="ef-note">
            {T.VersionsPage.demoNote}
          </p>
        )}
      </section>

      <section className="ef-panel">
        <h2>
          {T.VersionsPage.isolationHeading} <small>{isolated ? T.common.on : T.common.off}</small>
        </h2>

        <p className="ef-notice" data-flag={T.VersionsPage.noticeFlag}>
          {ISOLATION_NOTICE}
        </p>

        <p className="ef-lede ef-read">
          {T.VersionsPage.modsTargetLead}{' '}
          <code>{isolated ? T.VersionsPage.modsPathIsolated : T.VersionsPage.modsPathShared}</code>
          {isolated ? T.VersionsPage.targetIsolated : T.VersionsPage.targetShared}
        </p>

        <p className="ef-lede ef-read">
          {T.VersionsPage.noMigration}
        </p>

        <ul className="ef-ledger">
          <li>
            <span>{T.VersionsPage.manualMigrate}</span>
            <span className="path">{T.VersionsPage.manualMigrateValue}</span>
          </li>
          <li>
            <span>{T.VersionsPage.whoReads}</span>
            <span>{T.VersionsPage.whoReadsValue}</span>
          </li>
          <li>
            <span>{T.VersionsPage.officialLauncher}</span>
            <span>{T.VersionsPage.officialReads}</span>
          </li>
        </ul>
      </section>

      <section className="ef-panel">
        <h2>
          {T.VersionsPage.summaryHeading} <small>Summary</small>
        </h2>
        <ul className="ef-ledger">
          <li>
            <span>{T.VersionsPage.versionCount}</span>
            <span>{items?.length ?? '—'}</span>
          </li>
          <li>
            <span>{T.VersionsPage.withLoader}</span>
            <span>{loaderCount}</span>
          </li>
          <li>
            <span>{T.VersionsPage.size}</span>
            <span>{formatBytes(total)}</span>
          </li>
        </ul>
      </section>
    </div>
  );
}
