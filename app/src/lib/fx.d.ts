// `fx.js` 的类型声明：这份脚本来自 endfield-ui 套件，保持原样不改逻辑，
// 只在这里补上 TypeScript 需要的签名。

/** 让一个元素里的数字/字母乱码后落定。 */
export function scramble(el: Element, ms?: number): void;

/** 打开页面时让 `[data-fx-load]` 的读数依次落定。 */
export function wireReadouts(root?: ParentNode): void;

/** 深浅切换：接管 `[data-theme-toggle]`，记住选择，同步 aria-pressed。 */
export function wireTheme(root?: ParentNode, options?: { reveal?: boolean }): void;

/** 指针视差：在元素上写 --mx / --my。 */
export function wireParallax(el: Element | null): void;

/** 关闭 `<dialog class="ef-drawer">` / `.ef-dialog`，等退出动效播完。 */
export function closeDialog(dialog: HTMLDialogElement, value?: string): Promise<void>;

/** 接管 `data-open` / `data-close`、Esc、点遮罩。 */
export function wireDialogs(root?: ParentNode): void;

/** 提示条：一句话，2 秒后擦掉。 */
export function toast(text: string, ms?: number): HTMLElement;

/** 列表里删掉一行：向右擦掉，下面的行补位。 */
export function removeRow(row: HTMLElement): Promise<void>;

/** 按钮「做完了」：黄色穿过一次，可以短暂换字。 */
export function flashDone(btn: HTMLElement, label?: string, ms?: number): void;
