import { formatCount, pad2 } from '../lib/format';
import { T } from '../i18n';
import type { Task } from '../lib/useInstall';

/**
 * 安装阶段轨道。
 *
 * 套件里没有这个部件，按「步骤条」的规则推出来：
 * - 不能点 → 全部直角，不切角
 * - 当前阶段 = 强调色填充（只表示「就在这里」）
 * - 已过阶段 = 墨色实心，编号反白
 * - 还没到的 = 浅色，细线隔开
 * - 被取消／失败时停住的那一格 = 虚线框（套件里虚线的含义：没走完）
 *
 * 阶段数、序号、名字全部来自引擎的 `Progress::Stage`，界面不自己推：
 * 中途补插 Java 阶段时 `total` 会加一，这里跟着变。
 */
export function StageRail({ task }: { task: Task }) {
  if (task.total <= 0) return null;

  const finished = task.status !== 'running';

  return (
    <ol className="rail" aria-label={T.StageRail.ariaLabel}>
      {Array.from({ length: task.total }, (_, i) => {
        const stage = task.stages[i];
        const here = !finished && i + 1 === task.index;
        const stopped = finished && task.status !== 'done' && i + 1 === task.index;
        const done = task.status === 'done' || i + 1 < task.index;
        const state = here ? 'is-now' : stopped ? 'is-stop' : done ? 'is-done' : 'is-wait';
        const label = stage?.label || '—';

        let fig: string = T.StageRail.waiting;
        if (stopped) fig = T.StageRail.stopped;
        else if (stage?.measured) fig = T.StageRail.files(formatCount(stage.done), formatCount(stage.total));
        else if (done) fig = T.StageRail.ready;
        else if (here) fig = T.StageRail.running;

        return (
          <li key={i} className={state} aria-current={here ? 'step' : undefined}>
            <span className="no">{pad2(i + 1)}</span>
            <span className="zh">{label}</span>
            <span className="fig">{fig}</span>
          </li>
        );
      })}
    </ol>
  );
}
