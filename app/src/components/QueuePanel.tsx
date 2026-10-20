import { formatBytes, formatCount, formatDuration } from '../lib/format';
import { LOADER_LABEL, SOURCE_LABEL } from '../lib/types';
import { elapsedMs, type Task } from '../lib/useInstall';

interface Props {
  tasks: Task[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  onCancel: () => void;
  onOpenDir: (path: string) => void;
}

function StateTag({ status }: { status: Task['status'] }) {
  if (status === 'running') return <span className="ef-state is-alert">安装中</span>;
  if (status === 'done') return <span className="ef-state">完成</span>;
  if (status === 'failed') return <span className="ef-state is-alert">失败</span>;
  return <span className="ef-state is-draft">已取消</span>;
}

/** 安装队列：一轮安装一行，左边是版本和真实读数，右边是能做的操作。 */
export function QueuePanel({ tasks, selectedId, onSelect, onCancel, onOpenDir }: Props) {
  if (tasks.length === 0) {
    return (
      <p className="ef-empty">
        队列是空的。每装完一次，这一轮的文件数、体积和用时都会记在这里，可以随时回看。
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
                  {task.req.loader ? LOADER_LABEL[task.req.loader.kind] : '原版'}
                </span>
                <StateTag status={task.status} />
              </div>
              <div className="meta">
                <span>{formatCount(files)} 个文件</span>
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
                查看
              </button>
              {running ? (
                <button className="ef-btn small" type="button" onClick={onCancel}>
                  取消
                </button>
              ) : (
                <button
                  className="ef-btn small"
                  type="button"
                  onClick={() => onOpenDir(task.req.game_dir)}
                >
                  打开目录
                </button>
              )}
            </div>
          </li>
        );
      })}
    </ul>
  );
}
