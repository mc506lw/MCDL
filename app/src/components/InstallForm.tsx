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
import { formatBytes } from '../lib/format';
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
      setError('未选择版本。清单加载后会自动填入最新正式版。');
      return;
    }
    if (!gameDir.trim()) {
      setError('请选择游戏目录，或点「默认目录」使用系统 .minecraft。');
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
        <div className="ef-seg" role="radiogroup" aria-label="版本类型">
          <label>
            <input
              type="radio"
              name="version-filter"
              checked={versionFilter === 'release'}
              onChange={() => setVersionFilter('release')}
              disabled={running}
            />
            <span>正式版</span>
          </label>
          <label>
            <input
              type="radio"
              name="version-filter"
              checked={versionFilter === 'all'}
              onChange={() => setVersionFilter('all')}
              disabled={running}
            />
            <span>全部（含测试版）</span>
          </label>
        </div>
        {loadingVersions ? (
          <span className="ef-skeleton line" aria-label="正在拉取版本清单" />
        ) : (
          <div className="select-wrap">
            <VersionPicker
              versions={pickableVersions}
              value={version}
              onChange={setVersion}
              disabled={running}
              loading={loadingVersions}
              label="Minecraft 版本"
            />
          </div>
        )}
        {selectedIsSnapshot && (
          <small>
            <span className="ef-chip">测试版</span> {selectedVersion?.type} │
            这是 Mojang 的测试构建，可能不稳定；安装路径与正式版完全一致。
          </small>
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
        <Select
          label="加载器"
          value={loader}
          onChange={(v) => setLoader(v as LoaderChoice)}
          disabled={running}
          options={LOADERS.map((kind) => ({
            value: kind,
            label: kind === 'none' ? '不安装加载器（原版）' : LOADER_LABEL[kind],
          }))}
        />
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

      <InstallExtras
        mcVersion={version}
        mcKind={selectedVersion?.type ?? 'release'}
        loader={loader}
        gameDir={gameDir}
        running={running}
        options={extras}
        onChange={setExtras}
      />

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
