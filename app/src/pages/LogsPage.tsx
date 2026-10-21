import { useEffect, useMemo, useRef, useState } from 'react';

import { T } from '../i18n';
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
  { id: 'all', zh: T.LogsPage.filterAll },
  { id: 'info', zh: T.LogsPage.filterInfo },
  { id: 'warn', zh: T.LogsPage.filterWarn },
  { id: 'error', zh: T.LogsPage.filterError },
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
          {T.LogsPage.title} <small>Logs</small>
        </h1>
        <p className="ef-field">{T.LogsPage.count(logs.length)}</p>
      </header>

      <p className="ef-lede ef-read">
        {T.LogsPage.lede}
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
            <b>{T.LogsPage.follow}</b>
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
          {T.LogsPage.clear}
        </button>
      </div>

      {shown.length === 0 ? (
        <p className="ef-empty log-empty">
          {logs.length === 0
            ? T.LogsPage.empty
            : T.LogsPage.emptyFiltered}
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
                <span className="ef-state is-alert tag">{log.level === 'warn' ? T.LogsPage.levelWarn : T.LogsPage.levelError}</span>
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
