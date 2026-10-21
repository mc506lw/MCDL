import { useEffect, useState, type FormEvent } from 'react';

import { api } from '../lib/api';
import { formatBytes } from '../lib/format';
import type { Settings } from '../lib/settings';
import { DirPicker } from './DirPicker';
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
      setLoaderError('请求超过 10 秒没有返回');
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

  // 设置里改了默认目录时同步过来（用户手动改过就不动）
  useEffect(() => {
    if (!gameDir && defaultDir) setGameDir(defaultDir);
  }, [defaultDir, gameDir]);

  const [preview, setPreview] = useState<InstallPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);

  const latest = versions?.latest.release ?? '';

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
      setError('未选择版本。清单加载后会自动填入最新正式版。');
      return;
    }
    if (!gameDir.trim()) {
      setError('请选择游戏目录，或点「默认目录」使用系统 .minecraft。');
      return;
    }
    setError(null);
    onStart({
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
    });
  }

  return (
    <form className="ef-form setup" onSubmit={submit}>
      <div className="field">
        <div className="field-head">
          <span id="ver-label">版本</span>
          {latest && (
            <small>
              最新正式版{' '}
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
        {loadingVersions ? (
          <span className="ef-skeleton line" aria-label="正在拉取版本清单" />
        ) : (
          <div className="select-wrap">
            <VersionPicker
              versions={versions?.versions ?? []}
              value={version}
              onChange={setVersion}
              disabled={running}
              loading={loadingVersions}
              label="Minecraft 版本"
            />
          </div>
        )}
      </div>

      {versionsError && (
        <p className="ef-notice" data-flag="清单没拉到">
          {versionsError}
          <button className="ef-btn small" type="button" onClick={onRetryVersions}>
            重试
          </button>
        </p>
      )}

      <div className="field">
        <span id="loader-label">加载器</span>
        <div className="select-wrap">
          <select
            aria-labelledby="loader-label"
            value={loader}
            onChange={(e) => setLoader(e.target.value as LoaderChoice)}
            disabled={running}
          >
            {LOADERS.map((kind) => (
              <option key={kind} value={kind}>
                {kind === 'none' ? '不安装加载器（原版）' : LOADER_LABEL[kind]}
              </option>
            ))}
          </select>
        </div>
      </div>

      {loader !== 'none' && (
        <div className="field">
          <span id="lv-label">加载器版本</span>
          {loaderLoading ? (
            <span className="ef-skeleton line" aria-label="正在获取加载器版本" />
          ) : loaderError ? (
            <>
              <p className="ef-notice" data-flag="取不到版本">
                无法获取版本列表，将使用最新版。
              </p>
              <input
                aria-labelledby="lv-label"
                value={loaderVersion}
                onChange={(e) => setLoaderVersion(e.target.value)}
                placeholder="最新版"
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
                <option value="">最新版（默认）</option>
                {(loaderVersions ?? []).map((item) => (
                  <option key={item} value={item}>
                    {item}
                  </option>
                ))}
              </select>
            </div>
          )}
          <small>最多显示 20 个版本，从新到旧。选「最新版」即可。</small>
        </div>
      )}

      <div className="field">
        <span>下载源</span>
        <div className="ef-seg" role="radiogroup" aria-label="下载源">
          <label>
            <input
              type="radio"
              name="source"
              checked={source === 'official'}
              onChange={() => setSource('official')}
              disabled={running}
            />
            <span>官方源</span>
          </label>
          <label>
            <input
              type="radio"
              name="source"
              checked={source === 'bmclapi'}
              onChange={() => setSource('bmclapi')}
              disabled={running}
            />
            <span>BMCLAPI 第三方镜像</span>
          </label>
        </div>
        <small>
          {source === 'official'
            ? '默认使用官方源；镜像仅用于加速，需手动开启。'
            : '镜像仅用于依赖库、资源文件和游戏本体；版本清单与版本 JSON 仍走官方。'}
        </small>
      </div>

      {source === 'bmclapi' && (
        <p className="ef-notice" data-flag="第三方镜像">
          BMCLAPI 是社区维护的第三方镜像，不是 Mojang 官方服务器。
        </p>
      )}

      <div className="field">
        <span id="dir-label">游戏目录</span>
        <DirPicker
          value={gameDir}
          dirs={settings.dirs}
          onChange={setGameDir}
          disabled={running}
          label="游戏目录"
        />
        <small>版本、依赖库、资源文件都装在这里。</small>
      </div>

      <p className="ef-field preview-line" aria-live="polite">
        {previewing && '正在估算这次要下多少…'}
        {!previewing && preview && (
          <>
            预计 {preview.files} 个文件 │ {formatBytes(preview.bytes)} │ 依赖库{' '}
            {preview.libraries} 个 │ 资源 {preview.assets} 个 │ Java {preview.java_major}
            {preview.java_path ? '（本机已有）' : '（需要下载）'}
          </>
        )}
        {!previewing && !preview && '选择版本和目录后显示预计下载量。'}
      </p>

      <label className="ef-setting">
        <span>
          <b>只装最小集合</b>
          <small>跳过资源文件。游戏可启动，但会缺少音效和语言文件。</small>
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

      <p className="field-hint">
        并发 {settings.concurrency} · Java {settings.javaPath || '自动探测'} ·{' '}
        <button className="ef-btn small" type="button" onClick={onOpenSettings}>
          在设置里改
        </button>
      </p>

      {error && (
        <p className="ef-notice" data-flag="还差一步">
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
          {running ? '安装中' : '开始安装'}
        </button>
        <button className="ef-btn" type="button" onClick={onCancel} disabled={!running}>
          取消
        </button>
      </p>
    </form>
  );
}
