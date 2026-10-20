import type { CSSProperties } from 'react';

import { EnvPanel } from '../components/EnvPanel';
import { InstallForm } from '../components/InstallForm';
import { QueuePanel } from '../components/QueuePanel';
import { Readout } from '../components/Readout';
import type { Settings } from '../lib/settings';
import type { InstallRequest, JavaRuntime, VersionManifest } from '../lib/types';
import type { Task } from '../lib/useInstall';

interface Props {
  task: Task | null;
  tasks: Task[];
  selectedId: number | null;
  onSelect: (id: number) => void;
  onStart: (req: InstallRequest) => void;
  onCancel: () => void;
  versions: VersionManifest | null;
  loadingVersions: boolean;
  versionsError: string | null;
  onRetryVersions: () => void;
  defaultDir: string;
  settings: Settings;
  java: JavaRuntime[] | null;
  javaLoading: boolean;
  javaError: string | null;
  onRefreshJava: () => void;
  onOpenDir: (path: string) => void;
  onOpenSettings: () => void;
  demo: boolean;
}

const step = (i: number) => ({ '--i': i }) as CSSProperties;

/**
 * 主界面：控制台类型的页面。
 * 左栏是干活的表单，右栏是这一轮安装的实时读数（主视觉），下面两栏是队列和环境。
 */
export function InstallPage({
  task,
  tasks,
  selectedId,
  onSelect,
  onStart,
  onCancel,
  versions,
  loadingVersions,
  versionsError,
  onRetryVersions,
  defaultDir,
  settings,
  java,
  javaLoading,
  javaError,
  onRefreshJava,
  onOpenDir,
  onOpenSettings,
  demo,
}: Props) {
  const running = tasks.some((t) => t.status === 'running');

  return (
    <div className="wrap">
      <header className="ef-head">
        <h1>
          安装 <small>Install</small>
        </h1>
        <p>
          选版本、加载器和下载源，点开始。右边的读数全部来自这一次真实的下载。
        </p>
      </header>

      {demo && (
        <p className="ef-notice" data-flag="演示数据">
          浏览器预览：进度来自内置的演示队列，不是真实下载。装成桌面应用之后，每个数字都来自引擎。
        </p>
      )}

      <div className="work">
        <section className="ef-panel ef-in-power" style={step(0)} aria-labelledby="setup-h">
          <h2 id="setup-h">
            安装设置 <small>Setup</small>
          </h2>
          <InstallForm
            versions={versions}
            loadingVersions={loadingVersions}
            versionsError={versionsError}
            onRetryVersions={onRetryVersions}
            defaultDir={defaultDir}
            settings={settings}
            running={running}
            onStart={onStart}
            onCancel={onCancel}
            onOpenSettings={onOpenSettings}
          />
        </section>

        <div className="stack">
          <section className="ef-panel ef-in-power hero" style={step(1)} aria-labelledby="hero-h">
            <h2 id="hero-h">
              {task && task.status !== 'running' ? '这次安装' : '当前任务'} <small>Readout</small>
            </h2>
            <Readout task={task} />
          </section>

          <section className="ef-panel ef-in-power queue" style={step(2)} aria-labelledby="queue-h">
            <h2 id="queue-h">
              安装队列 <small>{tasks.length} 轮</small>
            </h2>
            <QueuePanel
              tasks={tasks}
              selectedId={selectedId}
              onSelect={onSelect}
              onCancel={onCancel}
              onOpenDir={onOpenDir}
            />
          </section>

          <section className="ef-panel ef-in-power env" style={step(2)} aria-labelledby="env-h">
            <h2 id="env-h">
              本机环境 <small>Runtime</small>
            </h2>
            <EnvPanel
              java={java}
              loading={javaLoading}
              error={javaError}
              onRefresh={onRefreshJava}
              onOpenDir={onOpenDir}
              gameDir={task?.req.game_dir || defaultDir}
            />
          </section>
        </div>
      </div>
    </div>
  );
}
