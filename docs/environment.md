# 开发环境实测记录

> 记录时间：2026-09-18
> 平台：Windows 10.0.26200 x64
> 说明：本文记录的是**实测**结果，不是推断。凡未实测的项在"未验证"小节单独列出。

## 1. 工具链核心

| 项 | 值 | 位置 |
|---|---|---|
| rustup | 1.98.1 | `C:\Users\15185\.cargo\bin` |
| rustc | 1.98.1（2026-09-01，LLVM 22.1.8） | 同上 |
| cargo | 1.98.1（2026-08-05） | 同上 |
| 工具链 | `stable-x86_64-pc-windows-msvc`（唯一，默认激活） | `C:\Users\15185\.rustup` |
| 编译目标 | `x86_64-pc-windows-msvc`、`x86_64-unknown-linux-musl` | — |
| CARGO_HOME | 未设置（默认 `C:\Users\15185\.cargo`） | — |
| RUSTUP_HOME | 未设置（默认 `C:\Users\15185\.rustup`） | — |

**已装组件（9 个）**：`rustc`、`cargo`、`rust-std`（windows-msvc 与 linux-musl 两份）、
`clippy` 0.1.98、`rustfmt` 1.9.0、`rust-analyzer` 1.98.1、`llvm-tools`、`rust-src`。

## 2. cargo 子命令与工具

| 命令 | 版本 | 用途 |
|---|---|---|
| `cargo-clippy` | 0.1.98 | 静态检查 |
| `cargo-fmt` | 1.9.0 | 格式化 |
| `rust-analyzer` | 1.98.1 | 语言服务 |
| `cargo-watch` | 8.5.3 | 改动自动重编重跑 |
| `cargo-nextest` | 0.9.145 | 测试运行器 |
| `cargo-audit` | 0.22.2（源码编译） | 依赖漏洞扫描 |
| `cargo-llvm-cov` | 0.9.1 | 覆盖率 |
| `cargo-binstall` | 1.23.0 | 拉取预编译二进制 |
| `cargo`、`rustc`、`rustdoc`、`rustup` | 1.98.1 | 本体 |

**不可用**：同目录下 `cargo-miri.exe`、`rls.exe`、`rust-gdb`、`rust-gdbgui`、`rust-lldb`
是占位代理，实际不可用（miri 需要 nightly；rls 已废弃；后三者依赖系统 gdb/lldb，本机没有）。
调试走 VS Code 的 CodeLLDB。

## 3. C/C++ 工具链（编译 `-sys` crate 用）

| 项 | 值 |
|---|---|
| Visual Studio | Community 2022，17.14.7，`D:\develop\vs2022\community` |
| MSVC 工具集 | 14.44.35207 |
| 编译器 | `...\VC\Tools\MSVC\14.44.35207\bin\Hostx64\x64\cl.exe` |
| 链接器 | 同目录 `link.exe` |
| Windows SDK | 10.0.26100.0（Include 与 Lib 齐全，`D:\Windows Kits\10`） |

不需要开 Developer 命令行——`cc` crate 会自动定位 MSVC 与 SDK，
已用真实项目验证（`build.rs` 编译 C 代码 + 链接 + 运行，全通过）。

## 4. 编辑器

VS Code 1.137.0，Rust 相关扩展：
`rust-lang.rust-analyzer` 0.3.3049、`vadimcn.vscode-lldb` 1.12.3、`fill-labs.dependi` 1.20.0。

## 5. 环境变量

用户 PATH 末尾已加 `C:\Users\15185\.cargo\bin`（注册表值类型保持 `REG_EXPAND_SZ`），
变更已广播：**新开终端直接可用**；已打开的终端需
`export PATH="$PATH:$HOME/.cargo/bin"`。除 PATH 外未改任何环境变量。

> 注意：本项目的自动化命令在 Git Bash 中执行，每次调用都是新 shell，
> 因此脚本中显式 `export PATH="$PATH:/c/Users/15185/.cargo/bin"` 更稳妥。

## 6. 磁盘

| 盘 | 可用 | 说明 |
|---|---|---|
| C: | 32 GB（94% 满） | 告急，避免放 target |
| D: | 293 GB | 充裕 |
| F: | 226 GB | 充裕，**本项目位于 F:** |

Rust `target/` 膨胀快，本项目已位于 F 盘，符合建议。

## 7. 未安装（按需再装）

- nightly 工具链（连带 `miri`、`cargo-expand`、`cargo-udeps`、`build-std` 不可用）。
- 其他架构编译目标（aarch64、windows-gnu 等）。
- crates.io 国内镜像——实测直连速度良好（17 秒下完 5 个组件），暂无必要。

## 8. 已验证事实（可复现）

- 两个目标平台各自完整编译：Windows exe 5.2 秒 / Linux musl 静态 7.2 秒。
- C 代码端到端编译、链接、运行通过。
- `clippy` 与 `fmt` 在真实项目上跑通。
- `cargo search` 可连通 crates.io（本项目 Phase 0 复测：`cargo search thevenin` 返回结果）。

## 9. 与本项目的关系

- 本项目为 **纯 Rust workspace**，不依赖 C 工具链即可构建
  （除非引入 `-sys` 类 crate；当前选型 Thevenin 为纯 Rust，见 `docs/backend-evaluation.md`）。
- 无 nightly 依赖：项目应保持在 stable 上可构建。
- 跨平台声明：**仅 Windows MSVC 实测**，Linux 目标未在本项目验证，
  文档中不得声称跨平台通过。
