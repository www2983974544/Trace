<div align="center">
  <img src="src-tauri/icons/128x128.png" width="88" alt="Trace icon" />
</div>

# Trace

极简、本地优先的个人日记与 Gemini AI 周期复盘应用 —— 打开即写、月历打卡、多周期汇总导出与跨端数据迁移，提供 **Windows 桌面版** 与 **Android 移动版**。

[![License: PolyForm Noncommercial 1.0.0](https://img.shields.io/badge/license-PolyForm%20Noncommercial%201.0.0-blue)](LICENSE)
[![Release v1.0](https://img.shields.io/badge/Release-v1.0-3B5BDB)](https://github.com/www2983974544/Trace/releases/tag/v1.0)
[![Windows](https://img.shields.io/badge/Windows-v1.0-0078D6)](https://github.com/www2983974544/Trace/releases/tag/v1.0)
[![Android](https://img.shields.io/badge/Android-v1.0-3DDC84)](https://github.com/www2983974544/Trace/releases/tag/v1.0)

> **源码公开，禁止任何商业用途。** 采用 PolyForm Noncommercial License 1.0.0，详见 [LICENSE](LICENSE)。

## 这是什么

Trace 面向「把日常记录留给自己、需要时一键复盘」的个人写作场景：打开应用即停留在今天，无多余社交或云同步干扰；所有日记、设置与 AI 分析报告均存放在本地 SQLite 数据库中，同时支持通过单文件 `.json` 备份在 Windows 与 Android 之间自由迁移。

Windows 桌面版与 Android 移动版基于同一套 **Tauri 2 + Svelte 5 + Rust** 代码库构建，保持统一版本号（当前 `v1.0`，对应配置 `1.0.0`），界面分别针对桌面与触屏交互设计：

| | Windows 桌面版 | Android 移动版 |
| --- | --- | --- |
| 当前版本 | `v1.0`（`1.0.0`） | `v1.0`（`1.0.0`） |
| 运行环境 | Windows 10 / 11（x64，需 WebView2 运行时） | Android 7.0+（API 24+，ARM64） |
| 技术栈 | Tauri 2 + Svelte 5 + Rust + SQLite | Tauri 2 Mobile + Svelte 5 + Rust + SQLite |
| 分发包 | `.exe` 安装包（NSIS） / `.msi` 安装包 | `.apk` 安装包（Release 签名） |

## 核心能力

- **打开即写、零摩擦保存**：默认定位当天日期，支持 500ms 输入防抖、失焦与窗口隐藏自动保存；空白内容自动清理，不留空记录；内置纯文本编辑与安全过滤后的轻量 Markdown 预览切换。
- **双端独立布局**：
  - **桌面端**：左侧固定月历打卡网格与周期控制面板，右侧在「沉浸式写作」与「AI 复盘助手」之间切换。
  - **移动端**：首屏展示今日高权重卡片（含字数/预览）与近期历史列表，底部提供「今日 / 日历 / 导出 & AI」三标签导航，点击卡片进入全屏写作画布。
- **月历打卡与多周期汇总**：按月标记已记录日期圆点，支持日历跳转与历史补录；按 **日 / 周 / 月 / 年** 统计实写篇数、字符数与估算 Token 数，一键导出 `.md` / `.txt` 或复制到剪贴板。
- **Gemini AI 复盘助手**：
  - 内置日/周/月/年复盘提示词模板，支持自由编辑提示词。
  - 基于 Rust + `reqwest` 通过 SSE 流式生成分析报告，实时渲染 Markdown 并自动保存到本地历史记录。
  - 支持配置 Gemini API Key、模型预设（如 `gemini-3.8-flash` 等）与自定义代理 Base URL，内置连接测试，并在 Windows 下自动读取系统 HTTP/HTTPS 代理。
- **本地隐私与跨端迁移**：数据全部写入本地 `trace.db`（SQLite WAL 模式）；设置面板支持一键导出全量 `.json` 备份（含全部日记与 AI 报告），导入时可选「按日期智能合并（仅更新较新内容）」或「全量覆盖」。
- **主题切换**：支持跟随系统、浅色、深色三种外观。

## 项目架构

```text
Trace/
├── src/                          # Svelte 5 + TypeScript 前端界面
│   ├── App.svelte                # 桌面端与移动端双布局调度与状态管理
│   ├── components/
│   │   ├── CalendarGrid.svelte         # 月历打卡网格
│   │   ├── ImmersiveEditor.svelte      # 写作画布与 Markdown 预览
│   │   ├── PeriodActionPanel.svelte    # 周期快捷导出与入口面板
│   │   ├── AiAssistantPane.svelte      # 周期汇总、流式 Gemini 分析与历史报告
│   │   └── SettingsBackupModal.svelte  # Gemini 配置、主题与 JSON 备份恢复
│   └── lib/                      # Tauri IPC 封装、安全 Markdown 渲染、类型与提示词
├── src-tauri/                    # Rust 后端与 Tauri 配置
│   ├── src/
│   │   ├── lib.rs                # Tauri 应用初始化与命令注册
│   │   ├── commands.rs           # 前端调用命令与文件保存/移动端导出
│   │   ├── db.rs                 # SQLite 表结构、日记与设置读写
│   │   ├── export.rs             # 日/周/月/年周期范围解析与文本汇总
│   │   ├── backup.rs             # JSON 备份导出与合并/覆盖导入
│   │   └── gemini.rs             # Gemini REST / SSE 流式调用与代理适配
│   ├── capabilities/             # Tauri 权限声明
│   ├── icons/                    # 应用图标资源
│   ├── gen/android/              # Android Gradle 工程与原生配置
│   ├── Cargo.toml
│   └── tauri.conf.json
├── CHANGELOG.md                  # 版本更新记录
├── LICENSE                       # PolyForm Noncommercial License 1.0.0
└── package.json
```

## 下载与安装

前往 [GitHub Releases](https://github.com/www2983974544/Trace/releases) 下载对应平台安装包：

- **Windows**：下载 `Trace_1.0.0_x64-setup.exe` 或 `Trace_1.0.0_x64_en-US.msi` 直接安装。
- **Android**：下载 `Trace_1.0.0_arm64.apk` 安装（若提示未知来源应用请允许安装）。

## 本地开发与构建

### 环境要求
- Node.js 20+
- Rust 工具链（`rustup` / `cargo`）与 Tauri 2 系统依赖（Windows 需 WebView2 与 MSVC 构建工具）
- 如需构建 Android APK：JDK 21、Android SDK / NDK，以及 Rust Android 目标（如 `aarch64-linux-android`）

### 常用命令

```bash
# 安装前端依赖
npm install

# 前端类型检查与 Rust 单元测试
npm run check
cargo test --manifest-path src-tauri/Cargo.toml

# 启动桌面端开发模式
npm run tauri dev

# 构建 Windows 安装包（NSIS / MSI）
npm run tauri build

# 构建 Android ARM64 APK
npm run tauri android build --apk --target aarch64
```

> **Android Release 签名（本地私有配置）**：`src-tauri/gen/android/app/build.gradle.kts` 会自动检测被 `.gitignore` 忽略的 `src-tauri/gen/android/keystore.properties`。若本地配置了 `storeFile`、`storePassword`、`keyAlias`、`keyPassword`，打包将自动使用该密钥对 Release APK 签名；未配置时则生成未签名 Release 包。
>
> **Windows 开发者模式提示**：若 Windows 未开启开发者模式导致 `tauri android build` 在软链接 `libtrace_lib.so` 时报权限错误，可在 Rust 编译完成后手动将 `src-tauri/target/aarch64-linux-android/release/libtrace_lib.so` 复制至 `src-tauri/gen/android/app/src/main/jniLibs/arm64-v8a/`，再进入 `src-tauri/gen/android` 运行 `.\gradlew.bat assembleArm64Release -x rustBuildArm64Release`。

## 版本规则与更新日志

- 版本号由 Windows 与 Android 双端共享：`v1.0` → `v1.1` → `v1.2` …
- 详细变更请查看 [CHANGELOG.md](CHANGELOG.md)。

## 许可协议

本项目采用 **[PolyForm Noncommercial License 1.0.0](LICENSE)**：源码公开，允许个人学习、研究与非商业使用，**禁止任何商业用途**。
