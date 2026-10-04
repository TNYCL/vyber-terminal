import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "ci_scope.py"
spec = importlib.util.spec_from_file_location("ci_scope", SCRIPT)
ci_scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_scope)


class ScopeGuards(unittest.TestCase):
    def test_only_known_documentation_paths_skip_full_ci(self):
        self.assertTrue(ci_scope.documentation_only(["README.md", "docs/RELEASING.md"]))
        for path in (
            "src/template.md",
            "assets/vyber.svg",
            "Cargo.lock",
            "LICENSE-MIT",
            "THIRD_PARTY_NOTICES.md",
            ".github/workflows/ci.yml",
            "scripts/ci_scope.py",
            "docs/runtime.json",
        ):
            with self.subTest(path=path):
                self.assertFalse(ci_scope.documentation_only(["README.md", path]))
        self.assertFalse(ci_scope.documentation_only([]))

    def test_manual_and_reusable_calls_force_full_checks(self):
        self.assertTrue(ci_scope.classify("workflow_dispatch", "", ""))
        self.assertTrue(ci_scope.classify("workflow_call", "", ""))
        self.assertTrue(ci_scope.classify("pull_request", "", "", force_full=True))

    def test_ready_accepts_only_intentional_documentation_skips(self):
        results = {name: {"result": "skipped"} for name in ci_scope.JOBS}
        results["changes"]["result"] = "success"
        ci_scope.require_checks("false", results)
        for name in ci_scope.JOBS:
            for status in ("failure", "cancelled"):
                with self.subTest(name=name, status=status):
                    broken = {key: dict(value) for key, value in results.items()}
                    broken[name]["result"] = status
                    with self.assertRaises(ValueError):
                        ci_scope.require_checks("false", broken)

    def test_ready_never_skips_required_full_checks(self):
        results = {name: {"result": "success"} for name in ci_scope.JOBS}
        ci_scope.require_checks("true", results)
        for name in ci_scope.JOBS:
            for status in ("failure", "cancelled", "skipped"):
                with self.subTest(name=name, status=status):
                    broken = {key: dict(value) for key, value in results.items()}
                    broken[name]["result"] = status
                    with self.assertRaises(ValueError):
                        ci_scope.require_checks("true", broken)

    def test_ready_rejects_unknown_scope_or_missing_jobs(self):
        results = {name: {"result": "success"} for name in ci_scope.JOBS}
        for full in ("", "unknown", "True"):
            with self.subTest(full=full), self.assertRaises(ValueError):
                ci_scope.require_checks(full, results)
        with self.assertRaises(ValueError):
            ci_scope.require_checks("true", {"changes": {"result": "success"}})


class GitScopeIntegration(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="vyber-ci-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.assertEqual(self.root.parent, Path(tempfile.gettempdir()).resolve())
        self.assertTrue(self.root.name.startswith("vyber-ci-test-"))
        self.git("-c", "init.defaultBranch=main", "init", "--quiet")
        self.write("README.md", "Initial readme\n")
        self.write("src/main.rs", "fn main() {}\n")
        self.write("docs/guide.md", "Initial guide\n")
        self.base = self.commit()

    def git(self, *args):
        return subprocess.check_output(
            [
                "git", "-c", f"core.hooksPath={self.root / 'empty-hooks'}",
                "-c", "commit.gpgsign=false", "-c", "user.name=CI test",
                "-c", "user.email=ci-test@example.invalid", *args,
            ],
            cwd=self.root,
            stderr=subprocess.PIPE,
        ).decode().strip()

    def write(self, path, content):
        file = self.root / path
        file.parent.mkdir(parents=True, exist_ok=True)
        file.write_text(content, encoding="utf-8")

    def commit(self):
        self.git("add", "--all")
        self.git("commit", "--quiet", "-m", "Fixture change")
        return self.git("rev-parse", "HEAD")

    def classify(self, event, base, head, **kwargs):
        return ci_scope.classify(event, base, head, cwd=self.root, **kwargs)

    def test_documentation_pr_and_push_take_fast_path(self):
        self.write("README.md", "Updated readme\n")
        head = self.commit()
        self.assertFalse(self.classify("pull_request", self.base, head))
        self.assertFalse(self.classify("push", self.base, head))
        self.assertTrue(self.classify("push", self.base, head, force_full=True))

    def test_push_checks_every_commit_not_only_the_documentation_tip(self):
        self.write("src/main.rs", 'fn main() { println!("changed"); }\n')
        self.commit()
        self.write("README.md", "Documentation tip\n")
        head = self.commit()
        self.assertTrue(self.classify("push", self.base, head))

    def test_pr_uses_merge_base_when_main_has_unrelated_changes(self):
        self.git("checkout", "--quiet", "-b", "docs")
        self.write("README.md", "Feature documentation\n")
        head = self.commit()
        self.git("checkout", "--quiet", "main")
        self.write("src/main.rs", "fn main() { /* unrelated main change */ }\n")
        advanced_base = self.commit()
        self.assertFalse(self.classify("pull_request", advanced_base, head))

    def test_moving_source_into_docs_still_requires_full_ci(self):
        (self.root / "src/main.rs").rename(self.root / "docs/example.md")
        head = self.commit()
        self.assertTrue(self.classify("pull_request", self.base, head))
        self.assertTrue(self.classify("push", self.base, head))

    def test_missing_or_zero_history_defaults_to_full_ci(self):
        self.assertTrue(self.classify("push", "0" * 40, self.base))
        self.assertTrue(self.classify("push", "a" * 40, self.base))
        self.assertTrue(self.classify("pull_request", "", self.base))
        self.assertTrue(self.classify("push", self.base, self.base))

    def test_cli_writes_scope_and_checks_required_job_results(self):
        self.write("README.md", "CLI documentation change\n")
        head = self.commit()
        output = self.root / "output.txt"
        env = dict(os.environ, EVENT_NAME="push", BASE_SHA=self.base, HEAD_SHA=head,
                   FORCE_FULL="false", GITHUB_OUTPUT=str(output))
        subprocess.run([sys.executable, str(SCRIPT), "classify"], cwd=self.root,
                       env=env, check=True, capture_output=True)
        self.assertEqual(output.read_text(encoding="utf-8"), "full=false\n")
        results = {name: {"result": "skipped"} for name in ci_scope.JOBS}
        results["changes"]["result"] = "success"
        env.update(FULL_CI="false", RESULTS=json.dumps(results))
        subprocess.run([sys.executable, str(SCRIPT), "check"], cwd=self.root,
                       env=env, check=True, capture_output=True)
        env["FULL_CI"] = "true"
        failed = subprocess.run([sys.executable, str(SCRIPT), "check"], cwd=self.root,
                                env=env, capture_output=True)
        self.assertNotEqual(failed.returncode, 0)


if __name__ == "__main__":
    unittest.main()
