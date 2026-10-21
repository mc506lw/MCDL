import { IS_TAURI } from '../lib/api';
import { SECTIONS, type Section } from '../lib/nav';

interface Props {
  section: Section;
  onSection: (section: Section) => void;
  /** 浏览器预览时挂一个「演示数据」标记，别让演示数字冒充真实下载 */
  demo: boolean;
}

/**
 * 深色命令栏，同时也是窗口标题栏（tauri.conf.json 里 decorations: false）。
 *
 * 图标 + 中文 + 英文小字的三段式，是桌面工具的一级导航该有的分量：
 * 只有文字的小 tab 更像网页的页内锚点。当前分区是一整块强调色；
 * 拖拽区靠 data-tauri-drag-region，按钮是它自己的目标元素，点按钮不会拖窗口。
 */
export function CommandBar({ section, onSection, demo }: Props) {
  return (
    <header className="ef-cmdbar cmdbar" data-tauri-drag-region>
      <a
        className="brand"
        href="#install"
        data-tauri-drag-region
        onClick={(event) => {
          event.preventDefault();
          onSection('install');
        }}
      >
        <span className="seal" aria-hidden="true">
          M
        </span>
        <span className="brand-text" data-tauri-drag-region>
          <b>MCDL</b>
          <small>Minecraft 安装器</small>
        </span>
      </a>

      <nav aria-label="分区">
        {SECTIONS.map((item) => (
          <a
            key={item.id}
            href={`#${item.id}`}
            aria-current={section === item.id ? 'page' : undefined}
            onClick={(event) => {
              event.preventDefault();
              onSection(item.id);
            }}
          >
            <SectionIcon id={item.id} />
            <span className="nav-text">
              {item.zh}
              <small>{item.en}</small>
            </span>
          </a>
        ))}
      </nav>

      <div className="util">
        {demo && (
          <span className="ef-state is-draft demo" title="预览模式：内置演示队列">
            预览
          </span>
        )}
        <button className="ef-btn small" type="button" data-theme-toggle>
          深浅
        </button>
        {IS_TAURI && <WindowControls />}
      </div>
    </header>
  );
}

/** 分区图标。几何线条，跟着 currentColor 走，不用任何游戏素材。 */
function SectionIcon({ id }: { id: Section }) {
  const common = {
    width: 18,
    height: 18,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 1.9,
    strokeLinecap: 'square' as const,
    'aria-hidden': true,
  };

  switch (id) {
    case 'install':
      // 向下箭头落进托盘
      return (
        <svg {...common}>
          <path d="M12 3v11" />
          <path d="M7.5 10.5 12 15l4.5-4.5" />
          <path d="M4 20h16" />
        </svg>
      );
    case 'versions':
      // 层叠的版本卡片
      return (
        <svg {...common}>
          <rect x="3.5" y="4" width="17" height="5" />
          <rect x="3.5" y="10.5" width="17" height="5" />
          <path d="M3.5 17.5h17" />
        </svg>
      );
    case 'logs':
      // 文本行
      return (
        <svg {...common}>
          <path d="M5 6h14M5 11h14M5 16h9" />
        </svg>
      );
    case 'settings':
      // 方框 + 内点，表示参数
      return (
        <svg {...common}>
          <rect x="4" y="4" width="16" height="16" />
          <path d="M9 12h6M12 9v6" />
        </svg>
      );
    default:
      return null;
  }
}

async function windowAction(action: 'minimize' | 'maximize' | 'close') {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const win = getCurrentWindow();
  if (action === 'minimize') await win.minimize();
  else if (action === 'maximize') await win.toggleMaximize();
  else await win.close();
}

function WindowControls() {
  return (
    <div className="winctl" aria-label="窗口">
      <button type="button" aria-label="最小化" onClick={() => void windowAction('minimize')}>
        –
      </button>
      <button type="button" aria-label="最大化" onClick={() => void windowAction('maximize')}>
        □
      </button>
      <button type="button" aria-label="关闭" onClick={() => void windowAction('close')}>
        ✕
      </button>
    </div>
  );
}
