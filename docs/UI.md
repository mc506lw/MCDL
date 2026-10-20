# 界面

```
变体：冷库（ice #7DE6FF / cool / tight / grid / standard / full）
主视觉：安装队列 —— 版本号、加载器、体积、进度的实时读数，数字全部来自引擎的进度事件
页面类型：控制台（安装）+ 阅读（日志 / 设置）
```

变体只在**一处**设置：`app/index.html` 的 `<html>` 上

```html
<html data-ef-accent="ice" data-ef-paper="cool" data-ef-cut="tight"
      data-ef-ground="grid" data-ef-density="standard" data-ef-motion="full">
```

风格来自 `endfield-ui` skill，`app/src/styles/endfield.css` 是套件原文件（只改过 `lib/fx.js` 的两处，见文末）。
**不要在单个部件上另写颜色**，要改就改上面这一行。

## 一、三张页面

| 分区 | 页面类型 | 干什么 |
| --- | --- | --- |
| 安装 `Install` | 控制台 | 左边表单（版本 / 加载器 / 下载源 / 目录），右边这一轮的实时读数、队列、本机 Java |
| 日志 `Logs` | 阅读 | 引擎推上来的每条进度和错误，等宽行，按级别筛 |
| 设置 `Config` | 阅读 | 并发数、默认目录、Java 路径；本机探测到的运行时 |

命令栏（深色、两套主题都深色）是唯一的导航，同时也是窗口标题栏：
`tauri.conf.json` 里 `decorations: false`，靠 `data-tauri-drag-region` 拖窗口，
右边三个按钮走 `@tauri-apps/api/window`。当前分区是一整块 ice 色。

## 二、主视觉：安装队列的读数

控制台模板的中心是分段圆盘，那是"数据本来就分成几块"时才成立的东西。
这个产品真正的主视觉是**一次安装正在发生什么**，所以中间那块换成了读数：

```
┌ 当前任务 ────────────────────────────────────────────────┐
│ 1.21.8 │ 原版 │ 官方源 │ C:\Users\…\.minecraft           │ ← 真实字段，竖线分隔
│  34.6          00:00            120                      │ ← 三个大数字，宽体等宽
│  MB/s 下载速度  本阶段剩余      已下载文件 · 共 128        │
│ 02 / 05 · 下载依赖库                          093%        │ ← 阶段序号来自引擎
│ ▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▮▯▯▯  ← 24 格分段进度条            │
│ 01 已就绪 │ 02 120/128 │ 03 待执行 │ 04 待执行 │ 05 待执行 │ ← 阶段轨道
│ 累计已传输 55.7 MB / 已用时 2.6 s / 状态 / Java 运行时      │
└──────────────────────────────────────────────────────────┘
```

读数全部来自 `Progress` 事件，界面一个数都不编：

- **速度**：最近 1.6 秒的字节增量 ÷ 时间。卡住时自己掉到 0，不停在最后一次的数值上。
- **本阶段剩余**：`(bytes_total − bytes_done) ÷ 速度`。算不出来写 `—`，不猜。
- **文件数**：`Files.done / Files.total`。Meta 阶段引擎不报文件数，这一格写 `—`，
  进度条切成"不知道进度"的走格样式（`.ef-progress.is-unknown`）。
- **百分比**：`bytes_total > 0 ? bytes_done/bytes_total : done/total`。
  全命中缓存时 `bytes_total` 是 0，这时候看文件数，否则会一直显示 0%。
- **阶段序号**：直接用 `Stage.index / Stage.total`。引擎中途补插 Java 阶段时 `total` 会加一，界面跟着变。

空的时候是一句话，不是一堆 0：

> 还没有开始安装。左边选好版本和加载器，点「开始安装」，这里会出现下载速度、
> 本阶段剩余时间，以及每个阶段的文件数和字节数。

## 三、为这个产品做的三处非模板化设计

1. **主视觉换成安装队列的读数**（上面那张图）。模板的中心是分段圆盘，这里不需要：
   产品真正要回答的是"现在在下什么、还剩多少"。所以中间是三个大数字 + 24 格进度条 +
   阶段轨道，模板里的 `dial.js` 没有用。

2. **阶段轨道**（`components/StageRail.tsx`）。套件里没有这个部件，按
   `references/variation.md` 最后一节推出来的：它表示"走到第几步"，不能点 → 全部直角不切角；
   当前阶段用 ice 填充；已过的阶段墨色实心、编号反白；被取消时停住的那一格是虚线框（"没走完"）。
   阶段数和名字来自引擎的 `Stage` 事件，不是界面自己写死的 5 步。

3. **下载源的合规提示**。`docs/ARCHITECTURE.md` 要求镜像默认关闭、界面上写明「第三方镜像」。
   所以下载源是二选一的分段控件，标签直接写「BMCLAPI 第三方镜像」；切到镜像时
   下面多一条黄旗提示「BMCLAPI 是社区维护的第三方镜像，不是 Mojang 官方服务器」，
   分段下方的说明也换成「清单和版本 JSON 仍然走官方，只有依赖库、资源文件和游戏本体走镜像」。

另外两处小设计：浏览器预览时命令栏挂一个虚线框的「演示数据」标记 + 页面顶部一条提示
（避免演示数字被当成真实下载）；本机 Java 探测结果排在环境面板里，装之前先能看见。

## 四、规则的执行位置

| 规则 | 在哪儿 |
| --- | --- |
| 圆角一律 0 | 套件本身没有圆角；自写的 `.setup select`、`.settings-block input`、`.winctl button` 都是直角 |
| 强调色只做填充、只表示"当前/可点/提醒" | 当前分区（命令栏）、当前阶段（轨道）、进度条填充、黄旗提示、`.ef-btn.go` 悬停擦入 |
| 红色只给删除 | 只有「清空日志」「清除本机设置」两个按钮 |
| 切角 = 可点击 | 切角只出现在按钮、编号标签、筛选块、命令栏徽记、当前分区上；看板、输入框、阶段轨道都是直角 |
| 英文宽体大写 + 粗体中文 | `.ef-eyebrow` / `.ef-pair`：`// 安装 SETUP`、`DOWNLOADS / 下载`、`RUNTIMES / 本机 Java` |
| 细线分区、不堆卡片 | 日志、设置、看板内部全部 1px 细线；只有 `ef-panel` 这一层板 |
| 不用投影/发光/渐变 | 全站零 `box-shadow`、零渐变装饰（套件的切角用 `linear-gradient` 做遮罩，那是形状不是装饰） |
| 数字都是真的 | 见第二节；日志里的时间戳是事件到达时间，版本清单的日期来自 Mojang |
| 动效短硬机械 | 面板通电 `.ef-in-power`、三色条 `.ef-in-sweep`、按钮擦入、深浅圆形揭幕；`prefers-reduced-motion` 下全关 |
| 键盘焦点可见 | 套件自带 `:focus-visible`；命令栏按钮、阶段轨道、日志筛选都测过 Tab |

## 五、工程结构

```
app/
  index.html                 变体属性、字体预载、防白闪的主题脚本
  src/
    main.tsx                 挂载 + 引入两个样式表
    App.tsx                  分区切换、取数（清单 / Java / 默认目录）、设置
    lib/
      types.ts               与 crates/mcdl-core/src/types.rs 一一对应的协议
      api.ts                 invoke / mock 二选一的那层门
      mock.ts                浏览器演示后端（定时器推进的假进度）
      useInstall.ts          安装状态机（阶段、速度、百分比）
      settings.ts            本机设置（localStorage）
      format.ts              字节 / 速度 / 时长 / 计数
      fx.js + fx.d.ts        套件的动效函数（只改了两处，见下）
      nav.ts                 三个分区
    components/              命令栏、阶段轨道、读数、表单、队列、环境
    pages/                   安装 / 日志 / 设置
    styles/endfield.css      套件原文件（含 contours/）
    styles/app.css           字体接入 + 本产品版式
  public/fonts/Saira.ttf     自托管（OFL 1.1，许可证同目录）
  src-tauri/                 Tauri 2 命令层
```

### 浏览器里为什么能独立跑

`lib/api.ts` 里每个函数都是"检测 `window.__TAURI_INTERNALS__`，有就走 `invoke`，没有就走 mock"。
mock（`lib/mock.ts`）用定时器按真实阶段顺序推进，事件结构（`stage` / `files` / `log`，
`Stage.index` 从 1 起、`total` 是本轮阶段数）和 Rust 那边完全一致，
所以接上真后端时界面代码一行都不用改。

`pnpm dev` 打开 `http://localhost:1420/` 就能看到完整界面；命令栏会挂「演示数据」标记。

## 六、Tauri 命令层

`app/src-tauri/src/lib.rs`，只有六个命令，不写任何下载逻辑：

| 命令 | 干什么 |
| --- | --- |
| `list_versions` | `meta::fetch_manifest`，清单永远走官方源 |
| `list_loaders` | `loader::available_loader_versions`；最多 20 条、从新到旧、5 分钟缓存，单次 ≤10 秒 |
| `install` | 起一个任务跑 `Installer::run`，进度走 `Channel<InstallEvent>` |
| `cancel` | abort 掉那个 future |
| `detect_java` | `java::detect()` |
| `list_runtimes` | 列 `<gameDir>/runtime/jre-<major>/bin/java.exe`，只读目录 |
| `default_game_dir` | 系统默认的 `.minecraft` |
| `open_game_dir` | 文件管理器打开目录（`tauri-plugin-opener`） |

### 加载器版本下拉的降级

`list_loaders` 会超时（单次 ≤10 秒）也可能失败，界面不能因此卡住：

- 拉的时候显示骨架条；
- 成功：下拉里是「最新版（默认）」+ 最多 20 个版本，从新到旧，不做无限滚动；
- 失败或 10 秒超时：黄旗提示「无法获取版本列表，将使用最新版。」，
  并把下拉换成文本框，用户想手填还是可以填，直接点开始就是用最新版。

## 六、第三方镜像的标注（合规底线）

镜像只是加速，默认关闭，而且必须写明是第三方的。界面上有两处：

1. **BMCLAPI 下载源**：分段控件的标签直接写「BMCLAPI 第三方镜像」；选中时下面多一条
   黄旗提示「BMCLAPI 是社区维护的第三方镜像，不是 Mojang 官方服务器。」，
   分段下方的说明也改成「清单和版本 JSON 仍然走官方，只有依赖库、资源文件和游戏本体走镜像。」
2. **Java 运行时的清华镜像回退**：本机没有合适运行时、官方 Adoptium 又失败时，
   引擎会退到清华镜像取同一份 Temurin 构建。设置页「目录与运行时」下面常驻一条黄旗提示：
   「Java 运行时：官方 Adoptium 下载失败时，会改用清华镜像（第三方）获取同一份 Temurin 构建。」；
   安装页的「本机环境」看板里也有一句短版。

两处都用 `.ef-notice` 的黄旗，不用红色（红色只给删除）。

进度用 `tauri::ipc::Channel<InstallEvent>`，`InstallEvent` 是 `progress` / `done` / `failed` / `cancelled`
四种。`Progress` 原样嵌套，不改字段——契约在 `types.rs`。

取消 = `abort()` 掉 JoinHandle：引擎没有取消令牌，但所有下载都在 await 点上，
drop 掉 future 就是最干净的取消；已经落盘的文件留着，下次按 SHA1 复用。

### 还没接上的真实后端

| 位置 | 现在 | 接上之后 |
| --- | --- | --- |
| `crates/mcdl-core` 的 `meta` / `net` / `download` / `loader` / `java` | 已实现（并行开发中），`cargo check` 过 | 不用改界面 |
| 设置持久化 | `localStorage`（`mcdl.settings`） | 换成写 `<config>/mcdl/settings.json`；`settings.ts` 的三个函数是唯一入口 |
| 选择游戏目录 | 文本框 + 「默认目录」按钮 | 接 `tauri-plugin-dialog` 的目录选择框 |
| 安装历史 | 只在内存里，关掉就没了 | 落一份 `history.json`，或从 `versions/` 扫 |
| 版本清单缓存 | 每次打开都拉 | 落盘 + `If-None-Match` |
| CSP | `tauri.conf.json` 里是 `null` | 写死 `default-src 'self'`（dev 的 HMR websocket 需要单独放行） |

已经接上的：`list_versions`、`list_loaders`、`install`、`cancel`、`detect_java`、
`list_runtimes`、`default_game_dir`、`open_game_dir`。浏览器里这些全部有结构一致的 mock，
所以 `pnpm dev` 不需要 Rust 也能把三张页面点一遍。

## 七、验收

```
cd app && pnpm install && pnpm build      # tsc + vite build，零错误
cd app && pnpm dev                        # 浏览器里完整可交互（mock）
cargo check -p mcdl-app                   # 命令层能编译
```

版式自查用无头 Chromium 真的点了一遍（`app/.shots/shoot.mjs`，本地工具，不进仓库）：

- 1440×900 与 960×900 都没有横向滚动（`documentElement.scrollWidth === innerWidth`）
- 960 宽仍是两栏（`368px 495.219px`），低于 940 才并成一栏
- 完整跑完一轮安装（3,150 个文件 / 732 MB / 10.5 s），再跑一轮中途取消（「已取消」，轨道上那一格变虚线）
- 日志页 9 条、设置页 2 个 Java 运行时
- 浅色深色都看过；`Saira` 字体确认 `document.fonts` 里是 `loaded`
- 全站 `borderRadius` 非 0 的元素：0 个；`box-shadow` 非 none 的元素：0 个
- 控制台零报错、零未捕获异常、零失败请求

## 八、对套件文件的两处本地改动

`app/src/lib/fx.js` 是套件原文件，只动了两行，都写在注释里：

1. `wireTheme()` 里给按钮打了 `data-ef-theme-wired` 标记，保证只接一次监听。
   这个函数没有解绑，React 的 StrictMode / Fast Refresh 会把 effect 跑两遍，
   重复监听会让一次点击翻两次主题（看起来像没反应）。
2. View Transition 的 `ready` 加了 `.catch()`。连续点两次时旧的那次会被 skip，
   `ready` 会 reject，不接住就是一条未处理的 promise 拒绝。
