// 命令栏的分区。中文粗体在上，英文宽体小字在下。
//
// 中文来自 i18n 字典（`app/src/i18n/zh-CN.ts` 的 nav 组），改文案去那里改。

import { T } from '../i18n';

export const SECTIONS = [
  { id: 'install', zh: T.nav.s1, en: 'Install' },
  { id: 'versions', zh: T.nav.s2, en: 'Versions' },
  { id: 'import', zh: T.nav.s5, en: 'Import' },
  { id: 'logs', zh: T.nav.s3, en: 'Logs' },
  { id: 'settings', zh: T.nav.s4, en: 'Config' },
] as const;

export type Section = (typeof SECTIONS)[number]['id'];
