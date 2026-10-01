import importlib.util
import json
from pathlib import Path
import tempfile
import struct
import unittest

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

class ReleaseGuards(unittest.TestCase):
    def test_wrong_architecture_or_non_executable_is_rejected(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "vyber"
            header = bytearray(64)
            header[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", header, 18, 62)
            path.write_bytes(header)
            release.verify_binary_arch(path, "x86_64-unknown-linux-gnu")
            with self.assertRaises(ValueError):
                release.verify_binary_arch(path, "aarch64-unknown-linux-gnu")
            with self.assertRaises(ValueError):
                release.verify_binary_arch(path, "x86_64-pc-windows-msvc")
            path.write_bytes(b"truncated")
            with self.assertRaises(ValueError):
                release.verify_binary_arch(path, "x86_64-unknown-linux-gnu")

    def test_only_semver_tags_are_accepted(self):
        for tag in ("v0.1.0", "v1.2.3-alpha.1", "v1.0.0+build.7"):
            self.assertEqual(release.validate_tag(tag), tag[1:])
        for tag in ("main", "v01.2.3", "v1.2", "v1.2.3-alpha..1", "v1.2.3-01", "v1.2.3\nsha=bad", "v1.2.3;echo bad"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                release.validate_tag(tag)

    def fixture(self, directory):
        for target in release.TARGETS:
            path = directory / release.asset_name(target, "0.1.0-alpha.1")
            path.write_bytes(b"package data")
            metadata = {"file": path.name, "target": target, "version": "0.1.0-alpha.1", "commit": "a" * 40, "sha256": release.digest(path), "bytes": path.stat().st_size}
            (directory / (path.name + ".metadata.json")).write_text(json.dumps(metadata), encoding="utf-8")

    def test_all_five_matching_artifacts_are_required(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.fixture(directory)
            self.assertEqual(len(release.validate_packages(directory, "0.1.0-alpha.1", "a" * 40)), 5)
            next(path for path in directory.iterdir() if path.suffix == ".dmg").unlink()
            with self.assertRaises(ValueError):
                release.validate_packages(directory, "0.1.0-alpha.1", "a" * 40)

    def test_tampering_or_different_commit_stops_release(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.fixture(directory)
            with self.assertRaises(ValueError):
                release.validate_packages(directory, "0.1.0-alpha.1", "b" * 40)
            (directory / release.asset_name(next(iter(release.TARGETS)), "0.1.0-alpha.1")).write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                release.validate_packages(directory, "0.1.0-alpha.1", "a" * 40)

    def test_unexpected_package_stops_release(self):
        with tempfile.TemporaryDirectory() as name:
            directory = Path(name)
            self.fixture(directory)
            (directory / "unknown.zip").write_bytes(b"bad")
            with self.assertRaises(ValueError):
                release.validate_packages(directory, "0.1.0-alpha.1", "a" * 40)

if __name__ == "__main__":
    unittest.main()
