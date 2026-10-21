// 通用下拉。和 VersionPicker / DirPicker 同一套外观与键盘行为。
//
// 为什么不用原生 <select>：Windows 上它展开是系统灰底高亮 + 系统箭头，
// 和这套设计（直角、细线、切角、冰蓝填充）完全对不上。
// 同一个"从几个里选一个"的角色，在四个页面里必须长得一样。

import { useEffect, useRef, useState } from 'react';

import { T } from '../i18n';

export interface SelectOption {
  value: string;
  label: string;
  /** 副标题，例如版本号或说明 */
  hint?: string;
  disabled?: boolean;
}

interface Props {
  options: SelectOption[];
  value: string;
  onChange: (value: string) => void;
  disabled?: boolean;
  label: string;
  placeholder?: string;
}

export function Select({ options, value, onChange, disabled, label, placeholder }: Props) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const boxRef = useRef<HTMLDivElement>(null);

  const current = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    function onDown(event: MouseEvent) {
      if (!boxRef.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', onDown);
    return () => document.removeEventListener('mousedown', onDown);
  }, [open]);

  function pick(option: SelectOption) {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
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
    <div className="ef-select" ref={boxRef}>
      <button
        type="button"
        className="ef-select-trigger"
        aria-label={label}
        aria-haspopup="listbox"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        onKeyDown={onKeyDown}
      >
        <span className="ef-select-label">{current?.label ?? placeholder ?? T.Select.placeholder}</span>
        <span className="ef-select-caret" aria-hidden="true" />
      </button>

      {open && (
        <ul className="ef-select-options" role="listbox">
          {options.map((option, index) => (
            <li
              key={option.value}
              role="option"
              aria-selected={option.value === value}
              aria-disabled={option.disabled}
              className={[
                'ef-select-option',
                option.value === value ? 'is-current' : '',
                index === active ? 'is-active' : '',
                option.disabled ? 'is-disabled' : '',
              ]
                .filter(Boolean)
                .join(' ')}
              onMouseEnter={() => setActive(index)}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(option);
              }}
            >
              <span>{option.label}</span>
              {option.hint && <small>{option.hint}</small>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
