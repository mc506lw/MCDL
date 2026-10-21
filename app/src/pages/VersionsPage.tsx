// 版本页：这个游戏目录里已经装了什么。
//
// 数据全部来自磁盘扫描（`installed::list`），不是我们自己的记账——
// 别的启动器装的版本也会出现在这里，因为那本来就是同一个 `versions/` 目录。

import { useCallback, useEffect, useState } from 'react';

import { api, IS_TAURI } from '../lib/api';
import { DirPicker } from '../components/DirPicker';
import { formatBytes, formatDate } from '../lib/format';
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
          版本 <small>Versions</small>
        </h1>
      </header>

      <div className="versions-bar">
        <DirPicker value={gameDir} dirs={dirs} onChange={onChangeDir} label="游戏目录" />
        <button className="ef-btn small" type="button" onClick={reload}>
          重新扫描
        </button>
        <button className="ef-btn small" type="button" onClick={onOpenSettings}>
          管理目录
        </button>
      </div>

      {error && (
        <p className="ef-notice" data-flag="出错">
          {error}
        </p>
      )}

      <section className="ef-panel">
        <h2>
          已装版本{' '}
          <small>
            {items === null ? '扫描中' : `${items.length} 个 · ${formatBytes(total)}`}
          </small>
        </h2>

        {items !== null && items.length === 0 && (
          <p className="ef-empty">
            这个目录里还没有任何版本。回到「安装」页选一个版本开始，装完之后它会出现在这里。
          </p>
        )}

        {items !== null && items.length > 0 && (
          <ul className="ef-ledger version-list">
            {items.map((v) => (
              <li key={v.id}>
                <div className="version-main">
                  <span className="version-id">{v.id}</span>
                  <span className="ef-chip">{v.loader ?? '原版'}</span>
                  <span className="ef-chip" title={ISOLATION_NOTICE}>
                    {isolated ? '隔离' : '共享'}
                  </span>
                  <span className="version-meta">
                    {v.kind} │ {v.files} 个文件 │ {formatBytes(v.bytes)} │{' '}
                    {v.modified
                      ? formatDate(new Date(v.modified * 1000).toISOString())
                      : '—'}
                    {modsTargetOf(v.id) && <> │ 模组 {modsTargetOf(v.id)}</>}
                  </span>
                </div>

                <div className="version-actions">
                  {confirming === v.id ? (
                    <>
                      <span className="version-warn">删掉？</span>
                      <button
                        className="ef-btn small danger"
                        type="button"
                        onClick={() => remove(v.id)}
                        disabled={busy === v.id}
                      >
                        {busy === v.id ? '删除中…' : '确认删除'}
                      </button>
                      <button
                        className="ef-btn small"
                        type="button"
                        onClick={() => setConfirming(null)}
                      >
                        取消
                      </button>
                    </>
                  ) : (
                    <button
                      className="ef-btn small"
                      type="button"
                      onClick={() => setConfirming(v.id)}
                    >
                      删除
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}

        {!IS_TAURI && (
          <p className="ef-note">
            浏览器预览用的是演示数据；装成桌面应用后这里显示真实磁盘内容。
          </p>
        )}
      </section>

      <section className="ef-panel">
        <h2>
          版本隔离 <small>{isolated ? '开' : '关'}</small>
        </h2>

        <p className="ef-notice" data-flag="给谁用">
          {ISOLATION_NOTICE}
        </p>

        <p className="ef-lede ef-read">
          当前设置下，新装的模组会写进{' '}
          <code>{isolated ? 'versions/<版本id>/mods/' : 'mods/'}</code>
          {isolated
            ? '。每个版本一份，同目录装第二个整合包不会混。'
            : '。所有版本共用同一份，装第二个整合包会和第一个混在一起。'}
        </p>

        <p className="ef-lede ef-read">
          这里不提供「把某个版本改成隔离」的自动迁移：根 <code>mods/</code>{' '}
          里的文件属于哪个版本只能靠猜，搬错会静默弄坏别的实例。要换落点，重新装一次那个
          整合包，或者按下面手动搬。
        </p>

        <ul className="ef-ledger">
          <li>
            <span>手动迁移（推荐用支持隔离的启动器操作）</span>
            <span className="path">
              根 mods/ 里该版本专属的 jar → versions/&lt;版本id&gt;/mods/
            </span>
          </li>
          <li>
            <span>隔离后的模组谁能读</span>
            <span>PCL / HMCL 等支持版本隔离的启动器</span>
          </li>
          <li>
            <span>官方启动器</span>
            <span>只读根目录的 mods/</span>
          </li>
        </ul>
      </section>

      <section className="ef-panel">
        <h2>
          统计 <small>Summary</small>
        </h2>
        <ul className="ef-ledger">
          <li>
            <span>版本数</span>
            <span>{items?.length ?? '—'}</span>
          </li>
          <li>
            <span>带加载器</span>
            <span>{loaderCount}</span>
          </li>
          <li>
            <span>占用</span>
            <span>{formatBytes(total)}</span>
          </li>
        </ul>
      </section>
    </div>
  );
}
