import { useEffect, useState, type FormEvent } from 'react';

import { api } from '../lib/api';
import { formatDate } from '../lib/format';
import type { Settings } from '../lib/settings';
import {
  LOADER_LABEL,
  type InstallRequest,
  type LoaderKind,
  type Source,
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

  const releases = versions?.versions.filter((v) => v.type === 'release') ?? [];
  const latest = versions?.latest.release ?? '';

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!version) {
      setError('还没选版本。清单拉下来之后这里会自动填上最新正式版。');
      return;
    }
    if (!gameDir.trim()) {
      setError('游戏目录不能空。点「默认目录」用系统的 .minecraft，或者自己填一个。');
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
            <select
              aria-labelledby="ver-label"
              value={version}
              onChange={(e) => setVersion(e.target.value)}
              disabled={running}
            >
              {releases.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.id}
                  {item.releaseTime ? ` · ${formatDate(item.releaseTime)}` : ''}
                </option>
              ))}
            </select>
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
          <small>列表最多 20 条，从新到旧。留「最新版」就不用管。</small>
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
            ? '默认走官方源。镜像只是加速，需要自己打开。'
            : '版本清单和版本 JSON 仍然走官方，只有依赖库、资源文件和游戏本体走镜像。'}
        </small>
      </div>

      {source === 'bmclapi' && (
        <p className="ef-notice" data-flag="第三方镜像">
          BMCLAPI 是社区维护的第三方镜像，不是 Mojang 官方服务器。
        </p>
      )}

      <div className="field">
        <span id="dir-label">游戏目录</span>
        <div className="inline">
          <input
            aria-labelledby="dir-label"
            value={gameDir}
            onChange={(e) => setGameDir(e.target.value)}
            placeholder=".minecraft 所在目录"
            disabled={running}
            spellCheck={false}
          />
          <button
            className="ef-btn small"
            type="button"
            onClick={() => setGameDir(defaultDir)}
            disabled={running}
          >
            默认目录
          </button>
        </div>
        <small>版本、依赖库、资源文件都装在这里。</small>
      </div>

      <label className="ef-setting">
        <span>
          <b>只装最小集合</b>
          <small>跳过资源文件。能启动，但游戏里会缺音效和语言文件。</small>
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
