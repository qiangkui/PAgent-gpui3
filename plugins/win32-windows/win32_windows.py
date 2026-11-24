#!/usr/bin/env python3
"""Read-only Win32 window enumeration for the PAgent plugin.

The plugin passes one JSON object as argv[1] and receives one JSON object on
stdout. Only user32/kernel32 read calls are made: nothing is moved, clicked or
changed.
"""

from __future__ import annotations

import ctypes
import json
import sys
from ctypes import wintypes

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

GW_CHILD = 5
GW_HWNDNEXT = 2
GW_OWNER = 4
GA_ROOT = 2
WM_GETTEXT = 0x000D
SMTO_ABORTIFHUNG = 0x0002
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000

WNDENUMPROC = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)


class RECT(ctypes.Structure):
    _fields_ = [
        ("left", wintypes.LONG),
        ("top", wintypes.LONG),
        ("right", wintypes.LONG),
        ("bottom", wintypes.LONG),
    ]


class POINT(ctypes.Structure):
    _fields_ = [("x", wintypes.LONG), ("y", wintypes.LONG)]


user32.EnumWindows.argtypes = [WNDENUMPROC, wintypes.LPARAM]
user32.EnumWindows.restype = wintypes.BOOL
user32.EnumChildWindows.argtypes = [wintypes.HWND, WNDENUMPROC, wintypes.LPARAM]
user32.EnumChildWindows.restype = wintypes.BOOL
user32.IsWindow.argtypes = [wintypes.HWND]
user32.IsWindow.restype = wintypes.BOOL
user32.IsWindowVisible.argtypes = [wintypes.HWND]
user32.IsWindowVisible.restype = wintypes.BOOL
user32.IsWindowEnabled.argtypes = [wintypes.HWND]
user32.IsWindowEnabled.restype = wintypes.BOOL
user32.GetParent.argtypes = [wintypes.HWND]
user32.GetParent.restype = wintypes.HWND
user32.GetWindow.argtypes = [wintypes.HWND, wintypes.UINT]
user32.GetWindow.restype = wintypes.HWND
user32.GetForegroundWindow.argtypes = []
user32.GetForegroundWindow.restype = wintypes.HWND
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
user32.GetWindowThreadProcessId.restype = wintypes.DWORD
user32.GetWindowTextLengthW.argtypes = [wintypes.HWND]
user32.GetWindowTextLengthW.restype = ctypes.c_int
user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetClassNameW.restype = ctypes.c_int
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(RECT)]
user32.GetWindowRect.restype = wintypes.BOOL
user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(RECT)]
user32.GetClientRect.restype = wintypes.BOOL
user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(POINT)]
user32.ClientToScreen.restype = wintypes.BOOL
user32.GetWindowLongW.argtypes = [wintypes.HWND, ctypes.c_int]
user32.GetWindowLongW.restype = ctypes.c_long
user32.SendMessageTimeoutW.argtypes = [
    wintypes.HWND,
    wintypes.UINT,
    wintypes.WPARAM,
    ctypes.c_void_p,
    wintypes.UINT,
    wintypes.UINT,
    ctypes.POINTER(ctypes.c_size_t),
]
user32.SendMessageTimeoutW.restype = ctypes.c_size_t

kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = [
    wintypes.HANDLE,
    wintypes.DWORD,
    wintypes.LPWSTR,
    ctypes.POINTER(wintypes.DWORD),
]
kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL

try:
    user32.GetDpiForWindow.argtypes = [wintypes.HWND]
    user32.GetDpiForWindow.restype = wintypes.UINT
except AttributeError:
    user32.GetDpiForWindow = None


def hwnd_text(handle) -> str | None:
    """One stable hex spelling; the model passes it back unchanged."""
    value = int(handle or 0)
    return f"0x{value:X}" if value else None


def text_of(hwnd) -> str:
    """Window text with a short timeout, so a hung app cannot hang the scan."""
    size = min(max(user32.GetWindowTextLengthW(hwnd) + 1, 256), 4096)
    buffer = ctypes.create_unicode_buffer(size)
    result = ctypes.c_size_t(0)
    ok = user32.SendMessageTimeoutW(
        hwnd,
        WM_GETTEXT,
        size,
        ctypes.cast(buffer, ctypes.c_void_p),
        SMTO_ABORTIFHUNG,
        120,
        ctypes.byref(result),
    )
    return buffer.value if ok else ""


def class_of(hwnd) -> str:
    buffer = ctypes.create_unicode_buffer(512)
    user32.GetClassNameW(hwnd, buffer, len(buffer))
    return buffer.value


def rect_of(hwnd) -> dict | None:
    rect = RECT()
    if not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return None
    return {
        "left": rect.left,
        "top": rect.top,
        "right": rect.right,
        "bottom": rect.bottom,
        "width": max(0, rect.right - rect.left),
        "height": max(0, rect.bottom - rect.top),
    }


def client_rect_of(hwnd) -> dict | None:
    rect = RECT()
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)):
        return None
    origin = POINT(rect.left, rect.top)
    user32.ClientToScreen(hwnd, ctypes.byref(origin))
    return {
        "left": origin.x,
        "top": origin.y,
        "width": max(0, rect.right - rect.left),
        "height": max(0, rect.bottom - rect.top),
    }


def process_path(pid: int) -> str:
    if not pid:
        return ""
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return ""
    try:
        size = wintypes.DWORD(2048)
        buffer = ctypes.create_unicode_buffer(size.value)
        if kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            return buffer.value
        return ""
    finally:
        kernel32.CloseHandle(handle)


def parent_of(hwnd) -> int:
    return int(user32.GetParent(hwnd) or 0)


def owner_of(hwnd) -> int:
    return int(user32.GetWindow(hwnd, GW_OWNER) or 0)


def child_count(hwnd) -> int:
    count = 0
    child = user32.GetWindow(hwnd, GW_CHILD)
    guard = 0
    while child and guard < 20_000:
        count += 1
        guard += 1
        child = user32.GetWindow(child, GW_HWNDNEXT)
    return count


def basic(hwnd, depth: int, z_index: int | None = None) -> dict:
    pid = wintypes.DWORD(0)
    thread = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    item = {
        "handle": hwnd_text(hwnd),
        "title": text_of(hwnd),
        "class_name": class_of(hwnd),
        "rect": rect_of(hwnd),
        "depth": depth,
        "parent": hwnd_text(parent_of(hwnd)),
        "owner": hwnd_text(owner_of(hwnd)),
        "pid": pid.value,
        "thread_id": int(thread),
        "visible": bool(user32.IsWindowVisible(hwnd)),
        "enabled": bool(user32.IsWindowEnabled(hwnd)),
        "child_count": child_count(hwnd),
    }
    if z_index is not None:
        item["z_index"] = z_index
    return item


def matches(item: dict, title: str, klass: str) -> bool:
    if title and title.lower() not in item["title"].lower():
        return False
    if klass and klass.lower() not in item["class_name"].lower():
        return False
    return True


def relative_depth(hwnd, root: int) -> int:
    current = int(hwnd)
    depth = 0
    while current and current != root and depth <= 128:
        current = parent_of(current)
        depth += 1
    return depth if current == root else 0


def handle_value(value) -> int:
    text = str(value or "").strip()
    return int(text, 0) if text else 0


def hex_style(value: int) -> str:
    return f"0x{int(value) & 0xFFFFFFFF:08X}"


def ancestor_chain(hwnd) -> list[dict]:
    items = []
    current = parent_of(hwnd)
    guard = 0
    while current and guard < 128:
        items.append(
            {
                "handle": hwnd_text(current),
                "title": text_of(current),
                "class_name": class_of(current),
            }
        )
        guard += 1
        current = parent_of(current)
    items.reverse()
    return items


def root_ancestor(hwnd) -> int:
    current = int(hwnd)
    guard = 0
    while guard < 128:
        parent = parent_of(current)
        if not parent:
            return current
        current = parent
        guard += 1
    return current


def descendant_count(hwnd) -> tuple[int, bool]:
    count = 0
    capped = False

    @WNDENUMPROC
    def callback(_child, _param):
        nonlocal count, capped
        count += 1
        if count >= 10_000:
            capped = True
            return False
        return True

    user32.EnumChildWindows(hwnd, callback, 0)
    return count, capped


def children_preview(hwnd, limit: int = 20) -> list[dict]:
    items = []
    child = user32.GetWindow(hwnd, GW_CHILD)
    guard = 0
    while child and guard < limit:
        items.append(
            {
                "handle": hwnd_text(child),
                "title": text_of(child),
                "class_name": class_of(child),
                "visible": bool(user32.IsWindowVisible(child)),
            }
        )
        guard += 1
        child = user32.GetWindow(child, GW_HWNDNEXT)
    return items


def list_top(payload: dict) -> dict:
    limit = max(1, min(500, int(payload.get("limit", 200))))
    include_invisible = payload.get("include_invisible") is True
    title = str(payload.get("title_contains", "") or "")
    klass = str(payload.get("class_contains", "") or "")
    items: list[dict] = []
    seen = 0
    truncated = False

    @WNDENUMPROC
    def callback(hwnd, _param):
        nonlocal seen, truncated
        seen += 1
        if not include_invisible and not user32.IsWindowVisible(hwnd):
            return True
        item = basic(hwnd, 0, seen - 1)
        if not matches(item, title, klass):
            return True
        if len(items) >= limit:
            truncated = True
            return False
        items.append(item)
        return True

    user32.EnumWindows(callback, 0)
    return {
        "ok": True,
        "action": "top",
        "count": len(items),
        "seen": seen,
        "truncated": truncated,
        "include_invisible": include_invisible,
        "windows": items,
        "hint": "传 parent=<handle> 展开子窗口；传 handle 给 win32_window_info 获取详情。",
    }


def list_children(payload: dict) -> dict:
    root = handle_value(payload.get("parent"))
    if not root or not user32.IsWindow(root):
        raise ValueError(f"不是有效窗口句柄：{payload.get('parent')}")
    depth_limit = max(1, min(8, int(payload.get("depth", 1))))
    limit = max(1, min(500, int(payload.get("limit", 200))))
    include_invisible = payload.get("include_invisible") is True
    title = str(payload.get("title_contains", "") or "")
    klass = str(payload.get("class_contains", "") or "")
    items: list[dict] = []
    seen = 0
    truncated = False

    @WNDENUMPROC
    def callback(hwnd, _param):
        nonlocal seen, truncated
        seen += 1
        depth = relative_depth(hwnd, root)
        if depth == 0 or depth > depth_limit:
            return True
        if not include_invisible and not user32.IsWindowVisible(hwnd):
            return True
        item = basic(hwnd, depth)
        if not matches(item, title, klass):
            return True
        if len(items) >= limit:
            truncated = True
            return False
        items.append(item)
        return True

    user32.EnumChildWindows(root, callback, 0)
    return {
        "ok": True,
        "action": "children",
        "root": basic(root, 0),
        "depth": depth_limit,
        "count": len(items),
        "seen": seen,
        "truncated": truncated,
        "include_invisible": include_invisible,
        "windows": items,
        "hint": "对结果里的 handle 继续传 parent 可查看更下一层；win32_window_info 看详情。",
    }


def window_info(payload: dict) -> dict:
    target = handle_value(payload.get("handle"))
    if not target or not user32.IsWindow(target):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    item = basic(target, 0)
    item["client_rect"] = client_rect_of(target)
    item["style"] = hex_style(user32.GetWindowLongW(target, -16))
    item["ex_style"] = hex_style(user32.GetWindowLongW(target, -20))
    item["process_path"] = process_path(item["pid"])
    item["dpi"] = int(user32.GetDpiForWindow(target)) if user32.GetDpiForWindow else 0
    item["foreground"] = int(user32.GetForegroundWindow() or 0) == target
    item["root"] = hwnd_text(root_ancestor(target))
    item["ancestors"] = ancestor_chain(target)
    descendants, capped = descendant_count(target)
    item["descendant_count"] = descendants
    item["descendant_count_capped"] = capped
    item["children_preview"] = children_preview(target)
    return {
        "ok": True,
        "action": "info",
        "window": item,
        "hint": "children_preview 只是前 20 个；用 win32_list_windows(parent=handle) 查看完整子级。",
    }


def main() -> None:
    try:
        payload = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}
        action = str(payload.get("action", "top"))
        if action == "top":
            result = list_top(payload)
        elif action == "children":
            result = list_children(payload)
        elif action == "info":
            result = window_info(payload)
        else:
            raise ValueError(f"未知 action：{action}")
    except Exception as error:  # noqa: BLE001 - the JSON error is the API
        result = {"ok": False, "error": str(error)}
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()