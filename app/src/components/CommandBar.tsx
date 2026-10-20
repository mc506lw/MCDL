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
 * 当前分区是一整块强调色；拖拽区靠 data-tauri-drag-region，
 * 按钮是它自己的目标元素，所以点按钮不会拖动窗口。
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
            {item.zh}
            <small>{item.en}</small>
          </a>
        ))}
      </nav>

      <div className="util">
        {demo && (
          <span className="ef-state is-draft demo" title="浏览器预览用的是内置演示队列">
            演示数据
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
