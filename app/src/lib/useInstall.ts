// 安装状态机：把引擎的事件流攒成界面要的东西。
//
// 这里只做三件事：按 `Stage.index` 排阶段、按字节算速度、按引擎给的事实算百分比。
// 百分比一律用 core-net 的兜底规则：有字节总量就看字节，没有就看文件数
// （全命中缓存时 `bytes_total` 会是 0、`done == total`，那时候看字节会一直是 0%）。

import { useCallback, useEffect, useReducer, useRef } from 'react';

import { api } from './api';
import type {
  InstallEvent,
  InstallReport,
  InstallRequest,
  LogLevel,
  StageKind,
} from './types';

export type TaskStatus = 'running' | 'done' | 'failed' | 'cancelled';

export interface StageEntry {
  /** 1 起的阶段序号，来自 `Progress::Stage.index` */
  index: number;
  kind: StageKind;
  /** 引擎给的一句话，直接显示 */
  label: string;
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  /** 收到过 Files 事件（Meta 这类阶段不会发） */
  measured: boolean;
}

export interface Task {
  id: number;
  req: InstallRequest;
  status: TaskStatus;
  startedAt: number;
  endedAt: number | null;
  /** 当前阶段序号，0 = 还没收到第一个阶段事件 */
  index: number;
  total: number;
  stages: StageEntry[];
  /** 跨阶段累计的已传输字节 */
  bytesDone: number;
  /** bytes/s，最近 1.6 秒的窗口 */
  speed: number;
  report: InstallReport | null;
  message: string | null;
  /** 算速度用的采样点，不进界面 */
  samples: Array<{ t: number; b: number }>;
}

export interface LogEntry {
  id: number;
  at: number;
  level: LogLevel;
  message: string;
  taskId: number;
}

export interface InstallState {
  tasks: Task[];
  logs: LogEntry[];
}

type Action =
  | { type: 'start'; task: Task }
  | { type: 'event'; id: number; event: InstallEvent; at: number }
  | { type: 'tick'; at: number }
  | { type: 'clear-logs' };

const SPEED_WINDOW = 1600;

function blankStage(index: number): StageEntry {
  return {
    index,
    kind: 'meta',
    label: '',
    done: 0,
    total: 0,
    bytesDone: 0,
    bytesTotal: 0,
    measured: false,
  };
}

/** 把 `task.stages` 补到 `size` 长，缺的位置留空壳，界面按 index 取。 */
function grow(stages: StageEntry[], size: number): StageEntry[] {
  const next = stages.slice();
  for (let i = next.length; i < size; i += 1) next.push(blankStage(i + 1));
  return next;
}

function sumBytes(stages: StageEntry[]): number {
  return stages.reduce((sum, s) => sum + s.bytesDone, 0);
}

function pushLog(logs: LogEntry[], taskId: number, level: LogLevel, message: string, at: number) {
  return [...logs, { id: logs.length + 1, at, level, message, taskId }].slice(-600);
}

function patch(state: InstallState, id: number, fn: (task: Task) => Task): InstallState {
  return {
    ...state,
    tasks: state.tasks.map((task) => (task.id === id ? fn(task) : task)),
  };
}

function reducer(state: InstallState, action: Action): InstallState {
  switch (action.type) {
    case 'start':
      return { tasks: [action.task, ...state.tasks].slice(0, 40), logs: state.logs };

    case 'clear-logs':
      return { ...state, logs: [] };

    case 'event': {
      const target = state.tasks.find((t) => t.id === action.id);
      if (!target || target.status !== 'running') return state;
      const { event, at } = action;

      if (event.type === 'done') {
        return patch(state, action.id, (task) => ({
          ...task,
          status: 'done',
          endedAt: at,
          report: event.report,
          bytesDone: event.report.bytes,
          speed: 0,
          index: Math.max(task.index, task.total),
        }));
      }

      if (event.type === 'failed') {
        return {
          ...patch(state, action.id, (task) => ({
            ...task,
            status: 'failed',
            endedAt: at,
            message: event.message,
            speed: 0,
          })),
          logs: pushLog(state.logs, action.id, 'error', event.message, at),
        };
      }

      if (event.type === 'cancelled') {
        return {
          ...patch(state, action.id, (task) => ({
            ...task,
            status: 'cancelled',
            endedAt: at,
            speed: 0,
          })),
          logs: pushLog(state.logs, action.id, 'warn', '已取消，已经下载的文件留在原处', at),
        };
      }

      const progress = event.progress;

      if (progress.type === 'log') {
        return {
          ...state,
          logs: pushLog(state.logs, action.id, progress.level, progress.message, at),
        };
      }

      if (progress.type === 'stage') {
        return patch(state, action.id, (task) => {
          const stages = grow(task.stages, Math.max(progress.total, progress.index));
          const slot = stages[progress.index - 1] ?? blankStage(progress.index);
          stages[progress.index - 1] = {
            ...slot,
            index: progress.index,
            kind: progress.kind,
            label: progress.label,
          };
          return { ...task, index: progress.index, total: progress.total, stages };
        });
      }

      // Files：文件数与字节数都按本阶段算，累计值自己加。
      return patch(state, action.id, (task) => {
        const stages = grow(task.stages, Math.max(task.stages.length, task.index));
        const at0 = stages.findIndex((s) => s.kind === progress.kind);
        const slot = at0 >= 0 ? stages[at0] : blankStage(task.index || 1);
        const updated: StageEntry = {
          ...slot,
          kind: progress.kind,
          done: progress.done,
          total: progress.total,
          bytesDone: progress.bytes_done,
          bytesTotal: progress.bytes_total,
          measured: true,
        };
        if (at0 >= 0) stages[at0] = updated;
        else stages.push(updated);

        const bytesDone = sumBytes(stages);
        const samples = [...task.samples, { t: at, b: bytesDone }].filter(
          (s) => at - s.t <= SPEED_WINDOW,
        );
        return { ...task, stages, bytesDone, samples };
      });
    }

    case 'tick': {
      let changed = false;
      const tasks = state.tasks.map((task) => {
        if (task.status !== 'running') return task;
        const samples = task.samples.filter((s) => action.at - s.t <= SPEED_WINDOW);
        if (samples.length < 2) {
          if (task.speed === 0 && samples.length === task.samples.length) return task;
          changed = true;
          return { ...task, samples, speed: 0 };
        }
        const first = samples[0];
        const last = samples[samples.length - 1];
        const dt = (last.t - first.t) / 1000;
        const speed = dt > 0 ? Math.max(0, (last.b - first.b) / dt) : 0;
        changed = true;
        return { ...task, samples, speed };
      });
      return changed ? { ...state, tasks } : state;
    }

    default:
      return state;
  }
}

export const INITIAL_STATE: InstallState = { tasks: [], logs: [] };

let nextTaskId = 1;

export function useInstall() {
  const [state, dispatch] = useReducer(reducer, INITIAL_STATE);
  const idRef = useRef(0);

  const running = state.tasks.find((t) => t.status === 'running') ?? null;
  // 依赖要用稳定的 id：用整个 task 对象的话，每来一条进度就重建一次定时器，
  // 400ms 的 tick 永远等不到触发。
  const runningId = running?.id ?? null;

  // 只要有任务在跑，就每 400ms 重新算一次速度：卡住的时候读数会自己掉到 0，
  // 而不是停在最后一次的数值上。
  useEffect(() => {
    if (runningId === null) return;
    const timer = window.setInterval(() => dispatch({ type: 'tick', at: Date.now() }), 400);
    return () => window.clearInterval(timer);
  }, [runningId]);

  const start = useCallback(async (req: InstallRequest) => {
    const id = nextTaskId;
    nextTaskId += 1;
    idRef.current = id;

    dispatch({
      type: 'start',
      task: {
        id,
        req,
        status: 'running',
        startedAt: Date.now(),
        endedAt: null,
        index: 0,
        total: 0,
        stages: [],
        bytesDone: 0,
        speed: 0,
        report: null,
        message: null,
        samples: [],
      },
    });

    try {
      await api.install(req, (event) => {
        dispatch({ type: 'event', id, event, at: Date.now() });
      });
    } catch (error) {
      dispatch({
        type: 'event',
        id,
        event: { type: 'failed', message: String(error) },
        at: Date.now(),
      });
    }
  }, []);

  const cancel = useCallback(async () => {
    const id = idRef.current;
    try {
      const cancelled = await api.cancel();
      if (cancelled) {
        dispatch({ type: 'event', id, event: { type: 'cancelled' }, at: Date.now() });
      }
      return cancelled;
    } catch (error) {
      dispatch({
        type: 'event',
        id,
        event: { type: 'failed', message: String(error) },
        at: Date.now(),
      });
      return false;
    }
  }, []);

  const clearLogs = useCallback(() => dispatch({ type: 'clear-logs' }), []);

  return { ...state, running, start, cancel, clearLogs };
}

// ---------------------------------------------------------------- 派生读数

/** 本阶段完成度：有字节总量看字节，没有就看文件数。 */
export function stageRatio(stage: StageEntry | null | undefined): number {
  if (!stage) return 0;
  if (stage.bytesTotal > 0) return Math.min(1, stage.bytesDone / stage.bytesTotal);
  if (stage.total > 0) return Math.min(1, stage.done / stage.total);
  return 0;
}

/** 本阶段还要多久（秒）。速度是 0 或者没有字节总量时返回 null。 */
export function stageEta(task: Task | null): number | null {
  if (!task) return null;
  const stage = task.stages[task.index - 1];
  if (!stage || stage.bytesTotal <= 0 || task.speed <= 0) return null;
  const left = stage.bytesTotal - stage.bytesDone;
  if (left <= 0) return 0;
  return left / task.speed;
}

/** 一次安装从开始到现在的用时。 */
export function elapsedMs(task: Task | null, now = Date.now()): number {
  if (!task) return 0;
  return (task.endedAt ?? now) - task.startedAt;
}
