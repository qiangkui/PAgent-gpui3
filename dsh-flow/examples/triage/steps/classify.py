"""A stand-in for the model, so this example runs offline.

The `cli` AI backend writes `{prompt, system, model, schema, ...}` to stdin and
expects `{"text": "..."}` or `{"json": {...}}` on stdout. Replace this script by
removing the `backend` / `command` lines from `flow.yml` to use a real provider.
"""
import json
import sys

REASONS = {
    "bug": "页面功能异常",
    "question": "需要进一步确认",
    "other": "与产品无关",
}


def classify(prompt: str) -> dict:
    if "白屏" in prompt or "报错" in prompt or "崩溃" in prompt:
        category = "bug"
    elif "怎么" in prompt or "?" in prompt or "？" in prompt:
        category = "question"
    else:
        category = "other"
    return {"category": category, "reason": REASONS[category]}


def main() -> None:
    request = json.loads(sys.stdin.read() or "{}")
    print(json.dumps({"json": classify(request.get("prompt") or "")}, ensure_ascii=False))


if __name__ == "__main__":
    main()
