import { formatBytes, formatCount, formatDuration } from '../lib/format';
import { LOADER_LABEL, SOURCE_LABEL } from '../lib/types';
import { T } from '../i18n';
import { elapsedMs, type Task } from '../lib/useInstall';

interface Props {
  tasks: Task[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  onCancel: () => void;
  onOpenDir: (path: string) => void;
}

function StateTag({ status }: { status: Task['status'] }) {
  if (status === 'running') return <span className="ef-state is-alert">{T.common.statusRunning}</span>;
  if (status === 'done') return <span className="ef-state">{T.common.statusDone}</span>;
  if (status === 'failed') return <span className="ef-state is-alert">{T.common.statusFailed}</span>;
  return <span className="ef-state is-draft">{T.common.statusCancelled}</span>;
}

/** 安装队列：一轮安装一行，左边是版本和真实读数，右边是能做的操作。 */
export function QueuePanel({ tasks, selectedId, onSelect, onCancel, onOpenDir }: Props) {
  if (tasks.length === 0) {
    return (
      <p className="ef-empty">
        {T.QueuePanel.empty}
      </p>
    );
  }

  return (
    <ul className="ef-rows">
      {tasks.map((task) => {
        const running = task.status === 'running';
        const stage = task.stages[task.index - 1] ?? null;
        const files = task.report
          ? task.report.files
          : stage?.measured
            ? stage.done
            : 0;
        const bytes = task.report ? task.report.bytes : task.bytesDone;
        const duration = task.report ? task.report.duration_ms : elapsedMs(task);

        return (
          <li key={task.id} aria-current={task.id === selectedId ? 'true' : undefined}>
            <div>
              <div className="ver">
                {task.req.mc_version}
                <span className="loader">
                  {task.req.loader ? LOADER_LABEL[task.req.loader.kind] : T.common.vanilla}
                </span>
                <StateTag status={task.status} />
              </div>
              <div className="meta">
                <span>{T.QueuePanel.files(formatCount(files))}</span>
                <span>{formatBytes(bytes)}</span>
                <span>{formatDuration(duration)}</span>
                <span>{SOURCE_LABEL[task.req.source]}</span>
              </div>
              {task.status === 'failed' && task.message && (
                <div className="fail-msg">{task.message}</div>
              )}
            </div>
            <div className="acts">
              <button className="ef-btn small" type="button" onClick={() => onSelect(task.id)}>
                {T.QueuePanel.view}
              </button>
              {running ? (
                <button className="ef-btn small" type="button" onClick={onCancel}>
                  {T.common.cancel}
                </button>
              ) : (
                <button
                  className="ef-btn small"
                  type="button"
                  onClick={() => onOpenDir(task.req.game_dir)}
                >
                  {T.common.openDir}
                </button>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
