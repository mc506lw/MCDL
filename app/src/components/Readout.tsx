import type { CSSProperties } from 'react';

import { formatBytes, formatCount, formatDuration, formatEta, formatSpeed, pad2, padPercent } from '../lib/format';
import { SOURCE_LABEL, LOADER_LABEL } from '../lib/types';
import { elapsedMs, stageEta, stageRatio, type Task } from '../lib/useInstall';
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
          <p>
            还没有开始安装。左边选好版本和加载器，点「开始安装」，
            这里会出现下载速度、本阶段剩余时间，以及每个阶段的文件数和字节数。
          </p>
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
        <li>{task.req.loader ? `${LOADER_LABEL[task.req.loader.kind]}${task.req.loader.loader_version ? ` ${task.req.loader.loader_version}` : ''}` : '原版'}</li>
        <li>{SOURCE_LABEL[task.req.source]}</li>
        <li className="path" title={task.req.game_dir}>
          {task.req.game_dir || '未指定目录'}
        </li>
      </ul>

      <div className="ef-figs readouts">
        <div>
          <b>{running ? speed.value : '—'}</b>
          <small>{running ? `${speed.unit} 下载速度` : '下载速度'}</small>
        </div>
        <div>
          <b>{eta === null ? '—' : formatEta(eta)}</b>
          <small>本阶段剩余</small>
        </div>
        <div>
          <b>{measured ? formatCount(stage?.done ?? 0) : '—'}</b>
          <small>
            {measured
              ? `已下载文件 · 共 ${formatCount(stage?.total ?? 0)}`
              : '已下载文件（本阶段无文件数）'}
          </small>
        </div>
      </div>

      <div className="progress-block">
        <div
          className={`ef-progress${measured ? '' : ' is-unknown'}`}
          role="progressbar"
          aria-label="本阶段进度"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={measured ? Math.round(ratio * 100) : undefined}
          style={style}
        >
          <span className="ef-eyebrow">
            {task.total > 0
              ? `${pad2(task.index)} / ${pad2(task.total)} · ${stage?.label || '准备中'}`
              : '准备中'}
          </span>
          <span className="ef-field">{measured ? padPercent(ratio) : '——'}</span>
          <i />
        </div>
      </div>

      <StageRail task={task} />

      <ul className="ef-ledger">
        <li>
          <span>累计已传输</span>
          <span className="ef-field">{formatBytes(task.bytesDone)}</span>
        </li>
        <li>
          <span>已用时</span>
          <span className="ef-field">{formatDuration(elapsedMs(task))}</span>
        </li>
        <li>
          <span>{task.status === 'done' ? '装好的实例' : '状态'}</span>
          <span className="ef-field">
            {task.report
              ? task.report.installed_version
              : running
                ? '安装中'
                : task.status === 'cancelled'
                  ? '已取消'
                  : '失败'}
          </span>
        </li>
        <li>
          <span>Java 运行时</span>
          <span className="ef-field path" title={task.report?.java ?? ''}>
            {task.report?.java ?? (running ? '安装时自动探测' : '—')}
          </span>
        </li>
      </ul>
    </>
  );
}
