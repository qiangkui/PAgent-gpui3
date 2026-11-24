"""Normalize a feedback note: collapse whitespace, keep the text.

Reads the step context on stdin and writes JSON on stdout, like every other
step program.
"""
import json
import sys


def main() -> None:
    context = json.loads(sys.stdin.read() or "{}")
    note = context.get("inputs", {}).get("note") or ""
    print(json.dumps({"note": " ".join(note.split())}, ensure_ascii=False))


if __name__ == "__main__":
    main()
