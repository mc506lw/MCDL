// 游戏目录选择器。安装页、版本页、整合包页、导入页共用。
//
// 不用原生 <select>：Windows 上它带系统箭头和边框，跟这套设计（直角、细线、
// 切角表示可点击）完全对不上，而且路径太长时只会被截成一个省略号。
// 这里自绘一个下拉：触发器显示当前目录，展开后是完整路径列表 + 添加目录。

import { useEffect, useRef, useState } from 'react';

import { api, IS_TAURI } from '../lib/api';

interface Props {
  value: string;
  dirs: string[];
  onChange: (dir: string) => void;
  onPick?: (dir: string) => void;
  disabled?: boolean;
  /** 无障碍标签，例如「游戏目录」 */
  label: string;
}

export function DirPicker({ value, dirs, onChange, onPick, disabled, label }: Props) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);

  // 当前值不在列表里时也要能显示（用户手填过、或还没装过）
  const options = value && !dirs.includes(value) ? [value, ...dirs] : dirs;

  useEffect(() => {
    if (!open) return;
    function onDown(event: MouseEvent) {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  function pick(dir: string) {
    onChange(dir);
    setOpen(false);
  }

  async function browse() {
    setOpen(false);
    try {
      const { open: openDialog } = await import('@tauri-apps/plugin-dialog');
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: '选择 .minecraft 所在目录',
      });
      if (typeof picked === 'string' && picked) {
        onChange(picked);
        onPick?.(picked);
      }
    } catch {
      // 用户取消，或浏览器里没有原生对话框
    }
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (!open && (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ')) {
      event.preventDefault();
      setOpen(true);
      return;
    }
    if (event.key === 'Escape') {
      setOpen(false);
    } else if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActive((i) => Math.min(i + 1, options.length - 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActive((i) => Math.max(i - 1, 0));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const hit = options[active];
      if (hit) pick(hit);
    }
  }

  return (
    <div className="dir-picker" ref={boxRef}>
      <button
        type="button"
        className="dir-trigger"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={onKeyDown}
        title={value || '还没有选择目录'}
      >
        <span className="dir-path">{value || '选择 .minecraft 所在目录'}</span>
        <span className="dir-caret" aria-hidden="true" />
      </button>

      <button
        className="ef-btn small"
        type="button"
        onClick={() => void api.openGameDir(value).catch(() => undefined)}
        disabled={disabled || !value}
        title="在文件管理器里打开"
      >
        打开
      </button>

      {open && (
        <ul className="dir-options" role="listbox">
          {options.length === 0 && (
            <li className="dir-option is-empty">还没有记录任何目录</li>
          )}
          {options.map((dir, index) => (
            <li
              key={dir}
              role="option"
              aria-selected={dir === value}
              className={`dir-option${dir === value ? ' is-current' : ''}${
                index === active ? ' is-active' : ''
              }`}
              onMouseEnter={() => setActive(index)}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(dir);
              }}
              title={dir}
            >
              {dir}
            </li>
          ))}
          {IS_TAURI && (
            <li
              className="dir-option is-action"
              onMouseDown={(e) => {
                e.preventDefault();
                void browse();
              }}
            >
              + 添加目录…
            </li>
          )}
        </ul>
      )}
    </div>
  );
}
