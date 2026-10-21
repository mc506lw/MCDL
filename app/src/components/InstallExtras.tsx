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
import { T } from '../i18n';
import { formatBytes } from '../lib/format';
import type { LoaderKind } from '../lib/types';

interface Props {
  mcVersion: string;
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
      return T.InstallExtras.autoNameFabric(mcVersion);
    case 'quilt':
      return T.InstallExtras.autoNameQuilt(mcVersion);
    case 'forge':
      return T.InstallExtras.autoNameForge(mcVersion);
    case 'neoforge':
      return T.InstallExtras.autoNameNeoForge;
    default:
      return mcVersion;
  }
}

/** 计划查询的超时。引擎里 optifine.net 页面查询是 20 秒，这里留出余量。 */
const PLAN_TIMEOUT = 25_000;

export function InstallExtras({
  mcVersion,
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
      setPlanError(T.InstallExtras.planTimeout);
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
        ? T.InstallExtras.fabricOnlyForFabric
        : T.InstallExtras.fabricNotForForge;
    }
    if (planLoading) return T.InstallExtras.fabricQuerying;
    if (plan?.fabric_api_error) return T.InstallExtras.fabricError(plan.fabric_api_error);
    if (plan?.fabric_api) {
      return `${plan.fabric_api.filename} │ ${formatBytes(plan.fabric_api.size)}`;
    }
    if (plan) return T.InstallExtras.fabricNone;
    return T.InstallExtras.fabricPending;
  })();

  const optifineHint = (() => {
    if (planLoading) return T.InstallExtras.optifineQuerying;
    if (planError) return planError;
    if (!optifine) return T.InstallExtras.optifinePending;
    const build = optifine.build;
    if (optifineSupported) {
      return build
        ? T.InstallExtras.optifineAuto(
            build.filename,
            build.pre ? T.InstallExtras.previewBadge : '',
          )
        : optifine.reason;
    }
    if (build && optifine.manual_possible) {
      return T.InstallExtras.optifineManual(
        optifine.reason,
        build.filename,
        build.pre ? T.InstallExtras.previewBadge : '',
      );
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
          <span>{T.InstallExtras.extrasHeading}</span>
          <small>{T.InstallExtras.extrasEyebrow}</small>
        </div>
        <small>{T.InstallExtras.extrasNote}</small>
      </div>

      <label className="ef-setting">
        <span>
          <b>{T.InstallExtras.fabricApiLabel}</b>
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
          <b>{T.InstallExtras.optifineLabel}</b>
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
        {T.InstallExtras.optifineSource}
        {optifineSupported
          ? T.InstallExtras.optifineAutoScope
          : T.InstallExtras.optifineManualScope}
      </p>

      {optifine && !optifineSupported && optifine.manual_possible && (
        <p className="ef-actions">
          <button
            className="ef-btn small"
            type="button"
            onClick={() => void openInstaller()}
            disabled={running || manual.busy || !gameDir.trim()}
          >
            {manual.busy ? T.InstallExtras.openInstallerBusy : T.InstallExtras.openInstaller}
          </button>
        </p>
      )}

      {manual.message && (
        <p className="ef-notice" data-flag={T.InstallExtras.manualFlag}>
          {manual.message}
        </p>
      )}
      {manual.error && (
        <p className="ef-notice" data-flag={T.InstallExtras.openFailedFlag}>
          {manual.error}
        </p>
      )}

      <div className="field">
        <span id="extras-name-label">{T.InstallExtras.nameLabel}</span>
        <input
          aria-labelledby="extras-name-label"
          value={options.name}
          onChange={(event) => onChange({ ...options, name: event.target.value })}
          placeholder={T.InstallExtras.namePlaceholder}
          disabled={running}
          spellCheck={false}
        />
        <small>
          {nameError
            ? nameError
            : options.name.trim()
              ? T.InstallExtras.nameHintRenamed
              : T.InstallExtras.nameHintAuto(autoNameHint(mcVersion, loaderKind))}
        </small>
      </div>

      {extras.phase !== 'idle' && (
        <div className="field">
          <span>{T.InstallExtras.resultHeading}</span>

          {extras.phase === 'running' && (
            <>
              <span className="ef-skeleton line" aria-label={T.InstallExtras.resultRunning} />
              <small>{T.InstallExtras.resultRunningHint}</small>
            </>
          )}

          {extras.error && (
            <p className="ef-notice" data-flag={T.InstallExtras.resultFailedFlag}>
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
            <small>{T.InstallExtras.finalDir(extras.versionId)}</small>
          )}
          {extras.demo && (
            <small>{T.InstallExtras.demoNote}</small>
          )}
        </div>
      )}
    </>
  );
}
