import importlib.util
import json
import os
from pathlib import Path
import tempfile
import struct
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("release", Path(__file__).parents[1] / "release.py")
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)

class ReleaseGuards(unittest.TestCase):
    def test_zip_preserves_old_license_and_hidden_files(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / "Cargo.toml").write_text('[package]\nversion = "0.1.0-alpha.1"\n', encoding="utf-8")
            staging = root / "dist" / "staging"
            staging.mkdir(parents=True)
            license_file = staging / "LICENSE"
            license_file.write_bytes(b"upstream license")
            os.utime(license_file, (1, 1))
            (staging / ".notice").write_bytes(b"hidden notice")
            with patch.object(release, "ROOT", root):
                target = "x86_64-pc-windows-msvc"
                release.zip_package(target, staging)
                with zipfile.ZipFile(release.release_directory() / release.asset_name(target)) as archive:
                    self.assertEqual(archive.read("LICENSE"), b"upstream license")
                    self.assertEqual(archive.getinfo("LICENSE").date_time[0], 1980)
                    self.assertEqual(archive.read(".notice"), b"hidden notice")

    def test_windows_package_rejects_external_vc_runtime(self):
        data = bytearray(1024)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 128)
        data[128:132] = b"PE\0\0"
        struct.pack_into("<H", data, 132, 0x8664)
        struct.pack_into("<H", data, 134, 1)
        struct.pack_into("<H", data, 148, 240)
        struct.pack_into("<H", data, 152, 0x20B)
        struct.pack_into("<I", data, 272, 0x1000)
        struct.pack_into("<IIII", data, 400, 256, 0x1000, 256, 512)
        struct.pack_into("<I", data, 524, 0x1040)
        data[576:589] = b"KERNEL32.dll\0"
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / "vyber.exe"
            path.write_bytes(data)
            release.verify_binary_arch(path, "x86_64-pc-windows-msvc")
            self.assertEqual(release.windows_imports(path), ["KERNEL32.dll"])
            data[576:593] = b"VCRUNTIME140.dll\0"
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                release.verify_binary_arch(path, "x86_64-pc-windows-msvc")

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
