// 版本选择器。
//
// 原生 <select> 塞一百多个版本，滚起来没法用：不能搜索、不能键盘快速定位、
// 一屏只看得到七八个。桌面应用里这是个组合框该干的事。
//
// 交互：输入即过滤；↑↓ 移动高亮，Enter 选中，Esc 关闭并还原。

import { useEffect, useMemo, useRef, useState } from 'react';

import { T } from '../i18n';
import { formatDate } from '../lib/format';
import type { VersionSummary } from '../lib/types';

interface Props {
  versions: VersionSummary[];
  value: string;
  onChange: (id: string) => void;
  disabled?: boolean;
  /** 清单还没拉到时显示占位 */
  loading?: boolean;
  label: string;
}

const MAX_ROWS = 40;

export function VersionPicker({ versions, value, onChange, disabled, loading, label }: Props) {
  const [query, setQuery] = useState('');
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? versions.filter(
          (v) => v.id.toLowerCase().includes(q) || v.type.toLowerCase().includes(q),
        )
      : versions;
    return list.slice(0, MAX_ROWS);
  }, [versions, query]);

  // 点外面收起
  useEffect(() => {
    if (!open) return;
    function onDown(event: MouseEvent) {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  function commit(id: string) {
    onChange(id);
    setQuery('');
    setOpen(false);
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (!open && (event.key === 'ArrowDown' || event.key === 'Enter')) {
      setOpen(true);
      return;
    }
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive((i) => Math.min(i + 1, filtered.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const hit = filtered[active];
      if (hit) commit(hit.id);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      setOpen(false);
      setQuery('');
    }
  }

  const current = versions.find((v) => v.id === value);

  return (
    <div className="version-picker" ref={boxRef}>
      <input
        aria-label={label}
        role="combobox"
        aria-expanded={open}
        aria-controls="version-listbox"
        autoComplete="off"
        value={open ? query : (current ? current.id : '')}
        placeholder={loading ? T.VersionPicker.loading : T.VersionPicker.placeholder}
        disabled={disabled || loading}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onKeyDown={onKeyDown}
      />

      {open && !loading && (
        <ul id="version-listbox" role="listbox" className="version-options">
          {filtered.length === 0 && <li className="version-option is-empty">{T.VersionPicker.noMatch}</li>}
          {filtered.map((item, index) => (
            <li
              key={item.id}
              role="option"
              aria-selected={item.id === value}
              className={`version-option${index === active ? ' is-active' : ''}`}
              onMouseEnter={() => setActive(index)}
              onMouseDown={(e) => {
                e.preventDefault();
                commit(item.id);
              }}
            >
              <b>{item.id}</b>
              <small>
                {item.type}
                {item.releaseTime ? ` │ ${formatDate(item.releaseTime)}` : ''}
              </small>
            </li>
          ))}
          {filtered.length === MAX_ROWS && (
            <li className="version-option is-empty">{T.VersionPicker.moreHint}</li>
          )}
        </ul>
      )}
    </div>
  );
}
