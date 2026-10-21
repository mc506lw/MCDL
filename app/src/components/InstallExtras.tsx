// 「装完之后补东西」的选项区块：Fabric API、OptiFine、自定义版本名。
//
// 挂在 `InstallForm` 里，和安装主表单共用同一个 `<form className="ef-form setup">`，
// 所以只用套件里现成的类名，不新增样式。
//
// 这一块只负责「让用户看得懂、选得到」；真正干活的是 extras-api.ts → Tauri 命令 →
// mcdl-core::extras。所有结论（成功/跳过/失败/要手动）都来自引擎，不在这里编。

import { useEffect, useMemo, useState } from 'react';

import {
  EXTRA_LABEL,
  EXTRA_STATUS_LABEL,
  localNameCheck,
  openOptifineInstaller,
  resolveExtras,
  useExtrasState,
  validateVersionName,
  type ExtrasOptions,
  type ExtrasPlan,
  type NameCheck,
} from '../lib/extras-api';
import { formatBytes } from '../lib/format';
import type { LoaderKind } from '../lib/types';

interface Props {
  mcVersion: string;
  /** release / snapshot / old_beta / old_alpha */
  mcKind: string;
  loader: 'none' | LoaderKind;
  gameDir: string;
  running: boolean;
  options: ExtrasOptions;
  onChange: (next: ExtrasOptions) => void;
}

/** 安装器会生成什么样的版本目录名（只用来提示，不参与逻辑）。 */
function autoNameHint(mcVersion: string, loader: LoaderKind | null): string {
  switch (loader) {
    case 'fabric':
      return `fabric-loader-<加载器版本>-${mcVersion}`;
    case 'quilt':
      return `quilt-loader-<加载器版本>-${mcVersion}`;
    case 'forge':
      return `${mcVersion}-forge-<加载器版本>`;
    case 'neoforge':
      return `neoforge-<加载器版本>`;
    default:
      return mcVersion;
  }
}

/** 计划查询的超时。引擎里 optifine.net 页面查询是 20 秒，这里留出余量。 */
const PLAN_TIMEOUT = 25_000;

export function InstallExtras({
  mcVersion,
  mcKind,
  loader,
  gameDir,
  running,
  options,
  onChange,
}: Props) {
  const extras = useExtrasState();
  const [plan, setPlan] = useState<ExtrasPlan | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [planError, setPlanError] = useState<string | null>(null);
  const [remoteCheck, setRemoteCheck] = useState<NameCheck | null>(null);
  const [manual, setManual] = useState<{ busy: boolean; message: string | null; error: string | null }>(
    { busy: false, message: null, error: null },
  );

  const loaderKind: LoaderKind | null = loader === 'none' ? null : loader;
  const fabricFamily = loader === 'fabric' || loader === 'quilt';
  const snapshot = mcKind !== 'release';

  // 版本/加载器一变就重新问一次「能补什么」。
  useEffect(() => {
    if (!mcVersion) {
      setPlan(null);
      setPlanError(null);
      return;
    }
    let alive = true;
    setPlanLoading(true);
    setPlanError(null);
    setManual({ busy: false, message: null, error: null });

    const timer = window.setTimeout(() => {
      if (!alive) return;
      setPlanError('查询超过 25 秒没有返回（optifine.net 或 Modrinth 可能不可达）');
      setPlanLoading(false);
    }, PLAN_TIMEOUT);

    resolveExtras(mcVersion, loaderKind)
      .then((value) => {
        if (!alive) return;
        // 迟到的结果仍然要用：超时只是界面提示，不取消底层请求。
        setPlanError(null);
        setPlan(value);
      })
      .catch((reason: unknown) => {
        if (!alive) return;
        setPlan(null);
        setPlanError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        window.clearTimeout(timer);
        if (alive) setPlanLoading(false);
      });

    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mcVersion, loader]);

  const optifine = plan?.optifine;
  const optifineSupported = Boolean(optifine?.supported);

  // 计划说这个组合不支持时，把勾选状态也拉回来，避免「界面上勾着、实际跳过」。
  useEffect(() => {
    if (plan && !optifineSupported && options.optifine) {
      onChange({ ...options, optifine: false });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, optifineSupported]);

  // 版本名：本地即时校验 + 引擎异步校验（桌面端以引擎为准）。
  const localCheck = useMemo(() => localNameCheck(options.name), [options.name]);
  useEffect(() => {
    if (!options.name.trim()) {
      setRemoteCheck(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      validateVersionName(options.name)
        .then((check) => {
          if (alive) setRemoteCheck(check);
        })
        .catch(() => {
          // 校验本身失败不影响安装，退回本地结论。
          if (alive) setRemoteCheck(null);
        });
    }, 300);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [options.name]);

  const nameError = !options.name
    ? null
    : !localCheck.ok
      ? localCheck.reason
      : remoteCheck && !remoteCheck.ok
        ? remoteCheck.reason
        : null;

  const fabricHint = (() => {
    if (!fabricFamily) {
      return loader === 'none'
        ? '只有 Fabric / Quilt 需要它；原版和 Forge 系不装'
        : 'Forge 系不需要 Fabric API';
    }
    if (planLoading) return '正在查 Modrinth…';
    if (plan?.fabric_api_error) return `查不到版本：${plan.fabric_api_error}`;
    if (plan?.fabric_api) {
      return `${plan.fabric_api.filename} │ ${formatBytes(plan.fabric_api.size)}`;
    }
    if (plan) return 'Modrinth 上没有这个 MC 版本的构建，安装后会跳过';
    return '安装后从 Modrinth 取最新版放进 mods/';
  })();

  const optifineHint = (() => {
    if (planLoading) return '正在查 optifine.net…';
    if (planError) return planError;
    if (!optifine) return '选择版本后查询官方构建';
    const build = optifine.build;
    if (optifineSupported) {
      return build
        ? `${build.filename}${build.pre ? '（预览版）' : ''} → 放进 mods/，需要 Forge 加载`
        : optifine.reason;
    }
    if (build && optifine.manual_possible) {
      return `${optifine.reason}。官方构建：${build.filename}${build.pre ? '（预览版）' : ''}`;
    }
    return optifine.reason;
  })();

  async function openInstaller() {
    if (!gameDir.trim() || !mcVersion) return;
    setManual({ busy: true, message: null, error: null });
    try {
      const outcome = await openOptifineInstaller(gameDir.trim(), mcVersion);
      setManual({
        busy: false,
        message: [outcome.message, outcome.manual_hint].filter(Boolean).join(' '),
        error: null,
      });
    } catch (error) {
      setManual({
        busy: false,
        message: null,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  return (
    <>
      <div className="field">
        <div className="field-head">
          <span>装完之后</span>
          <small>附加项</small>
        </div>
        <small>
          这些都不属于安装主流程：某一项失败不会影响已经装好的版本，结果会逐条列出来。
        </small>
      </div>

      <label className="ef-setting">
        <span>
          <b>Fabric API</b>
          <small>{fabricHint}</small>
        </span>
        <input
          className="ef-switch"
          type="checkbox"
          role="switch"
          checked={fabricFamily && options.fabricApi}
          onChange={(event) => onChange({ ...options, fabricApi: event.target.checked })}
          disabled={running || !fabricFamily}
        />
      </label>

      <label className="ef-setting">
        <span>
          <b>OptiFine</b>
          <small>{optifineHint}</small>
        </span>
        <input
          className="ef-switch"
          type="checkbox"
          role="switch"
          checked={options.optifine}
          onChange={(event) => onChange({ ...options, optifine: event.target.checked })}
          disabled={running || !optifineSupported}
        />
      </label>

      <p className="field-hint">
        OptiFine 文件来自 optifine.net 官方，本工具不托管，也不代你点 Install。
        {optifineSupported
          ? '只有 Forge + 1.13 及以后能自动放进 mods/。'
          : '当前组合只能手动安装；原版需要官方安装器 patch 游戏。'}
      </p>

      {optifine && !optifineSupported && optifine.manual_possible && (
        <p className="ef-actions">
          <button
            className="ef-btn small"
            type="button"
            onClick={() => void openInstaller()}
            disabled={running || manual.busy || !gameDir.trim()}
          >
            {manual.busy ? '正在下载官方安装器…' : '下载并打开官方安装器'}
          </button>
        </p>
      )}

      {manual.message && (
        <p className="ef-notice" data-flag="需要手动完成">
          {manual.message}
        </p>
      )}
      {manual.error && (
        <p className="ef-notice" data-flag="打开安装器失败">
          {manual.error}
        </p>
      )}

      <div className="field">
        <span id="extras-name-label">版本名</span>
        <input
          aria-labelledby="extras-name-label"
          value={options.name}
          onChange={(event) => onChange({ ...options, name: event.target.value })}
          placeholder="留空 = 保留安装器生成的目录名"
          disabled={running}
          spellCheck={false}
        />
        <small>
          {nameError
            ? nameError
            : options.name.trim()
              ? `装完后 versions/ 下的目录会改成这个名字，JSON 里的 id 同步改写`
              : `安装器生成的名字通常形如 ${autoNameHint(mcVersion, loaderKind)}；留空就不改`}
          {snapshot && !nameError ? '（测试版同样适用）' : ''}
        </small>
      </div>

      {extras.phase !== 'idle' && (
        <div className="field">
          <span>附加项结果</span>

          {extras.phase === 'running' && (
            <>
              <span className="ef-skeleton line" aria-label="正在补齐附加项" />
              <small>正在改版本名 / 下载模组…</small>
            </>
          )}

          {extras.error && (
            <p className="ef-notice" data-flag="附加项失败">
              {extras.error}
            </p>
          )}

          {extras.items.map((notice, index) => (
            <p
              key={`${notice.item}-${index}`}
              className="ef-notice"
              data-flag={EXTRA_STATUS_LABEL[notice.status] ?? notice.status}
            >
              {`${EXTRA_LABEL[notice.item] ?? notice.item}：${notice.message}`}
            </p>
          ))}

          {extras.phase === 'done' && extras.versionId && (
            <small>最终版本目录：versions/{extras.versionId}</small>
          )}
          {extras.demo && (
            <small>浏览器预览：以上是演示结论，不会真的下载或改目录。</small>
          )}
        </div>
      )}
    </>
  );
}
