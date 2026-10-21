// 游戏目录选择器。安装页、版本页共用同一个，避免"同一个概念三套交互"。
//
// 交互只有三种，且顺序固定：
//   1. 从已记录的目录里选（下拉）
//   2. 浏览…（系统目录选择器，选完自动记进列表）
//   3. 打开（在文件管理器里定位）
// 「默认目录」不是独立概念——它只是列表里的一个值，由设置页负责维护。

import { useId } from 'react';

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
  const id = useId();

  // 当前值不在列表里时（用户手填过、或还没装过），把它作为一个临时选项塞进去，
  // 否则下拉会显示成空白，看起来像丢了设置。
  const options = value && !dirs.includes(value) ? [value, ...dirs] : dirs;

  async function browse() {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const picked = await open({
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

  return (
    <div className="dir-picker">
      {options.length > 0 ? (
        <select
          id={id}
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          disabled={disabled}
        >
          {options.length === 0 && <option value="">选择目录…</option>}
          {options.map((d) => (
            <option key={d} value={d}>
              {d}
            </option>
          ))}
        </select>
      ) : (
        <input
          id={id}
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder=".minecraft 所在目录"
          spellCheck={false}
          disabled={disabled}
        />
      )}

      {IS_TAURI && (
        <button className="ef-btn small" type="button" onClick={browse} disabled={disabled}>
          浏览…
        </button>
      )}

      <button
        className="ef-btn small"
        type="button"
        onClick={() => void api.openGameDir(value).catch(() => undefined)}
        disabled={disabled || !value}
        title="在文件管理器里打开"
      >
        打开
      </button>
    </div>
  );
}
