"""Select documentation-only CI without weakening the required ready check."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys


JOBS = {"changes", "quality", "audit", "native"}


def documentation_only(paths):
    return bool(paths) and all(
        path == "README.md" or (path.startswith("docs/") and path.endswith(".md"))
        for path in paths
    )


def git(*args, cwd=None):
    return subprocess.check_output(
        ["git", *args], cwd=cwd, stderr=subprocess.DEVNULL
    )


def classify(event, base, head, force_full=False, cwd=None):
    if force_full or event not in {"pull_request", "push"}:
        return True
    try:
        # Unknown history must take the full path, including first pushes.
        for sha in (base, head):
            if not re.fullmatch(r"[0-9a-f]{40}", sha or ""):
                return True
            git("cat-file", "-e", f"{sha}^{{commit}}", cwd=cwd)
        if event == "pull_request":
            base = git("merge-base", base, head, cwd=cwd).decode().strip()
        # Include both sides of renames: moving Rust into docs is still code.
        changed = git(
            "diff", "--name-only", "--no-renames", "-z", base, head, cwd=cwd
        )
        paths = changed.decode("utf-8", errors="surrogateescape").split("\0")
        return not documentation_only([path for path in paths if path])
    except (OSError, subprocess.CalledProcessError):
        return True


def require_checks(full_ci, results):
    if full_ci not in {"true", "false"} or set(results) != JOBS:
        raise ValueError("Missing or invalid CI scope/results")
    failed = {}
    for name, job in results.items():
        expected = "success" if name == "changes" or full_ci == "true" else "skipped"
        if job.get("result") != expected:
            failed[name] = job.get("result")
    if failed:
        raise ValueError(f"CI failed: {failed}")


def main():
    try:
        if len(sys.argv) != 2:
            raise ValueError("Usage: ci_scope.py classify|check")
        if sys.argv[1] == "classify":
            full = classify(
                os.environ.get("EVENT_NAME", ""),
                os.environ.get("BASE_SHA", ""),
                os.environ.get("HEAD_SHA", ""),
                os.environ.get("FORCE_FULL", "false").lower() != "false",
            )
            output = f"full={str(full).lower()}\n"
            with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as file:
                file.write(output)
            print("Full CI required." if full else "Documentation only; heavy checks skipped.")
        elif sys.argv[1] == "check":
            require_checks(os.environ["FULL_CI"], json.loads(os.environ["RESULTS"]))
            print("All required checks passed.")
        else:
            raise ValueError("Usage: ci_scope.py classify|check")
    except (KeyError, OSError, TypeError, ValueError) as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
