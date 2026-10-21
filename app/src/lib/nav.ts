// 命令栏的分区。中文粗体在上，英文宽体小字在下。
//
// 中文来自 i18n 字典（`app/src/i18n/zh-CN.ts` 的 nav 组），改文案去那里改。

import { T } from '../i18n';

export const SECTIONS = [
  { id: 'install', zh: T.nav.install, en: 'Install' },
  { id: 'versions', zh: T.nav.versions, en: 'Versions' },
  { id: 'packs', zh: T.nav.packs, en: 'Modpacks' },
  { id: 'import', zh: T.nav.import, en: 'Import' },
  { id: 'logs', zh: T.nav.logs, en: 'Logs' },
  { id: 'settings', zh: T.nav.settings, en: 'Config' },
] as const;

export type Section = (typeof SECTIONS)[number]['id'];
