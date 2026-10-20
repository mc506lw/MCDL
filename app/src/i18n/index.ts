// 文案入口。
//
// 用法：`import { T } from '../i18n';` 然后 `T.install.s3`。
// 直接引用对象而不是 `t('a.b.c')`：键名拼错会在编译期报错，不用等到运行时。

import { zhCN } from './zh-CN';

export const T = zhCN;
export type { Dict } from './zh-CN';

/**
 * 目前只有中文。加语言时在这里加一张表，并把 `T` 换成按设置返回的函数；
 * 在那之前 `T` 就是一个常量对象，没有运行时开销。
 */
export const LOCALES = ['zh-CN'] as const;
export type Locale = (typeof LOCALES)[number];
