// 命令栏的分区。中文粗体在上，英文宽体小字在下。

export const SECTIONS = [
  { id: 'install', zh: '安装', en: 'Install' },
  { id: 'versions', zh: '版本', en: 'Versions' },
  { id: 'logs', zh: '日志', en: 'Logs' },
  { id: 'settings', zh: '设置', en: 'Config' },
] as const;

export type Section = (typeof SECTIONS)[number]['id'];
