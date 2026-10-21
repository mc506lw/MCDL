import { useCallback, useEffect, useState } from 'react';

import { CommandBar } from './components/CommandBar';
import { api, IS_TAURI } from './lib/api';
import { toast, wireReadouts, wireTheme } from './lib/fx';
import { SECTIONS, type Section } from './lib/nav';
import {
  DEFAULT_SETTINGS,
  clearSettings,
  loadSettings,
  rememberDir,
  saveSettings,
  type Settings,
} from './lib/settings';
import type { InstallRequest, JavaRuntime, VersionManifest } from './lib/types';
import { useInstall } from './lib/useInstall';
import { InstallPage } from './pages/InstallPage';
import { LogsPage } from './pages/LogsPage';
import { SettingsPage } from './pages/SettingsPage';
import { VersionsPage } from './pages/VersionsPage';

export default function App() {
  const [section, setSection] = useState<Section>('install');

  // 桌面习惯：Ctrl+1…4 直接切分区。鼠标之外总得有一条更快的路。
  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (!event.ctrlKey || event.altKey || event.metaKey) return;
      const index = Number(event.key);
      if (!Number.isInteger(index) || index < 1 || index > SECTIONS.length) return;
      event.preventDefault();
      setSection(SECTIONS[index - 1].id);
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
  const install = useInstall();

  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [osDefaultDir, setOsDefaultDir] = useState('');
  const [versions, setVersions] = useState<VersionManifest | null>(null);
  const [loadingVersions, setLoadingVersions] = useState(true);
  const [versionsError, setVersionsError] = useState<string | null>(null);
  const [java, setJava] = useState<JavaRuntime[] | null>(null);
  const [javaLoading, setJavaLoading] = useState(true);
  const [javaError, setJavaError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);

  const defaultDir = settings.gameDir || osDefaultDir;

  // ------------------------------------------------------------ 取数据

  const loadVersions = useCallback(async () => {
    setLoadingVersions(true);
    setVersionsError(null);
    try {
      setVersions(await api.listVersions());
    } catch (error) {
      setVersionsError(String(error));
    } finally {
      setLoadingVersions(false);
    }
  }, []);

  const loadJava = useCallback(async () => {
    setJavaLoading(true);
    setJavaError(null);
    try {
      setJava(await api.detectJava());
    } catch (error) {
      setJavaError(String(error));
    } finally {
      setJavaLoading(false);
    }
  }, []);

  useEffect(() => {
    // wireTheme 自己在元素上做了幂等（见 lib/fx.js 的本地改动）
    wireTheme();
    void loadVersions();
    void loadJava();
    api
      .defaultGameDir()
      .then(setOsDefaultDir)
      .catch(() => setOsDefaultDir(''));
  }, [loadVersions, loadJava]);

  // 换页时让新出现的读数落定一次（[data-fx-load]）
  useEffect(() => {
    wireReadouts(document);
  }, [section]);

  // ------------------------------------------------------------ 动作

  const startInstall = useCallback(
    async (req: InstallRequest) => {
      setSelectedId(null);
      setSection('install');
      // 装过的目录自动进「常用目录」列表，下次一点就切过去
      setSettings((prev) => {
        const next = rememberDir(prev, req.game_dir);
        saveSettings(next);
        return next;
      });
      await install.start(req);
    },
    [install],
  );

  const openDir = useCallback(async (path: string) => {
    if (!path) return;
    try {
      await api.openGameDir(path);
    } catch (error) {
      toast(error instanceof Error ? error.message : String(error));
    }
  }, []);

  const saveAll = useCallback((next: Settings) => {
    setSettings(next);
    saveSettings(next);
    toast('已保存。');
  }, []);

  const resetAll = useCallback(() => {
    clearSettings();
    setSettings(DEFAULT_SETTINGS);
    toast('本机设置已清除。');
  }, []);

  // 队列里选中的那一轮；没选就跟着正在跑的那一轮
  const task =
    install.tasks.find((t) => t.id === selectedId) ?? install.running ?? install.tasks[0] ?? null;

  return (
    <>
      <div className="ef-tri ef-in-sweep" aria-hidden="true">
        <i />
        <i />
        <i />
      </div>

      <CommandBar section={section} onSection={setSection} demo={!IS_TAURI} />

      <main className="app-main">
        {section === 'install' && (
          <InstallPage
            task={task}
            tasks={install.tasks}
            selectedId={selectedId}
            onSelect={setSelectedId}
            onStart={startInstall}
            onCancel={() => void install.cancel()}
            versions={versions}
            loadingVersions={loadingVersions}
            versionsError={versionsError}
            onRetryVersions={() => void loadVersions()}
            defaultDir={defaultDir}
            settings={settings}
            java={java}
            javaLoading={javaLoading}
            javaError={javaError}
            onRefreshJava={() => void loadJava()}
            onOpenDir={openDir}
            onOpenSettings={() => setSection('settings')}
            demo={!IS_TAURI}
          />
        )}

        {section === 'versions' && (
          <VersionsPage
            gameDir={settings.gameDir || defaultDir}
            dirs={settings.dirs}
            onChangeDir={(dir) => saveAll(rememberDir(settings, dir))}
            onOpenSettings={() => setSection('settings')}
          />
        )}

        {section === 'logs' && (
          <LogsPage
            logs={install.logs}
            running={Boolean(install.running)}
            onClear={install.clearLogs}
          />
        )}

        {section === 'settings' && (
          <SettingsPage
            settings={settings}
            defaultDir={defaultDir}
            java={java}
            javaLoading={javaLoading}
            javaError={javaError}
            onRefreshJava={() => void loadJava()}
            onSave={saveAll}
            onReset={resetAll}
            onOpenDir={openDir}
          />
        )}
      </main>
    </>
  );
}
