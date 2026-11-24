"""Count files under the workflow's repo input.

Reads the step context as JSON on stdin, writes a JSON result on stdout, and
keeps diagnostics on stderr so stdout stays parseable.
"""
import json
import os
import sys

SKIP = {".git", "node_modules", ".dsh-flow", "__pycache__", ".venv", "dist", "lib", "target"}


def main() -> None:
    context = json.loads(sys.stdin.read() or "{}")
    root = context.get("inputs", {}).get("repo") or "."
    limit = int(context.get("inputs", {}).get("maxFiles") or 20000)

    files = 0
    total = 0
    truncated = False
    for current, dirs, names in os.walk(root):
        dirs[:] = [name for name in dirs if name not in SKIP]
        for name in names:
            path = os.path.join(current, name)
            try:
                total += os.path.getsize(path)
            except OSError:
                continue
            files += 1
            if files >= limit:
                truncated = True
                break
        if truncated:
            break

    sys.stderr.write(f"scanned {files} files under {root}\n")
    print(json.dumps({
        # Echo the input as written: an absolute path would make the recorded
        # contract test depend on the machine and the caller's working directory.
        "root": root,
        "files": files,
        "kib": round(total / 1024, 1),
        "truncated": truncated,
    }, ensure_ascii=False))


if __name__ == "__main__":
    main()
