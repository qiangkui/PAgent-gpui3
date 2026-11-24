#!/usr/bin/env python3
"""Mouse and keyboard input through Win32 SendInput.

The PAgent plugin sends one JSON object as argv[1] and receives one JSON object.
Every action supports `dry_run`, which validates handles/coordinates/keys but
does not inject input.
"""

from __future__ import annotations

import ctypes
import json
import sys
import time
from ctypes import wintypes

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    try:
        user32.SetProcessDPIAware()
    except Exception:
        pass

INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_RIGHTDOWN = 0x0008
MOUSEEVENTF_RIGHTUP = 0x0010
MOUSEEVENTF_MIDDLEDOWN = 0x0020
MOUSEEVENTF_MIDDLEUP = 0x0040
MOUSEEVENTF_WHEEL = 0x0800
MOUSEEVENTF_HWHEEL = 0x1000
SW_RESTORE = 9
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
WHEEL_DELTA = 120

ULONG_PTR = ctypes.c_size_t
DWORD = wintypes.DWORD


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wintypes.LONG),
        ("dy", wintypes.LONG),
        ("mouseData", DWORD),
        ("dwFlags", DWORD),
        ("time", DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wintypes.WORD),
        ("wScan", wintypes.WORD),
        ("dwFlags", DWORD),
        ("time", DWORD),
        ("dwExtraInfo", ULONG_PTR),
    ]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [
        ("uMsg", DWORD),
        ("wParamL", wintypes.WORD),
        ("wParamH", wintypes.WORD),
    ]


class INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", DWORD), ("union", INPUTUNION)]


class RECT(ctypes.Structure):
    _fields_ = [
        ("left", wintypes.LONG),
        ("top", wintypes.LONG),
        ("right", wintypes.LONG),
        ("bottom", wintypes.LONG),
    ]


class POINT(ctypes.Structure):
    _fields_ = [("x", wintypes.LONG), ("y", wintypes.LONG)]


user32.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.SendInput.restype = wintypes.UINT
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.SetCursorPos.restype = wintypes.BOOL
user32.GetCursorPos.argtypes = [ctypes.POINTER(POINT)]
user32.GetCursorPos.restype = wintypes.BOOL
user32.IsWindow.argtypes = [wintypes.HWND]
user32.IsWindow.restype = wintypes.BOOL
user32.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(RECT)]
user32.GetWindowRect.restype = wintypes.BOOL
user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(RECT)]
user32.GetClientRect.restype = wintypes.BOOL
user32.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(POINT)]
user32.ClientToScreen.restype = wintypes.BOOL
user32.GetForegroundWindow.argtypes = []
user32.GetForegroundWindow.restype = wintypes.HWND
user32.SetForegroundWindow.argtypes = [wintypes.HWND]
user32.SetForegroundWindow.restype = wintypes.BOOL
user32.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
user32.ShowWindow.restype = wintypes.BOOL
user32.GetWindowTextLengthW.argtypes = [wintypes.HWND]
user32.GetWindowTextLengthW.restype = ctypes.c_int
user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetWindowTextW.restype = ctypes.c_int
user32.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
user32.GetClassNameW.restype = ctypes.c_int
user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(DWORD)]
user32.GetWindowThreadProcessId.restype = DWORD
kernel32.OpenProcess.argtypes = [DWORD, wintypes.BOOL, DWORD]
kernel32.OpenProcess.restype = wintypes.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = [
    wintypes.HANDLE,
    DWORD,
    wintypes.LPWSTR,
    ctypes.POINTER(DWORD),
]
kernel32.QueryFullProcessImageNameW.restype = wintypes.BOOL
kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
kernel32.CloseHandle.restype = wintypes.BOOL


def handle_value(value) -> int:
    text = str(value or "").strip()
    return int(text, 0) if text else 0


def hwnd_text(handle) -> str | None:
    value = int(handle or 0)
    return f"0x{value:X}" if value else None


def window_title(hwnd) -> str:
    size = min(max(user32.GetWindowTextLengthW(hwnd) + 1, 256), 4096)
    buffer = ctypes.create_unicode_buffer(size)
    user32.GetWindowTextW(hwnd, buffer, size)
    return buffer.value


def window_class(hwnd) -> str:
    buffer = ctypes.create_unicode_buffer(512)
    user32.GetClassNameW(hwnd, buffer, len(buffer))
    return buffer.value


def window_rect(hwnd) -> dict | None:
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


def client_size(hwnd) -> tuple[int, int]:
    rect = RECT()
    if not user32.GetClientRect(hwnd, ctypes.byref(rect)):
        return (0, 0)
    return (max(0, rect.right - rect.left), max(0, rect.bottom - rect.top))


def process_path(pid: int) -> str:
    if not pid:
        return ""
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, pid)
    if not handle:
        return ""
    try:
        size = DWORD(2048)
        buffer = ctypes.create_unicode_buffer(size.value)
        if kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            return buffer.value
        return ""
    finally:
        kernel32.CloseHandle(handle)


def target_info(hwnd) -> dict:
    pid = DWORD(0)
    thread = user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return {
        "handle": hwnd_text(hwnd),
        "title": window_title(hwnd),
        "class_name": window_class(hwnd),
        "rect": window_rect(hwnd),
        "pid": pid.value,
        "thread_id": int(thread),
        "process_path": process_path(pid.value),
        "foreground": int(user32.GetForegroundWindow() or 0) == int(hwnd),
    }


def cursor_pos() -> tuple[int, int]:
    point = POINT()
    user32.GetCursorPos(ctypes.byref(point))
    return (point.x, point.y)


def focus_window(hwnd) -> bool:
    user32.ShowWindow(hwnd, SW_RESTORE)
    user32.SetForegroundWindow(hwnd)
    for _ in range(12):
        if int(user32.GetForegroundWindow() or 0) == int(hwnd):
            return True
        time.sleep(0.03)
    return False


def send_inputs(events: list[INPUT]) -> int:
    if not events:
        return 0
    array = (INPUT * len(events))(*events)
    return int(user32.SendInput(len(events), array, ctypes.sizeof(INPUT)))


def mouse_event(flags: int, data: int = 0) -> INPUT:
    item = INPUT()
    item.type = INPUT_MOUSE
    item.union.mi = MOUSEINPUT(0, 0, ctypes.c_uint32(data & 0xFFFFFFFF).value, flags, 0, 0)
    return item


def key_event(vk: int = 0, scan: int = 0, flags: int = 0) -> INPUT:
    item = INPUT()
    item.type = INPUT_KEYBOARD
    item.union.ki = KEYBDINPUT(vk, scan, flags, 0, 0)
    return item


def click_events(button: str, double: bool) -> list[INPUT]:
    down, up = {
        "left": (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        "right": (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        "middle": (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
    }[button]
    events = []
    for _ in range(2 if double else 1):
        events.append(mouse_event(down))
        events.append(mouse_event(up))
    return events


def resolve_point(handle: int, x, y) -> tuple[int, int] | None:
    has_x = x is not None
    has_y = y is not None
    if not has_x and not has_y:
        return None
    if has_x != has_y:
        raise ValueError("x 和 y 必须同时提供")
    px, py = int(x), int(y)
    if handle:
        width, height = client_size(handle)
        if px < 0 or py < 0 or px >= width or py >= height:
            raise ValueError(f"客户区坐标 ({px},{py}) 超出 {width}x{height}")
        point = POINT(px, py)
        if not user32.ClientToScreen(handle, ctypes.byref(point)):
            raise ValueError("无法把客户区坐标转换成屏幕坐标")
        return (point.x, point.y)
    return (px, py)


def dry_run(payload: dict) -> bool:
    return payload.get("dry_run") is True


def action_focus(payload: dict) -> dict:
    handle = handle_value(payload.get("handle"))
    if not handle or not user32.IsWindow(handle):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    info = target_info(handle)
    if dry_run(payload):
        return {"ok": True, "action": "focus", "dry_run": True, "window": info}
    if not focus_window(handle):
        raise RuntimeError("无法把窗口切到前台；Windows 前台锁或其他程序阻止了切换")
    info = target_info(handle)
    return {"ok": True, "action": "focus", "dry_run": False, "window": info}


def action_click(payload: dict) -> dict:
    handle = handle_value(payload.get("handle"))
    if handle and not user32.IsWindow(handle):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    button = str(payload.get("button", "left") or "left").lower()
    if button not in ("left", "right", "middle"):
        raise ValueError(f"button 必须是 left / right / middle，收到 {button}")
    double = payload.get("double") is True
    activate = payload.get("activate") is True
    point = resolve_point(handle, payload.get("x"), payload.get("y"))
    if activate and handle and not dry_run(payload) and not focus_window(handle):
        raise RuntimeError("无法先激活目标窗口，点击没有执行")
    before = cursor_pos()
    planned = point or before
    if not dry_run(payload):
        if not user32.SetCursorPos(planned[0], planned[1]):
            raise RuntimeError("无法移动鼠标（可能是安全桌面/权限问题）")
        sent = send_inputs(click_events(button, double))
        expected = 4 if double else 2
        if sent != expected:
            raise RuntimeError(f"SendInput 只接受 {sent}/{expected} 个鼠标事件，可能被 UIPI 阻止")
    return {
        "ok": True,
        "action": "click",
        "dry_run": dry_run(payload),
        "button": button,
        "double": double,
        "position": {"x": planned[0], "y": planned[1]},
        "cursor_before": {"x": before[0], "y": before[1]},
        "target": target_info(handle) if handle else None,
    }


def action_scroll(payload: dict) -> dict:
    handle = handle_value(payload.get("handle"))
    if handle and not user32.IsWindow(handle):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    clicks = int(payload.get("clicks", 0))
    if clicks == 0 or abs(clicks) > 100:
        raise ValueError("clicks 必须在 -100 到 100 之间且不能为 0")
    horizontal = payload.get("horizontal") is True
    point = resolve_point(handle, payload.get("x"), payload.get("y"))
    if point and not dry_run(payload) and not user32.SetCursorPos(point[0], point[1]):
        raise RuntimeError("无法移动鼠标到滚动位置")
    if not dry_run(payload):
        flags = MOUSEEVENTF_HWHEEL if horizontal else MOUSEEVENTF_WHEEL
        sent = send_inputs([mouse_event(flags, clicks * WHEEL_DELTA)])
        if sent != 1:
            raise RuntimeError(f"SendInput 只接受 {sent}/1 个滚轮事件，可能被 UIPI 阻止")
    return {
        "ok": True,
        "action": "scroll",
        "dry_run": dry_run(payload),
        "clicks": clicks,
        "horizontal": horizontal,
        "position": {"x": point[0], "y": point[1]} if point else None,
        "target": target_info(handle) if handle else None,
    }


VK_NAMES = {
    "CTRL": 0x11,
    "CONTROL": 0x11,
    "SHIFT": 0x10,
    "ALT": 0x12,
    "WIN": 0x5B,
    "LWIN": 0x5B,
    "RWIN": 0x5C,
    "ENTER": 0x0D,
    "RETURN": 0x0D,
    "TAB": 0x09,
    "ESC": 0x1B,
    "ESCAPE": 0x1B,
    "SPACE": 0x20,
    "BACKSPACE": 0x08,
    "DELETE": 0x2E,
    "INSERT": 0x2D,
    "UP": 0x26,
    "DOWN": 0x28,
    "LEFT": 0x25,
    "RIGHT": 0x27,
    "HOME": 0x24,
    "END": 0x23,
    "PAGEUP": 0x21,
    "PAGEDOWN": 0x22,
}
VK_NAMES.update({f"F{number}": 0x6F + number for number in range(1, 13)})
MODIFIERS = {"CTRL", "CONTROL", "SHIFT", "ALT", "WIN", "LWIN", "RWIN"}


def key_code(name: str) -> int:
    upper = name.strip().upper()
    if upper in VK_NAMES:
        return VK_NAMES[upper]
    if len(upper) == 1 and upper.isalnum():
        return ord(upper)
    raise ValueError(f"不支持的按键：{name}")


def unicode_events(text: str) -> list[INPUT]:
    events: list[INPUT] = []
    for char in text:
        if char == "\r":
            continue
        if char == "\n":
            events.append(key_event(0x0D))
            events.append(key_event(0x0D, 0, KEYEVENTF_KEYUP))
            continue
        if char == "\t":
            events.append(key_event(0x09))
            events.append(key_event(0x09, 0, KEYEVENTF_KEYUP))
            continue
        code = ord(char)
        if code > 0xFFFF:
            code -= 0x10000
            units = [0xD800 + (code >> 10), 0xDC00 + (code & 0x3FF)]
        else:
            units = [code]
        for unit in units:
            events.append(key_event(0, unit, KEYEVENTF_UNICODE))
            events.append(key_event(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP))
    return events


def send_in_chunks(events: list[INPUT]) -> None:
    for start in range(0, len(events), 200):
        chunk = events[start : start + 200]
        sent = send_inputs(chunk)
        if sent != len(chunk):
            raise RuntimeError(
                f"SendInput 只接受 {sent}/{len(chunk)} 个输入事件，可能被 UIPI 或安全软件阻止"
            )
        time.sleep(0.002)


def action_type(payload: dict) -> dict:
    text = str(payload.get("text", ""))
    if text == "":
        raise ValueError("text 不能为空")
    if len(text) > 5000:
        raise ValueError("text 最多 5000 字符")
    handle = handle_value(payload.get("handle"))
    if handle and not user32.IsWindow(handle):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    events = unicode_events(text)
    if not dry_run(payload):
        if handle and not focus_window(handle):
            raise RuntimeError("无法把目标窗口切到前台，文字没有输入")
        send_in_chunks(events)
    return {
        "ok": True,
        "action": "type",
        "dry_run": dry_run(payload),
        "characters": len(text),
        "events": len(events),
        "target": target_info(handle) if handle else None,
    }


def action_keys(payload: dict) -> dict:
    raw = payload.get("keys")
    if not isinstance(raw, list) or not raw:
        raise ValueError("keys 必须是非空数组")
    if len(raw) > 5:
        raise ValueError("一次最多 5 个按键")
    names = [str(item) for item in raw]
    codes = [key_code(name) for name in names]
    handle = handle_value(payload.get("handle"))
    if handle and not user32.IsWindow(handle):
        raise ValueError(f"不是有效窗口句柄：{payload.get('handle')}")
    modifiers = [index for index, name in enumerate(names) if name.strip().upper() in MODIFIERS]
    others = [index for index in range(len(names)) if index not in modifiers]
    if not others:
        raise ValueError("keys 里至少要有一个非修饰键")

    events: list[INPUT] = []
    for index in modifiers:
        events.append(key_event(codes[index]))
    for index in others:
        events.append(key_event(codes[index]))
        events.append(key_event(codes[index], 0, KEYEVENTF_KEYUP))
    for index in reversed(modifiers):
        events.append(key_event(codes[index], 0, KEYEVENTF_KEYUP))

    if not dry_run(payload):
        if handle and not focus_window(handle):
            raise RuntimeError("无法把目标窗口切到前台，按键没有发送")
        send_in_chunks(events)
    return {
        "ok": True,
        "action": "keys",
        "dry_run": dry_run(payload),
        "keys": names,
        "events": len(events),
        "target": target_info(handle) if handle else None,
    }


def main() -> None:
    try:
        payload = json.loads(sys.argv[1]) if len(sys.argv) > 1 else {}
        action = str(payload.get("action", ""))
        if action == "focus":
            result = action_focus(payload)
        elif action == "click":
            result = action_click(payload)
        elif action == "scroll":
            result = action_scroll(payload)
        elif action == "type":
            result = action_type(payload)
        elif action == "keys":
            result = action_keys(payload)
        else:
            raise ValueError(f"未知 action：{action}")
    except Exception as error:  # noqa: BLE001 - the JSON error is the API
        result = {"ok": False, "error": str(error)}
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    main()