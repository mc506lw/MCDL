import { useCallback, useEffect, useState } from 'react';

import { T } from '../i18n';
import { api } from '../lib/api';
import { ISOLATION_NOTICE } from '../lib/isolate';
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
  /** 主动添加一个游戏目录（不替换现有的）。 */
  async function addDir() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({
        directory: true,
        multiple: false,
        title: T.SettingsPage.addDirTitle,
      });
      if (typeof picked === 'string' && picked) {
        onSave(rememberDir(settings, picked));
      }
    } catch {
      // 用户取消或没有原生对话框：什么也不做
    }
  }

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
          {T.SettingsPage.title} <small>Settings</small>
        </h1>
        <p className="ef-field">{T.SettingsPage.scope}</p>
      </header>

      <p className="ef-lede ef-read">
        {T.SettingsPage.lede}
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Downloads</span>
          <span>{T.SettingsPage.downloadsHeading}</span>
        </span>
        <span className="ef-field">{T.SettingsPage.defaultConcurrency(DEFAULT_SETTINGS.concurrency)}</span>
      </h2>

      <div className="settings-block">
        <label className="ef-setting">
          <span>
            <b>{T.SettingsPage.concurrencyLabel}</b>
            <small>{T.SettingsPage.concurrencyHint}</small>
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
            <b>{T.SettingsPage.skipAssetsLabel}</b>
            <small>{T.SettingsPage.skipAssetsHint}</small>
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
          <span className="ef-eyebrow">Isolation</span>
          <span>{T.SettingsPage.isolationHeading}</span>
        </span>
        <span className="ef-field">{draft.isolated ? T.common.on : T.common.off}</span>
      </h2>

      <div className="settings-block">
        <label className="ef-setting">
          <span>
            <b>{T.SettingsPage.isolationLabel}</b>
            <small>{T.SettingsPage.isolationHint}</small>
          </span>
          <input
            className="ef-switch"
            type="checkbox"
            role="switch"
            checked={draft.isolated}
            onChange={(event) => patch({ isolated: event.target.checked })}
          />
        </label>

        <p className="ef-notice" data-flag={T.SettingsPage.noticeFlag}>
          {ISOLATION_NOTICE}
        </p>

        <p className="ef-lede ef-read">
          {T.SettingsPage.isolationExtra}
        </p>
      </div>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Directories</span>
          <span>{T.SettingsPage.dirsHeading}</span>
        </span>
      </h2>

      <div className="settings-block">
        <div className="ef-setting">
          <span>
            <b>{T.SettingsPage.defaultDirLabel}</b>
            <small>{T.SettingsPage.defaultDirHint}</small>
          </span>
          <span className="ctl wide">
            <input
              type="text"
              value={draft.gameDir}
              placeholder={defaultDir || T.SettingsPage.defaultDirPlaceholder}
              onChange={(event) => patch({ gameDir: event.target.value })}
              spellCheck={false}
            />
            <button
              className="ef-btn small"
              type="button"
              onClick={() => patch({ gameDir: defaultDir })}
              disabled={!defaultDir}
            >
              {T.SettingsPage.defaultBtn}
            </button>
          </span>
        </div>

        <div className="ef-setting">
          <span>
            <b>{T.SettingsPage.javaPathLabel}</b>
            <small>{T.SettingsPage.javaPathHint}</small>
          </span>
          <span className="ctl wide">
            <input
              type="text"
              value={draft.javaPath}
              placeholder={T.SettingsPage.javaPathPlaceholder}
              onChange={(event) => patch({ javaPath: event.target.value })}
              spellCheck={false}
            />
          </span>
        </div>

        <p className="ef-notice" data-flag={T.SettingsPage.mirrorFlag}>
          {T.SettingsPage.mirrorNote}
        </p>
      </div>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Runtimes</span>
          <span>{T.SettingsPage.javaHeading}</span>
        </span>
        <span className="ef-field">
          {java ? T.SettingsPage.javaCount(java.length) : T.SettingsPage.probing}
        </span>
      </h2>

      {javaLoading && <span className="ef-skeleton line" aria-label={T.SettingsPage.javaLoading} />}

      {javaError && (
        <p className="ef-notice" data-flag={T.SettingsPage.javaErrorFlag}>
          {javaError}
        </p>
      )}

      {java && java.length === 0 && !javaLoading && (
        <p className="ef-empty">
          {T.SettingsPage.noJava}
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
          {T.SettingsPage.refresh}
        </button>
        <button
          className="ef-btn small"
          type="button"
          onClick={() => onOpenDir(draft.gameDir || defaultDir)}
          disabled={!draft.gameDir && !defaultDir}
        >
          {T.common.openDir}
        </button>
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Downloaded</span>
          <span>{T.SettingsPage.downloadedHeading}</span>
        </span>
        <span className="ef-field">
          {runtimes ? T.SettingsPage.runtimesCount(runtimes.length) : T.SettingsPage.reading}
        </span>
      </h2>

      <p className="ef-lede ef-read">
        {T.SettingsPage.runtimesLede}
      </p>

      {runtimes && runtimes.length === 0 && (
        <p className="ef-empty">
          {T.SettingsPage.runtimesEmpty}
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
        <span>{T.SettingsPage.commonDirs}</span>
        <span>
          <button className="ef-btn small" type="button" onClick={addDir}>
            {T.SettingsPage.addDir}
          </button>{' '}
          <small>Dirs</small>
        </span>
      </div>

      {settings.dirs.length === 0 ? (
        <p className="ef-empty">
          {T.SettingsPage.dirsEmpty}
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
                  {settings.gameDir === dir ? T.SettingsPage.current : T.SettingsPage.switchTo}
                </button>{' '}
                <button
                  className="ef-btn small danger"
                  type="button"
                  onClick={() => onSave(forgetDir(settings, dir))}
                >
                  {T.SettingsPage.remove}
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
          {T.SettingsPage.save}
        </button>
        <button
          className="ef-btn"
          type="button"
          onClick={() => {
            setDraft(DEFAULT_SETTINGS);
            onSave(DEFAULT_SETTINGS);
          }}
        >
          {T.SettingsPage.reset}
        </button>
      </p>


      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">About</span>
          <span>{T.SettingsPage.aboutHeading}</span>
        </span>
      </h2>

      <p className="ef-lede ef-read">
        {T.SettingsPage.aboutLede}
      </p>

      <ul className="ef-ledger about-list">
        <li>
          <span>{T.SettingsPage.aboutVersion}</span>
          <span>0.1.0</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutGameSource}</span>
          <span>{T.SettingsPage.aboutGameSourceValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutMirror}</span>
          <span>{T.SettingsPage.aboutMirrorValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutJavaSource}</span>
          <span>{T.SettingsPage.aboutJavaSourceValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutLoader}</span>
          <span>{T.SettingsPage.aboutLoaderValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutData}</span>
          <span>{T.SettingsPage.aboutDataValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutLicense}</span>
          <span>{T.SettingsPage.aboutLicenseValue}</span>
        </li>
        <li>
          <span>{T.SettingsPage.aboutGroup}</span>
          <span>
            <a
              href="https://qm.qq.com/q/IEiEjWiXe4"
              target="_blank"
              rel="noreferrer"
            >
              {T.SettingsPage.aboutGroupValue}
            </a>
          </span>
        </li>
      </ul>

      <p className="ef-lede ef-read">
        {T.SettingsPage.licenseNote}
      </p>

      <h2 className="ef-group-head">
        <span className="ef-pair">
          <span className="ef-eyebrow">Danger</span>
          <span>{T.SettingsPage.dangerHeading}</span>
        </span>
      </h2>

      <p className="ef-lede ef-read">
        {T.SettingsPage.resetLede}
      </p>

      <p className="ef-actions" style={{ marginTop: '0.9rem' }}>
        <button className="ef-btn danger" type="button" onClick={onReset}>
          {T.SettingsPage.resetBtn}
        </button>
      </p>
    </div>
  );
}
