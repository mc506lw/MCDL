import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';

import { api } from '../lib/api';
import {
  DEFAULT_EXTRAS,
  hasExtrasRun,
  rememberExtrasRun,
  resetExtras,
  runExtrasAfterInstall,
  takeExtrasRun,
  type ExtrasOptions,
} from '../lib/extras-api';
import { formatBytes, formatCount } from '../lib/format';
import { T } from '../i18n';
import type { Settings } from '../lib/settings';
import { DirPicker } from './DirPicker';
import { InstallExtras } from './InstallExtras';
import { Select } from './Select';
import { VersionPicker } from './VersionPicker';
import {
  LOADER_LABEL,
  type InstallRequest,
  type LoaderKind,
  type Source,
  type InstallPreview,
  type VersionManifest,
} from '../lib/types';

type LoaderChoice = 'none' | LoaderKind;

interface Props {
  versions: VersionManifest | null;
  loadingVersions: boolean;
  versionsError: string | null;
  onRetryVersions: () => void;
  defaultDir: string;
  settings: Settings;
  running: boolean;
  onStart: (req: InstallRequest) => void;
  onCancel: () => void;
  onOpenSettings: () => void;
}

const LOADERS: LoaderChoice[] = ['none', 'fabric', 'quilt', 'forge', 'neoforge'];

export function InstallForm({
  versions,
  loadingVersions,
  versionsError,
  onRetryVersions,
  defaultDir,
  settings,
  running,
  onStart,
  onCancel,
  onOpenSettings,
}: Props) {
  const [version, setVersion] = useState('');
  const [loader, setLoader] = useState<LoaderChoice>('none');
  const [loaderVersion, setLoaderVersion] = useState('');
  const [loaderVersions, setLoaderVersions] = useState<string[] | null>(null);
  const [loaderLoading, setLoaderLoading] = useState(false);
  const [loaderError, setLoaderError] = useState<string | null>(null);
  const [source, setSource] = useState<Source>('official');
  const [gameDir, setGameDir] = useState(defaultDir);
  const [skipAssets, setSkipAssets] = useState(settings.skipAssets);
  const [error, setError] = useState<string | null>(null);

  // 版本清单里既有正式版也有快照。默认只看正式版，测试版要能主动切出来。
  const [versionFilter, setVersionFilter] = useState<'release' | 'all'>('release');
  // 附加项：Fabric API 默认勾选；OptiFine 默认不勾（只有 Forge 1.13+ 能自动装）；版本名默认留空。
  const [extras, setExtras] = useState<ExtrasOptions>(DEFAULT_EXTRAS);
  // 安装结束（running 变 false）后据此补附加项。待办放在 extras-api 的模块作用域里，
  // 这样安装期间切分区再回来（组件重新挂载）也不会丢。
  const wasRunning = useRef(false);

  // 换了加载器或版本，已经选的加载器版本就作废
  useEffect(() => {
    setLoaderVersion('');
  }, [loader, version]);

  // 拉加载器版本列表。引擎侧限 20 条、从新到旧、5 分钟缓存，单次 ≤10 秒；
  // 超时或者失败都降级成「用最新版」，不让整个表单卡住。
  useEffect(() => {
    if (loader === 'none' || !version) {
      setLoaderVersions(null);
      setLoaderError(null);
      setLoaderLoading(false);
      return;
    }
    let alive = true;
    setLoaderLoading(true);
    setLoaderError(null);
    const timeout = window.setTimeout(() => {
      if (!alive) return;
      setLoaderError(T.InstallForm.loaderTimeout);
      setLoaderLoading(false);
    }, 10_000);
    api
      .listLoaders(loader, version)
      .then((list) => {
        if (alive) setLoaderVersions(list);
      })
      .catch((reason: unknown) => {
        if (alive) setLoaderError(String(reason));
      })
      .finally(() => {
        window.clearTimeout(timeout);
        if (alive) setLoaderLoading(false);
      });
    return () => {
      alive = false;
      window.clearTimeout(timeout);
    };
  }, [loader, version]);

  // 清单到了之后默认选最新正式版
  useEffect(() => {
    if (!version && versions) setVersion(versions.latest.release);
  }, [versions, version]);

  // 切回「正式版」时，如果当前选的是快照，就落回最新正式版——
  // 否则列表里看不到当前项，界面上会出现一个「选着但不在列表里」的版本。
  useEffect(() => {
    if (versionFilter !== 'release' || !versions) return;
    const current = versions.versions.find((v) => v.id === version);
    if (current && current.type !== 'release') setVersion(versions.latest.release);
  }, [versionFilter, versions, version]);

  // 设置里改了默认目录时同步过来（用户手动改过就不动）
  useEffect(() => {
    if (!gameDir && defaultDir) setGameDir(defaultDir);
  }, [defaultDir, gameDir]);

  const [preview, setPreview] = useState<InstallPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);

  const latest = versions?.latest.release ?? '';

  // 版本选择器只显示筛选后的列表；快照和正式版的安装路径完全一致，差别只在类型。
  const pickableVersions = useMemo(() => {
    const all = versions?.versions ?? [];
    return versionFilter === 'release' ? all.filter((v) => v.type === 'release') : all;
  }, [versions, versionFilter]);

  const selectedVersion = versions?.versions.find((v) => v.id === version) ?? null;
  const selectedIsSnapshot = Boolean(selectedVersion && selectedVersion.type !== 'release');

  // 版本/加载器/源/目录一变就重新估一次，让用户在点「开始安装」之前就知道代价。
  useEffect(() => {
    if (!version || !gameDir.trim()) {
      setPreview(null);
      return;
    }
    let alive = true;
    setPreviewing(true);
    const timer = window.setTimeout(() => {
      api
        .previewInstall({
          game_dir: gameDir.trim(),
          mc_version: version,
          loader:
            loader === 'none'
              ? null
              : {
                  kind: loader,
                  mc_version: version,
                  loader_version: loaderVersion.trim() || null,
                },
          source,
          concurrency: settings.concurrency,
          java: settings.javaPath.trim() || null,
          skip_assets: skipAssets,
        })
        .then((p) => {
          if (alive) setPreview(p);
        })
        .catch(() => {
          if (alive) setPreview(null);
        })
        .finally(() => {
          if (alive) setPreviewing(false);
        });
    }, 600);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [version, loader, loaderVersion, source, gameDir, skipAssets, settings.concurrency, settings.javaPath]);


  function submit(event: FormEvent) {
    event.preventDefault();
    if (!version) {
      setError(T.InstallForm.noVersion);
      return;
    }
    if (!gameDir.trim()) {
      setError(T.InstallForm.noGameDir);
      return;
    }
    setError(null);
    const req: InstallRequest = {
      game_dir: gameDir.trim(),
      mc_version: version,
      loader:
        loader === 'none'
          ? null
          : {
              kind: loader,
              mc_version: version,
              loader_version: loaderVersion.trim() || null,
            },
      source,
      concurrency: settings.concurrency,
      java: settings.javaPath.trim() || null,
      skip_assets: skipAssets,
    };
    // 记下这一轮的附加项：安装事件流不在这个组件的管辖范围内，
    // 只能等 running 变回 false 再据此补。
    rememberExtrasRun({
      gameDir: req.game_dir,
      mcVersion: req.mc_version,
      loader: req.loader?.kind ?? null,
      options: { ...extras, name: extras.name.trim() },
      startedAt: Date.now(),
      // 附加模组（Fabric API / OptiFine）也遵守版本隔离。
      isolated: settings.isolated,
    });
    resetExtras();
    onStart(req);
  }

  // 安装结束（成功 / 失败 / 取消都会走到这里）之后补齐附加项。
  // 失败和取消时引擎侧会先确认 versions/ 下有没有这次新造的目录，没有就整段跳过。
  useEffect(() => {
    if (running) {
      wasRunning.current = true;
      return;
    }
    // 首次挂载且没有待办：页面刚打开，或者上一轮安装已经在别处跑完了。
    if (!wasRunning.current && !hasExtrasRun()) return;
    wasRunning.current = false;
    const pending = takeExtrasRun();
    if (!pending) return;
    void runExtrasAfterInstall(pending);
  }, [running]);

  return (
    <form className="ef-form setup" onSubmit={submit}>
      <div className="field">
        <div className="field-head">
          <span id="ver-label">{T.InstallForm.versionLabel}</span>
          {latest && (
            <small>
              {T.InstallForm.latestRelease}{' '}
              <button
                className="ef-chip"
                type="button"
                onClick={() => setVersion(latest)}
                disabled={running}
              >
                {latest}
              </button>
            </small>
          )}
        </div>
        <div className="ef-seg" role="radiogroup" aria-label={T.InstallForm.versionTypeLabel}>
          <label>
            <input
              type="radio"
              name="version-filter"
              checked={versionFilter === 'release'}
              onChange={() => setVersionFilter('release')}
              disabled={running}
            />
            <span>{T.InstallForm.filterRelease}</span>
          </label>
          <label>
            <input
              type="radio"
              name="version-filter"
              checked={versionFilter === 'all'}
              onChange={() => setVersionFilter('all')}
              disabled={running}
            />
            <span>{T.InstallForm.filterAll}</span>
          </label>
        </div>
        {loadingVersions ? (
          <span className="ef-skeleton line" aria-label={T.InstallForm.versionsLoading} />
        ) : (
          <div className="select-wrap">
            <VersionPicker
              versions={pickableVersions}
              value={version}
              onChange={setVersion}
              disabled={running}
              loading={loadingVersions}
              label={T.InstallForm.versionFieldLabel}
            />
          </div>
        )}
        {selectedIsSnapshot && (
          <small>
            <span className="ef-chip">{T.InstallForm.snapshotBadge}</span> {selectedVersion?.type} │
            {T.InstallForm.snapshotNote}
          </small>
        )}
      </div>

      {versionsError && (
        <p className="ef-notice" data-flag={T.InstallForm.versionsErrorFlag}>
          {versionsError}
          <button className="ef-btn small" type="button" onClick={onRetryVersions}>
            {T.common.retry}
          </button>
        </p>
      )}

      <div className="field">
        <span id="loader-label">{T.InstallForm.loaderLabel}</span>
        <Select
          label={T.InstallForm.loaderLabel}
          value={loader}
          onChange={(v) => setLoader(v as LoaderChoice)}
          disabled={running}
          options={LOADERS.map((kind) => ({
            value: kind,
            label: kind === 'none' ? T.InstallForm.loaderNone : LOADER_LABEL[kind],
          }))}
        />
      </div>

      {loader !== 'none' && (
        <div className="field">
          <span id="lv-label">{T.InstallForm.loaderVersionLabel}</span>
          {loaderLoading ? (
            <span className="ef-skeleton line" aria-label={T.InstallForm.loaderLoading} />
          ) : loaderError ? (
            <>
              <p className="ef-notice" data-flag={T.InstallForm.loaderErrorFlag}>
                {T.InstallForm.loaderErrorNote}
              </p>
              <input
                aria-labelledby="lv-label"
                value={loaderVersion}
                onChange={(e) => setLoaderVersion(e.target.value)}
                placeholder={T.InstallForm.loaderVersionPlaceholder}
                disabled={running}
              />
            </>
          ) : (
            <div className="select-wrap">
              <select
                aria-labelledby="lv-label"
                value={loaderVersion}
                onChange={(e) => setLoaderVersion(e.target.value)}
                disabled={running}
              >
                <option value="">{T.InstallForm.loaderLatest}</option>
                {(loaderVersions ?? []).map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </div>
          )}
          <small>{T.InstallForm.loaderHint}</small>
        </div>
      )}

      <div className="field">
        <span>{T.InstallForm.sourceLabel}</span>
        <div className="ef-seg" role="radiogroup" aria-label={T.InstallForm.sourceLabel}>
          <label>
            <input
              type="radio"
              name="source"
              checked={source === 'official'}
              onChange={() => setSource('official')}
              disabled={running}
            />
            <span>{T.InstallForm.sourceOfficial}</span>
          </label>
          <label>
            <input
              type="radio"
              name="source"
              checked={source === 'bmclapi'}
              onChange={() => setSource('bmclapi')}
              disabled={running}
            />
            <span>{T.InstallForm.sourceMirror}</span>
          </label>
        </div>
        <small>
          {source === 'official'
            ? T.InstallForm.sourceHintOfficial
            : T.InstallForm.sourceHintMirror}
        </small>
      </div>

      {source === 'bmclapi' && (
        <p className="ef-notice" data-flag={T.InstallForm.sourceMirrorFlag}>
          {T.InstallForm.sourceMirrorNote}
        </p>
      )}

      <div className="field">
        <span id="dir-label">{T.InstallForm.dirLabel}</span>
        <DirPicker
          value={gameDir}
          dirs={settings.dirs}
          onChange={setGameDir}
          disabled={running}
          label={T.InstallForm.dirLabel}
        />
        <small>{T.InstallForm.dirHint}</small>
      </div>

      <p className="ef-field preview-line" aria-live="polite">
        {previewing && T.InstallForm.previewEstimating}
        {!previewing && preview && (
          <>
            {T.InstallForm.preview({
              files: formatCount(preview.files),
              bytes: formatBytes(preview.bytes),
              libraries: preview.libraries,
              assets: preview.assets,
              javaMajor: preview.java_major,
              javaLocal: Boolean(preview.java_path),
            })}
          </>
        )}
        {!previewing && !preview && T.InstallForm.previewEmpty}
      </p>

      <label className="ef-setting">
        <span>
          <b>{T.InstallForm.skipAssetsLabel}</b>
          <small>{T.InstallForm.skipAssetsHint}</small>
        </span>
        <input
          className="ef-switch"
          type="checkbox"
          role="switch"
          checked={skipAssets}
          onChange={(e) => setSkipAssets(e.target.checked)}
          disabled={running}
        />
      </label>

      <InstallExtras
        mcVersion={version}
        loader={loader}
        gameDir={gameDir}
        running={running}
        options={extras}
        onChange={setExtras}
      />

      <p className="field-hint">
        {T.InstallForm.footerConcurrency(settings.concurrency)} · Java{' '}
        {settings.javaPath || T.InstallForm.footerJavaAuto} ·{' '}
        <button className="ef-btn small" type="button" onClick={onOpenSettings}>
          {T.InstallForm.footerEdit}
        </button>
      </p>

      {error && (
        <p className="ef-notice" data-flag={T.InstallForm.errorFlag}>
          {error}
        </p>
      )}

      <p className="ef-actions">
        <button
          className="ef-btn go"
          type="submit"
          aria-busy={running ? 'true' : undefined}
          disabled={running || loadingVersions || !version}
        >
          {running ? T.InstallForm.submitRunning : T.InstallForm.submit}
        </button>
        <button className="ef-btn" type="button" onClick={onCancel} disabled={!running}>
          {T.common.cancel}
        </button>
      </p>
    </form>
  );
}
