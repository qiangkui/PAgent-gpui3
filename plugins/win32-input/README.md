# Win32 鼠标键盘

外部插件：通过 Win32 `SendInput` 执行鼠标左/右键点击、滚轮、Unicode 文字输入和按键组合。

> 这是高权限插件。每次工具调用仍需 PAgent 审批；请只在你确认目标窗口正确时批准。

## 工具

### `win32_focus`

```json
{ "handle": "0x201D2" }
```

把指定窗口恢复并切到前台，返回是否成功。也可传 `snapshot` + `element`。

### `win32_observe` + 元素操作

先由 `win32-windows` 的 `win32_observe` 取得：

```json
{ "snapshot_id": "…", "elements": [{ "index": 3, "name": "确定", "control_type": "Button", "center": { "x": 500, "y": 300 } }] }
```

然后点击元素：

```json
{
  "snapshot": "…",
  "element": 3,
  "button": "left",
  "dry_run": true
}
```

快照 45 秒过期后会拒绝操作；密码框不允许 `win32_type`，密码框点击需要 `confirm_password=true`，含删除/支付等危险词的目标需要 `confirm_dangerous=true`。

### `win32_click`

```json
{
  "handle": "0x201D2",
  "x": 120,
  "y": 30,
  "button": "left",
  "double": false
}
```

- `handle` + `x/y`：按该窗口的**客户区坐标**点击。
- 只给 `x/y`：按屏幕坐标点击。
- 都不给：在当前鼠标位置点击。
- `button`：`left` / `right` / `middle`。
- `double`：是否双击。
- `activate`：点击前是否先激活目标窗口（默认 false）。

### `win32_scroll`

```json
{ "handle": "0x201D2", "clicks": 3 }
```

- `clicks` 正数向上/向右，负数向下/向左，一个单位是一格滚轮（120）。
- 可选 `x/y`（配合 handle 时为客户区坐标）先把鼠标移到该点。

### `win32_type`

```json
{ "handle": "0x201D2", "text": "你好 PAgent\n" }
```

- 使用 `SendInput` 的 Unicode 模式，可输入中文。
- `handle` 可选；提供时会先尝试切到前台，失败则拒绝输入，避免打到错误窗口。
- `\n` 转为 Enter，`\t` 转为 Tab。

### `win32_keys`

```json
{ "handle": "0x201D2", "keys": ["CTRL", "S"] }
```

支持的键名：

`CTRL` / `SHIFT` / `ALT` / `WIN`、
`ENTER` / `TAB` / `ESC` / `SPACE` / `BACKSPACE` / `DELETE`、
`UP` / `DOWN` / `LEFT` / `RIGHT` / `HOME` / `END` / `PAGEUP` / `PAGEDOWN`、
`F1`–`F12`、单个字母或数字。

### `dry_run`

所有操作工具都接受 `"dry_run": true`，只校验句柄/坐标/按键并返回计划，不实际移动或输入。建议模型第一次先 dry run，用户确认后再执行。

### `win32_drag`

```json
{
  "snapshot": "…",
  "from_x": 300, "from_y": 240,
  "to_x": 500, "to_y": 400,
  "duration_ms": 350
}
```

在新鲜观察快照窗口内拖拽；坐标与 observe 的屏幕物理像素一致。

### `win32_wait`

等待界面加载/动画完成，默认 1000ms，最大 60000ms。等待后应重新 observe。

## 限制

- 需要 Windows 和 `python`（或 `py -3`）在 PATH 中。
- `SendInput` 只对当前桌面会话有效；管理员/UIPI 保护窗口可能拒绝输入。
- 某些游戏、安全软件、远程桌面会忽略模拟输入。
- 不检测密码框、支付或删除确认；审批前请确认目标窗口和坐标。
- 需要 Python 进程具备 DPI 感知，脚本启动时会尝试设置。
- 插件本身没有沙箱，拥有当前用户权限。