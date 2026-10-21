import type { CSSProperties } from 'react';

import { formatBytes, formatCount, formatDuration, formatEta, formatSpeed, pad2, padPercent } from '../lib/format';
import { SOURCE_LABEL, LOADER_LABEL } from '../lib/types';
import { elapsedMs, stageEta, stageRatio, type Task } from '../lib/useInstall';
import { T } from '../i18n';
import { StageRail } from './StageRail';

/**
 * 主视觉：一次安装的实时读数。
 *
 * 不排成规整的卡片格子 —— 三个大数字（速度、本阶段剩余、文件数）在上面，
 * 下面一条分段进度条、一条阶段轨道、一张「标签｜数值」表。
 * 所有数字都来自引擎事件：没数据就写空状态的一句话，不编。
 */
export function Readout({ task }: { task: Task | null }) {
  if (!task) {
    return (
      <>
        <div className="empty-box">
          <p>{T.Readout.empty}</p>
        </div>
      </>
    );
  }

  const stage = task.stages[task.index - 1] ?? null;
  const measured = Boolean(stage?.measured);
  const ratio = stageRatio(stage);
  const eta = stageEta(task);
  const speed = formatSpeed(task.speed);
  const running = task.status === 'running';

  const style = measured ? ({ '--p': ratio } as CSSProperties) : undefined;

  return (
    <>
      <ul className="ef-facts ef-field">
        <li>{task.req.mc_version}</li>
        <li>{task.req.loader ? `${LOADER_LABEL[task.req.loader.kind]}${task.req.loader.loader_version ? ` ${task.req.loader.loader_version}` : ''}` : T.common.vanilla}</li>
        <li>{SOURCE_LABEL[task.req.source]}</li>
        <li className="path" title={task.req.game_dir}>
          {task.req.game_dir || T.common.dirUnset}
        </li>
      </ul>

      <div className="ef-figs readouts">
        <div>
          <b>{running ? speed.value : '—'}</b>
          <small>{T.Readout.speed(running ? speed.unit : '')}</small>
        </div>
        <div>
          <b>{eta === null ? '—' : formatEta(eta)}</b>
          <small>{T.Readout.eta}</small>
        </div>
        <div>
          <b>{measured ? formatCount(stage?.done ?? 0) : '—'}</b>
          <small>
            {measured
              ? T.Readout.files(formatCount(stage?.total ?? 0))
              : T.Readout.filesPlain}
          </small>
        </div>
      </div>

      <div className="progress-block">
        <div
          className={`ef-progress${measured ? '' : ' is-unknown'}`}
          role="progressbar"
          aria-label={T.Readout.progressAria}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={measured ? Math.round(ratio * 100) : undefined}
          style={style}
        >
          <span className="ef-eyebrow">
            {task.total > 0
              ? T.common.stageLine(
                  pad2(task.index),
                  pad2(task.total),
                  stage?.label || T.common.preparing,
                )
              : T.common.preparing}
          </span>
          <span className="ef-field">{measured ? padPercent(ratio) : '——'}</span>
          <i />
        </div>
      </div>

      <StageRail task={task} />

      <ul className="ef-ledger">
        <li>
          <span>{T.Readout.transferred}</span>
          <span className="ef-field">{formatBytes(task.bytesDone)}</span>
        </li>
        <li>
          <span>{T.common.elapsed}</span>
          <span className="ef-field">{formatDuration(elapsedMs(task))}</span>
        </li>
        <li>
          <span>{task.status === 'done' ? T.Readout.instanceDone : T.common.status}</span>
          <span className="ef-field">
            {task.report
              ? task.report.installed_version
              : running
                ? T.common.statusRunning
                : task.status === 'cancelled'
                  ? T.common.statusCancelled
                  : T.common.statusFailed}
          </span>
        </li>
        <li>
          <span>{T.Readout.javaLabel}</span>
          <span className="ef-field path" title={task.report?.java ?? ''}>
            {task.report?.java ?? (running ? T.Readout.javaAuto : '—')}
          </span>
        </li>
      </ul>
    </>
  );
}
