# Agent Teams 切换记录（2026-09-18）

目标：停用本地移植的团队插件 `team-task`，改用官方实验性 Agent Teams。

## 结果（当前进程已生效，重启后仍生效）

| 项 | 状态 |
|---|---|
| 本地 `team-task`（11 个 `team_task_*` 工具） | 已停止：`team_task_plan` → `unknown tool` |
| 官方 `@deepseek-ai/dsh-experimental-agent-team`（Team 服务） | `[active]` 84c929f7 |
| 官方 `@deepseek-ai/dsh-experimental-tool-agent-team`（9 个工具） | `[active]` 7ffc418a；`team_task_list` → `{"tasks":[]}` |
| 官方 `@deepseek-ai/dsh-experimental-client-ui-agent-team`（浏览器面板） | `[active]` 71233c61，client ✓ |

## 改了什么

1. **官方包落盘**（5 个，全部 0.1.5-rc.2，与 CLI 同版本）
   `D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\`
   下新增：`dsh-experimental-agent-team`、`dsh-experimental-tool-agent-team`、
   `dsh-experimental-agent-team-profile`、`dsh-experimental-agent-team-web-profile`、
   `dsh-experimental-client-ui-agent-team`（peer 依赖解析到 CLI 树内既有同版本实例，无重复实例风险）。

2. **profile 清单** `C:\Users\15185\.dsh\profiles\web\package.json`
   - 移除依赖与 bundle 条目 `team-task`
   - 在 `@deepseek-ai/dsh-web-app` 之后按序插入
     `@deepseek-ai/dsh-experimental-agent-team-profile`、
     `@deepseek-ai/dsh-experimental-agent-team-web-profile`
   - 备份：`package.json.bak-agentteam-20260918-191604`、`cordis.patch.yml.bak-agentteam-20260918-191604`

3. **node_modules**：移除 `profiles\web\node_modules\team-task` junction
   （`dsh-team-dashboard` junction 保留，未被任何 bundle 引用，处于惰性状态）。

4. **注入器注册表** `C:\Users\15185\.dsh\super-injector\registry.json`
   - 仅保留既有 `dsh-agy`；不登记本次三个官方包，避免重启时与 bundle patch 双路径重复装配。

5. **profile patch** `C:\Users\15185\.dsh\profiles\web\cordis.patch.yml`
   - 追加 `- id: team-task / disabled: true`：压住当前进程 boot 配置树里的旧 entry。
     重启后 team-task 已不在 bundles，该条匹配不到行 → 仅 warn 并跳过（`dsh-app-boot` applyEntryPatches 语义）。

## 本次无需重启的原因

三个官方包经注入器运行时装载（junction + loader.create）；profile bundles/patch 是重启后的持久路径，
两条路径不会同时装配同一包（注册表已不含它们）。

## 下次重启后会发生什么（官方 patch 生效）

- 禁用旧全局控件 `tool-subagent-control`、`tool-subagent-list-agents`
- `tool-subagent` / `tool-subagent-fork` 改为一次性（one-shot）子代理
- 插入 `agent-team`（maxMembers 8 / maxTasks 256）与 `tool-agent-team`，web 层插入 `ui-agent-team`
- 启动日志会出现一条 `patch: entry "team-task" not found` 警告（无害）

## 回滚

```powershell
# 1) 恢复 profile 清单与 patch
Copy-Item C:\Users\15185\.dsh\profiles\web\package.json.bak-agentteam-20260918-191604 C:\Users\15185\.dsh\profiles\web\package.json -Force
Copy-Item C:\Users\15185\.dsh\profiles\web\cordis.patch.yml.bak-agentteam-20260918-191604 C:\Users\15185\.dsh\profiles\web\cordis.patch.yml -Force

# 2) 移除官方包
Remove-Item -Recurse -Force D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-experimental-agent-team
Remove-Item -Recurse -Force D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-experimental-tool-agent-team
Remove-Item -Recurse -Force D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-experimental-agent-team-profile
Remove-Item -Recurse -Force D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-experimental-agent-team-web-profile
Remove-Item -Recurse -Force D:\node_global\node_modules\@deepseek-ai\dsh\node_modules\@deepseek-ai\dsh-experimental-client-ui-agent-team
# 3) 重启 dsh web
```

## 遗留观察（与本次切换无关，但影响下次启动清爽度）

- `C:\Users\15185\.dsh\.agent-presets\deepseek-team\` 目录已空（今天 18:02 被清），
  而 profile patch 里仍 insert 其 `host-commands.js` → 下次启动该 entry 会解析失败/告警。
- `pnpm-lock.yaml` 仍含 `team-task` 条目，下次 `dsh plugin ... add` 时 pnpm 会自动收敛。
