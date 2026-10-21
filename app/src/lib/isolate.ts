// 版本隔离的**前端镜像**。
//
// 唯一真理是 `crates/mcdl-core/src/isolate.rs`：模组落点由引擎算，界面只负责
// 把同一套规则显示出来（「这个版本的模组会在哪」），以及做输入时的即时校验。
// 两边规则必须一致，所以这里刻意写成同样几个纯函数，`selfCheck()` 里带了
// 对照用例；改了一边记得改另一边。
//
// 语义提醒：隔离是**启动期**语义。MCDL 不启动游戏，它只决定装到哪，
// 能不能读到取决于用户用哪个启动器。

import { T } from '../i18n';

/** 开关旁边常驻的那句话（与 `isolate.rs::ISOLATION_NOTICE` 同一份文案）。 */
export const ISOLATION_NOTICE = T.common.isolationNotice;

/** 一次落点计算的结果。`relative` 用 `/` 分隔，不依赖平台。 */
export interface ModsTarget {
  /** 拼好的路径（沿用 gameDir 的分隔符风格） */
  dir: string;
  /** 用的是隔离目录还是共享目录 */
  isolated: boolean;
  /** 相对 gameDir 的位置，例如 `versions/fabric-loader-0.16.9-1.21.1/mods` */
  relative: string;
}

const MODS = 'mods';
const VERSIONS = 'versions';
const FORBIDDEN = new Set(['/', '\\', ':', '*', '?', '"', '<', '>', '|']);

/**
 * 版本目录名能不能安全地当一层目录用。
 *
 * 和引擎一样：单层、普通名字通过；分隔符、盘符、控制字符、`.` / `..` 拒绝。
 */
export function isValidVersionId(versionId: string): boolean {
  const id = versionId.trim();
  if (!id) return false;
  if (id === '.' || id === '..') return false;
  for (const ch of id) {
    if (FORBIDDEN.has(ch) || ch.charCodeAt(0) < 0x20) return false;
  }
  return true;
}

/** 相对 gameDir 的模组目录（给人看的，永远用 `/`）。 */
export function modsRelative(versionId: string, isolated: boolean): string {
  if (!isolated) return MODS;
  return `${VERSIONS}/${versionId.trim()}/${MODS}`;
}

/**
 * 落点计算：`<gameDir>/versions/<id>/mods`（开）或 `<gameDir>/mods`（关）。
 *
 * 隔离开着但版本名不合法时抛错——不猜、不回退到根目录。
 */
export function modsDir(gameDir: string, versionId: string, isolated: boolean): ModsTarget {
  const root = gameDir.trim().replace(/[/\\]+$/, '');
  if (!isolated) {
    return { dir: join(root, MODS), isolated: false, relative: MODS };
  }
  if (!isValidVersionId(versionId)) {
    throw new Error(T.isolate.badVersionId(JSON.stringify(versionId)));
  }
  const relative = modsRelative(versionId, true);
  return { dir: join(root, relative), isolated: true, relative };
}

/** 日志/界面里的一行：`versions/<id>/mods/` 或 `mods/`。 */
export function modsLabel(target: ModsTarget): string {
  return `${target.relative}/`;
}

/** 这次安装用的模式，一句话。 */
export function isolationSummary(isolated: boolean): string {
  return isolated ? T.common.isolationOn : T.common.isolationOff;
}

function join(root: string, relative: string): string {
  const sep = root.includes('\\') ? '\\' : '/';
  return `${root}${sep}${relative.replace(/\//g, sep)}`;
}

/**
 * 自检：用同一批用例核对前端镜像和引擎的规则。
 *
 * 手动跑：`node --experimental-strip-types -e "import('./app/src/lib/isolate.ts').then(m=>m.selfCheck())"`
 * 任何一条不一致都会抛错。
 */
export function selfCheck(): void {
  const eq = (actual: unknown, expected: unknown, what: string) => {
    if (actual !== expected) {
      throw new Error(`${what}: 期望 ${JSON.stringify(expected)}，实际 ${JSON.stringify(actual)}`);
    }
  };

  // 开：版本目录下
  const iso = modsDir('C:\\mc\\.minecraft', 'fabric-loader-0.16.9-1.21.1', true);
  eq(iso.dir, 'C:\\mc\\.minecraft\\versions\\fabric-loader-0.16.9-1.21.1\\mods', '开·路径');
  eq(iso.relative, 'versions/fabric-loader-0.16.9-1.21.1/mods', '开·相对位置');
  eq(modsLabel(iso), 'versions/fabric-loader-0.16.9-1.21.1/mods/', '开·标签');

  // 关：根目录
  const shared = modsDir('C:\\mc\\.minecraft', 'whatever', false);
  eq(shared.dir, 'C:\\mc\\.minecraft\\mods', '关·路径');
  eq(shared.relative, 'mods', '关·相对位置');

  // 关掉隔离时版本名不参与路径
  eq(modsDir('/mc', '../evil', false).dir, '/mc/mods', '关·忽略版本名');

  // 合法的特殊版本名原样保留
  for (const id of ['1.21.1', '24w14a', '我的整合包 1.20.1', '1.20.1 (优化版)']) {
    eq(modsDir('/mc', id, true).relative, `versions/${id}/mods`, `开·特殊名 ${id}`);
  }

  // 穿越与分隔符一律拒绝
  for (const id of ['..', '.', '../evil', '..\\evil', 'a/b', 'a\\b', 'C:evil', '', '  ']) {
    let threw = false;
    try {
      modsDir('/mc', id, true);
    } catch {
      threw = true;
    }
    if (!threw) throw new Error(`应该拒绝的版本名被接受了：${JSON.stringify(id)}`);
  }
}
