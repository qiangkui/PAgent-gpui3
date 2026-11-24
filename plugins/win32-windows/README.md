# Win32 窗口检查

只读的外部插件：枚举桌面顶层窗口、按句柄展开子窗口/控件、按句柄读取详细信息。

## 工具

### `win32_list_windows`

- 省略 `parent`：列出顶层窗口。
- 传 `parent`：列出该句柄下的一层或多层子窗口。
- `depth`：展开层数（默认 1，最大 8）。
- `include_invisible`：是否包含不可见窗口/控件（默认 false；找不到时可设 true）。
- `title_contains` / `class_contains`：不区分大小写的过滤。
- `limit`：每类最多返回多少条（默认 200，最大 500）。

返回每个窗口的：

- `handle`：十六进制 HWND
- `title`、`class_name`
- `rect` / `width` / `height`
- `depth`、`parent`、`owner`
- `pid`、`thread_id`
- `visible`、`enabled`、`child_count`

### `win32_window_info`

传一个 `handle`，返回：

- 窗口位置、客户区、样式/扩展样式
- 标题、类名、父/所有者/根祖先
- 进程 id、线程 id、进程路径
- DPI、是否前台窗口
- 祖先链、后代数量

## 使用顺序

1. `win32_list_windows`（不带 parent，limit 50~100）；
2. 从结果中选择可疑窗口的 `handle`；
3. `win32_list_windows` 传 `parent=handle` 展开下一层；
4. 找到目标后调用 `win32_window_info` 获取进程、样式等详情。

## 环境与限制

- 需要 Windows 和 `python`（或 `py -3`）在 PATH 中。
- 插件只读枚举，不移动、不修改、不点击任何窗口。
- 只能看到当前桌面会话的 Win32 HWND；UWP/WinUI 的内部 XAML 控件通常不暴露为独立 HWND。
- 其他用户/管理员权限窗口可以枚举，但进程路径可能因权限读不到。
- 工具调用每次都要经过 PAgent 审批。