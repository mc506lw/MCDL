import { useEffect, useMemo, useRef, useState } from 'react';

import { formatClock } from '../lib/format';
import type { LogLevel } from '../lib/types';
import type { LogEntry } from '../lib/useInstall';

interface Props {
  logs: LogEntry[];
  running: boolean;
  onClear: () => void;
}

type Filter = 'all' | LogLevel;

const FILTERS: Array<{ id: Filter; zh: string }> = [
  { id: 'all', zh: '全部' },
  { id: 'info', zh: '信息' },
  { id: 'warn', zh: '警告' },
  { id: 'error', zh: '错误' },
];

/**
 * 日志页：阅读类型。等宽行，时间 │ 级别 │ 一句话，细线分隔。
 * 错误和警告用黄块标出来，红色只留给删除。
 */
export function LogsPage({ logs, running, onClear }: Props) {
  const [filter, setFilter] = useState<Filter>('all');
  const [follow, setFollow] = useState(true);
  const bottom = useRef<HTMLDivElement | null>(null);

  const counts = useMemo(() => {
    const base: Record<Filter, number> = { all: logs.length, info: 0, warn: 0, error: 0 };
    for (const log of logs) base[log.level] += 1;
    return base;
  }, [logs]);

  const shown = filter === 'all' ? logs : logs.filter((log) => log.level === filter);

  // 只在跟着最新时滚，翻上去看历史就别把人家拽回来
  useEffect(() => {
    if (!follow) return;
    bottom.current?.scrollIntoView({ block: 'nearest' });
  }, [shown.length, follow]);

  return (
    <div className="wrap narrow">
      <header className="ef-sheet-head">
        <h1>
          日志 <small>Logs</small>
        </h1>
        <p className="ef-field">共 {logs.length} 条</p>
      </header>

      <p className="ef-lede ef-read">
        引擎每一次安装推上来的进度、警告和错误都按时间落在这里。
        警告和错误前面有一块黄色标记，红色只用来表示删除。
      </p>

      <div className="log-tools">
        {FILTERS.map((item) => (
          <button
            key={item.id}
            className="ef-chip"
            type="button"
            aria-pressed={filter === item.id}
            onClick={() => setFilter(item.id)}
          >
            {item.zh} <span className="n">{counts[item.id]}</span>
          </button>
        ))}

        <label className="ef-setting grow">
          <span>
            <b>跟着最新一条</b>
          </span>
          <input
            className="ef-switch"
            type="checkbox"
            role="switch"
            checked={follow}
            onChange={(event) => setFollow(event.target.checked)}
          />
        </label>

        <button className="ef-btn small danger" type="button" onClick={onClear} disabled={!logs.length}>
          清空日志
        </button>
      </div>

      {shown.length === 0 ? (
        <p className="ef-empty log-empty">
          {logs.length === 0
            ? '还没有日志。开始一次安装之后，引擎抛出的每一条进度和错误都会按时间排在这里。'
            : '这个级别下还没有记录。换一个筛选看看。'}
        </p>
      ) : (
        <ol className="logs" aria-live={running ? 'polite' : 'off'}>
          {shown.map((log) => (
            <li key={log.id} className={`lv-${log.level}${log.level === 'error' ? ' is-error' : ''}`}>
              <time className="ef-field" dateTime={new Date(log.at).toISOString()}>
                {formatClock(log.at)}
              </time>
              {log.level === 'info' ? (
                <span className="tag plain">INFO</span>
              ) : (
                <span className="ef-state is-alert tag">{log.level === 'warn' ? '警告' : '错误'}</span>
              )}
              <span className="msg">{log.message}</span>
            </li>
          ))}
        </ol>
      )}

      <div ref={bottom} />
    </div>
  );
}
