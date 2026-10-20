// 版本页：这个游戏目录里已经装了什么。
//
// 数据全部来自磁盘扫描（`installed::list`），不是我们自己的记账——
// 别的启动器装的版本也会出现在这里，因为那本来就是同一个 `versions/` 目录。

import { useCallback, useEffect, useState } from 'react';

import { api, IS_TAURI } from '../lib/api';
import { formatBytes, formatDate } from '../lib/format';
import type { InstalledVersion } from '../lib/types';

interface Props {
  gameDir: string;
  onOpenSettings: () => void;
}

export function VersionsPage({ gameDir, onOpenSettings }: Props) {
  const [items, setItems] = useState<InstalledVersion[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

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
        <p>
          这个目录里已经装好的东西。删一个版本只删它自己的目录，共享的库和资源不动。
        </p>
      </header>

      <section className="ef-panel">
        <h2>
          当前目录 <small>Target</small>
        </h2>
        <p className="path" title={gameDir}>
          {gameDir || '还没有设置游戏目录'}
        </p>
        <div className="ef-actions">
          <button className="ef-btn small" type="button" onClick={reload}>
            重新扫描
          </button>
          <button className="ef-btn small" type="button" onClick={onOpenSettings}>
            换目录
          </button>
        </div>
      </section>

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
                  <span className="version-meta">
                    {v.kind} │ {v.files} 个文件 │ {formatBytes(v.bytes)} │{' '}
                    {v.modified
                      ? formatDate(new Date(v.modified * 1000).toISOString())
                      : '—'}
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
