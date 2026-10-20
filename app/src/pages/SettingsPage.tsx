import { useCallback, useEffect, useState } from 'react';

import { api } from '../lib/api';
import {
  DEFAULT_SETTINGS,
  clampConcurrency,
  forgetDir,
  rememberDir,
  type Settings,
} from '../lib/settings';
import type { JavaRuntime } from '../lib/types';

interface Props {
  settings: Settings;
  defaultDir: string;
  java: JavaRuntime[] | null;
  javaLoading: boolean;
  javaError: string | null;
  onRefreshJava: () => void;
  onSave: (settings: Settings) => void;
  onReset: () => void;
  onOpenDir: (path: string) => void;
}

/**
 * 设置页：阅读类型。一栏到底，细线分区，不堆卡片。
 * 每个设置项是「名称 + 一行说明」在左、控件在右。
 */
export function SettingsPage({
  settings,
  defaultDir,
  java,
  javaLoading,
  javaError,
  onRefreshJava,
  onSave,
  onReset,
  onOpenDir,
}: Props) {
  const [draft, setDraft] = useState<Settings>(settings);
  const [runtimes, setRuntimes] = useState<string[] | null>(null);

  useEffect(() => setDraft(settings), [settings]);

  // 游戏目录下已经落盘的 JRE：runtime/jre-<major>/bin/java.exe
  const dir = draft.gameDir || defaultDir;
  const loadRuntimes = useCallback(async () => {
    try {
      setRuntimes(await api.listRuntimes(dir));
    } catch {
      setRuntimes([]);
    }
  }, [dir]);

  useEffect(() => {
    void loadRuntimes();
  }, [loadRuntimes]);

  const patch = (next: Partial<Settings>) => setDraft((old) => ({ ...old, ...next }));
  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  return (
    <div className="wrap narrow">
      <header className="ef-sheet-head">
        <h1>
          设置 <small>Settings</small>
        </h1>
        <p className="ef-field">只影响这台机器</p>
      </header>

      <p className="ef-lede ef-read">
        这里的值会成为下一次安装的默认值。已经装好的实例不受影响。
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Downloads</span>
          <span>下载</span>
        </span>
        <span className="ef-field">默认 {DEFAULT_SETTINGS.concurrency}</span>
      </h2>

      <div className="settings-block">
        <label className="ef-setting">
          <span>
            <b>并发下载数</b>
            <small>同时下几个文件，1 到 64。太高会被服务器限速，反而更慢。</small>
          </span>
          <span className="ctl num">
            <input
              type="number"
              min={1}
              max={64}
              value={draft.concurrency}
              onChange={(event) => patch({ concurrency: clampConcurrency(event.target.value) })}
            />
          </span>
        </label>

        <label className="ef-setting">
          <span>
            <b>默认跳过资源文件</b>
            <small>只装能启动的最小集合。装完能进游戏，但会缺音效和语言文件。</small>
          </span>
          <input
            className="ef-switch"
            type="checkbox"
            role="switch"
            checked={draft.skipAssets}
            onChange={(event) => patch({ skipAssets: event.target.checked })}
          />
        </label>
      </div>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Directories</span>
          <span>目录与运行时</span>
        </span>
      </h2>

      <div className="settings-block">
        <div className="ef-setting">
          <span>
            <b>默认游戏目录</b>
            <small>.minecraft 所在目录。留空则用系统默认的那个。</small>
          </span>
          <span className="ctl wide">
            <input
              type="text"
              value={draft.gameDir}
              placeholder={defaultDir || '系统默认目录'}
              onChange={(event) => patch({ gameDir: event.target.value })}
              spellCheck={false}
            />
            <button
              className="ef-btn small"
              type="button"
              onClick={() => patch({ gameDir: defaultDir })}
              disabled={!defaultDir}
            >
              默认
            </button>
          </span>
        </div>

        <div className="ef-setting">
          <span>
            <b>Java 路径</b>
            <small>指定 javaw.exe 或 java。留空则按版本需要自动探测，探测不到再取一份。</small>
          </span>
          <span className="ctl wide">
            <input
              type="text"
              value={draft.javaPath}
              placeholder="自动探测"
              onChange={(event) => patch({ javaPath: event.target.value })}
              spellCheck={false}
            />
          </span>
        </div>

        <p className="ef-notice" data-flag="第三方镜像">
          Java 运行时：官方 Adoptium 下载失败时，会改用清华镜像（第三方）获取同一份 Temurin
          构建。
        </p>
      </div>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Runtimes</span>
          <span>本机 Java</span>
        </span>
        <span className="ef-field">{java ? `${java.length} 个` : '探测中'}</span>
      </h2>

      {javaLoading && <span className="ef-skeleton line" aria-label="正在探测本机 Java" />}

      {javaError && (
        <p className="ef-notice" data-flag="探测失败">
          {javaError}
        </p>
      )}

      {java && java.length === 0 && !javaLoading && (
        <p className="ef-empty">
          本机没找到 Java。安装时会按版本需要从 Adoptium 取一份，放在游戏目录的 runtime 下。
        </p>
      )}

      {java && java.length > 0 && (
        <ul className="ef-ledger">
          {java.map((rt) => (
            <li key={rt.path}>
              <span title={rt.path}>
                Java {rt.major} · {rt.vendor} · {rt.arch}
              </span>
              <span className="ef-field">{rt.version}</span>
            </li>
          ))}
        </ul>
      )}

      <p className="ef-actions" style={{ marginTop: '1rem' }}>
        <button
          className="ef-btn small"
          type="button"
          onClick={() => {
            onRefreshJava();
            void loadRuntimes();
          }}
        >
          重新探测
        </button>
        <button
          className="ef-btn small"
          type="button"
          onClick={() => onOpenDir(draft.gameDir || defaultDir)}
          disabled={!draft.gameDir && !defaultDir}
        >
          打开目录
        </button>
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Downloaded</span>
          <span>已下载的运行时</span>
        </span>
        <span className="ef-field">{runtimes ? `${runtimes.length} 个` : '读取中'}</span>
      </h2>

      <p className="ef-lede ef-read">
        按版本需要下载的 Java 会落在游戏目录的 <code>runtime/jre-&lt;主版本&gt;/</code> 下，
        和手工指定的 Java 分开管理。删掉这几个目录就等于让下次安装重新取一份。
      </p>

      {runtimes && runtimes.length === 0 && (
        <p className="ef-empty">
          游戏目录里还没有下载过运行时。需要的话，安装时它会自己取一份放在这里。
        </p>
      )}

      {runtimes && runtimes.length > 0 && (
        <ul className="ef-ledger">
          {runtimes.map((path) => (
            <li key={path}>
              <span className="path" title={path}>
                {path}
              </span>
            </li>
          ))}
        </ul>
      )}

      <div className="ef-group-head">
        <span>常用目录</span>
        <small>Dirs</small>
      </div>

      {settings.dirs.length === 0 ? (
        <p className="ef-empty">
          还没有记录任何目录。安装一次之后，用过的目录会自动出现在这里，
          安装页的目录输入框也能直接选。
        </p>
      ) : (
        <ul className="ef-ledger">
          {settings.dirs.map((dir) => (
            <li key={dir}>
              <span className="path" title={dir}>
                {dir}
              </span>
              <span>
                <button
                  className="ef-btn small"
                  type="button"
                  onClick={() => onSave(rememberDir(settings, dir))}
                  disabled={settings.gameDir === dir}
                >
                  {settings.gameDir === dir ? '当前' : '切到这个'}
                </button>{' '}
                <button
                  className="ef-btn small danger"
                  type="button"
                  onClick={() => onSave(forgetDir(settings, dir))}
                >
                  移除
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}

      <p className="ef-actions settings-save">
        <button
          className="ef-btn go"
          type="button"
          onClick={() => onSave(draft)}
          disabled={!dirty}
        >
          保存
        </button>
        <button
          className="ef-btn"
          type="button"
          onClick={() => {
            setDraft(DEFAULT_SETTINGS);
            onSave(DEFAULT_SETTINGS);
          }}
        >
          恢复默认
        </button>
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Danger</span>
          <span>危险操作</span>
        </span>
      </h2>

      <p className="ef-lede ef-read">
        清掉保存在本机的设置，回到出厂默认。已经装好的版本、下载好的文件都不会动。
      </p>

      <p className="ef-actions" style={{ marginTop: '0.9rem' }}>
        <button className="ef-btn danger" type="button" onClick={onReset}>
          清除本机设置
        </button>
      </p>
    </div>
  );
}
