"""Shared packaging and release guards. Requires Python 3.11+ and Cargo/Git."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
REPO = "TNYCL/vyber-terminal"
TARGETS = {
    "x86_64-pc-windows-msvc": ("windows", "x86_64", "zip"),
    "aarch64-apple-darwin": ("macos", "aarch64", "dmg"),
    "x86_64-apple-darwin": ("macos", "x86_64", "dmg"),
    "x86_64-unknown-linux-gnu": ("linux", "x86_64", "tar.gz"),
    "aarch64-unknown-linux-gnu": ("linux", "aarch64", "tar.gz"),
}
SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?")

def run(*args, **kwargs):
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8", **kwargs).strip()

def version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]

def validate_tag(tag):
    if not tag.startswith("v") or not (match := SEMVER.fullmatch(tag[1:])):
        raise ValueError("Release tag must be v followed by a semantic version")
    for identifiers in (match[4], match[5]):
        if identifiers and any(not part for part in identifiers.split(".")):
            raise ValueError("Empty version identifier")
    if match[4] and any(part.isdigit() and len(part) > 1 and part[0] == "0" for part in match[4].split(".")):
        raise ValueError("Numeric prerelease identifiers cannot have leading zeros")
    return tag[1:]

def asset_name(target, release_version=None):
    platform, arch, extension = TARGETS[target]
    return f"vyber-{release_version or version()}-{platform}-{arch}.{extension}"

def release_directory():
    directory = ROOT / "dist" / "release"
    directory.mkdir(parents=True, exist_ok=True)
    return directory

def binary_path(target):
    directory = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target"))
    return directory / target / "release" / ("vyber.exe" if TARGETS[target][0] == "windows" else "vyber")

def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()

def windows_imports(path):
    data = Path(path).read_bytes()
    try:
        pe = struct.unpack_from("<I", data, 60)[0]
        sections = struct.unpack_from("<H", data, pe + 6)[0]
        optional = pe + 24
        if data[:2] != b"MZ" or data[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", data, optional)[0] != 0x20B:
            raise ValueError("Expected a PE64 executable")
        section_table = optional + struct.unpack_from("<H", data, pe + 20)[0]
        ranges = [struct.unpack_from("<IIII", data, section_table + index * 40 + 8) for index in range(sections)]
        def offset(rva):
            for virtual_size, virtual_address, raw_size, raw in ranges:
                if virtual_address <= rva < virtual_address + min(virtual_size, raw_size):
                    return raw + rva - virtual_address
            raise ValueError("Invalid PE import address")
        import_rva = struct.unpack_from("<I", data, optional + 120)[0]
        if not import_rva:
            return []
        position = offset(import_rva)
        libraries = []
        while any(struct.unpack_from("<IIIII", data, position)):
            name = offset(struct.unpack_from("<I", data, position + 12)[0])
            libraries.append(data[name:data.index(b"\0", name)].decode("ascii"))
            position += 20
        return sorted(libraries, key=str.lower)
    except (struct.error, UnicodeDecodeError, IndexError) as error:
        raise ValueError("Invalid PE import table") from error

def verify_binary_arch(path, target):
    platform, architecture, _ = TARGETS[target]
    with Path(path).open("rb") as stream:
        header = stream.read(64)
        if len(header) < 64:
            raise ValueError("Truncated executable")
        if platform == "windows":
            if header[:2] != b"MZ":
                raise ValueError("Expected a Windows PE executable")
            stream.seek(struct.unpack_from("<I", header, 60)[0])
            pe = stream.read(6)
            if len(pe) != 6 or pe[:4] != b"PE\0\0" or struct.unpack_from("<H", pe, 4)[0] != 0x8664:
                raise ValueError("Expected a Windows x64 executable")
            runtime = [name for name in windows_imports(path) if re.match(r"(?:vcruntime|msvcp|msvcr)[0-9]", name, re.I)]
            if runtime:
                raise ValueError(f"Windows ZIP requires a static CRT; external runtime found: {runtime}")
        elif platform == "linux":
            expected = 62 if architecture == "x86_64" else 183
            if header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != expected:
                raise ValueError(f"Expected a Linux {architecture} ELF executable")
        else:
            expected = 0x01000007 if architecture == "x86_64" else 0x0100000C
            if header[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", header, 4)[0] != expected:
                raise ValueError(f"Expected a macOS {architecture} Mach-O executable")

def stage(target, directory):
    directory = Path(directory).resolve()
    if not directory.is_relative_to((ROOT / "dist").resolve()):
        raise ValueError("Package staging must stay inside this checkout's dist directory")
    directory.mkdir(parents=True, exist_ok=True)
    for name in ("LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md"):
        shutil.copy2(ROOT / name, directory / name)
    metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", target))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root = metadata["resolve"]["root"]
    selected, pending = set(), [root]
    while pending:
        package_id = pending.pop()
        if package_id in selected:
            continue
        selected.add(package_id)
        for dependency in nodes[package_id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    packages = sorted((p for p in metadata["packages"] if p["id"] in selected), key=lambda p: (p["name"], p["version"]))
    licenses = directory / "third-party-licenses"
    inventory = []
    spdx_packages = []
    identifiers = {p["id"]: f"SPDXRef-Package-{index}" for index, p in enumerate(packages)}
    for package in packages:
        source = Path(package["manifest_path"]).parent
        license_files = [path for path in source.iterdir() if path.is_file() and path.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE"))]
        if package.get("license_file"):
            explicit = source / package["license_file"]
            if explicit.is_file() and explicit not in license_files:
                license_files.append(explicit)
        copied = []
        for file in license_files:
            relative = Path(f"{package['name']}-{package['version']}") / file.name
            output = licenses / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, output)
            copied.append(str(relative).replace("\\", "/"))
        inventory.append({"name": package["name"], "version": package["version"], "license": package["license"], "repository": package["repository"], "license_files": copied})
        spdx_packages.append({
            "SPDXID": identifiers[package["id"]], "name": package["name"], "versionInfo": package["version"],
            "downloadLocation": f"https://crates.io/api/v1/crates/{package['name']}/{package['version']}/download" if package["source"] else "NOASSERTION",
            "filesAnalyzed": False, "licenseConcluded": "NOASSERTION", "licenseDeclared": package["license"] or "NOASSERTION",
            "copyrightText": "NOASSERTION",
        })
    (directory / "dependency-licenses.json").write_text(json.dumps(inventory, indent=2) + "\n", encoding="utf-8")
    commit = run("git", "rev-parse", "HEAD")
    relationships = [{"spdxElementId": "SPDXRef-DOCUMENT", "relationshipType": "DESCRIBES", "relatedSpdxElement": identifiers[root]}]
    for package_id in selected:
        for dependency in nodes[package_id]["deps"]:
            if dependency["pkg"] in selected and any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                relationships.append({"spdxElementId": identifiers[package_id], "relationshipType": "DEPENDS_ON", "relatedSpdxElement": identifiers[dependency["pkg"]]})
    sbom = {
        "spdxVersion": "SPDX-2.3", "dataLicense": "CC0-1.0", "SPDXID": "SPDXRef-DOCUMENT",
        "name": f"vyber-{version()}-{target}",
        "documentNamespace": f"https://github.com/{REPO}/sbom/{uuid.uuid5(uuid.NAMESPACE_URL, commit + target)}",
        "creationInfo": {"creators": ["Tool: vyber-release.py"], "created": datetime.fromisoformat(run("git", "show", "-s", "--format=%cI", "HEAD")).astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")},
        "comment": "Cargo resolved normal/build dependency graph for this target. Includes shipped upstream license files; licenseConcluded is not a legal determination.",
        "packages": spdx_packages, "relationships": sorted(relationships, key=lambda r: (r["spdxElementId"], r["relatedSpdxElement"])),
    }
    (directory / "sbom.spdx.json").write_text(json.dumps(sbom, indent=2) + "\n", encoding="utf-8")
    (directory / "INSTALL.md").write_text(installation(TARGETS[target][0]), encoding="utf-8")
    if TARGETS[target][0] == "windows":
        (directory / "runtime-libraries.txt").write_text("\n".join(windows_imports(binary_path(target))) + "\n", encoding="utf-8")

def installation(platform):
    common = "# Vyber\n\nGit is required for source-control features. ripgrep (rg) is optional for content search.\n\n"
    if platform == "windows":
        return common + "Extract this ZIP and run Vyber.exe on Windows 11 x64. The Visual C++ runtime is statically linked; no separate VC++ Redistributable installer is required. Git Bash is preferred when installed; PowerShell is the fallback. This preview is not Authenticode-signed. SmartScreen or local policy may warn or block it.\n"
    if platform == "macos":
        return common + "Drag Vyber.app to Applications. This preview is ad-hoc-signed and has not been Apple-notarized. Gatekeeper may block first launch. See Apple's supported Open Anyway flow in Privacy & Security after checking the release source. macOS 12 is a deployment target, not a verified minimum.\n"
    return common + "Run ./bin/vyber, or install bin/vyber to ~/.local/bin, share/applications/dev.vyber.terminal.desktop to ~/.local/share/applications, and the icon under ~/.local/share/icons/hicolor/256x256/apps. Requires a desktop session, a compatible GPU/driver, XDG utilities, fontconfig, DejaVu Sans Mono and Noto fonts. Desktop notifications require a session D-Bus notification service. Start from a terminal to diagnose missing runtime libraries.\n"

def zip_package(target, directory):
    directory = Path(directory).resolve()
    if TARGETS[target][0] != "windows" or not directory.is_relative_to((ROOT / "dist").resolve()):
        raise ValueError("Windows ZIP staging must stay inside this checkout's dist directory")
    with zipfile.ZipFile(release_directory() / asset_name(target), "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9, strict_timestamps=False) as archive:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                archive.write(path, path.relative_to(directory).as_posix())

def record(target):
    verify_binary_arch(binary_path(target), target)
    path = release_directory() / asset_name(target)
    if not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"Missing package: {path.name}")
    data = {"file": path.name, "target": target, "version": version(), "commit": run("git", "rev-parse", "HEAD"), "sha256": digest(path), "bytes": path.stat().st_size, "rust": run("rustc", "--version"), "signing": "ad-hoc" if TARGETS[target][0] == "macos" else "unsigned"}
    (path.parent / (path.name + ".metadata.json")).write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")

def verify_package(target):
    binary = binary_path(target)
    output = run(str(binary), "--version")
    if output != f"vyber {version()}":
        raise ValueError(f"Binary version mismatch: {output}")
    path = release_directory() / asset_name(target)
    required = {"INSTALL.md", "LICENSE-MIT", "LICENSE-APACHE", "THIRD_PARTY_NOTICES.md", "dependency-licenses.json", "sbom.spdx.json", "runtime-libraries.txt"}
    platform = TARGETS[target][0]
    if platform == "windows":
        with zipfile.ZipFile(path) as archive:
            entries = {Path(name).name for name in archive.namelist()}
            if not (required | {"Vyber.exe"}) <= entries:
                raise ValueError("Incomplete Windows ZIP")
            if any(name.lower().endswith(".lnk") for name in archive.namelist()):
                raise ValueError("Local shortcuts must not be shipped")
            if hashlib.sha256(archive.read("Vyber.exe")).hexdigest() != digest(binary):
                raise ValueError("ZIP contains a different executable")
    elif platform == "linux":
        with tarfile.open(path, "r:gz") as archive:
            entries = {Path(name).name for name in archive.getnames()}
            if not (required | {"vyber", "dev.vyber.terminal.desktop"}) <= entries:
                raise ValueError("Incomplete Linux archive")
            executable = next(entry for entry in archive if entry.name.endswith("/bin/vyber"))
            if not executable.mode & 0o111:
                raise ValueError("Linux binary lost executable permissions")
            if hashlib.sha256(archive.extractfile(executable).read()).hexdigest() != digest(binary):
                raise ValueError("Linux archive contains a different executable")
    else:
        run("hdiutil", "verify", str(path))
        with tempfile.TemporaryDirectory(prefix="vyber-dmg-check-") as mount:
            run("hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", mount, str(path))
            try:
                app = Path(mount) / "Vyber.app"
                resources = app / "Contents" / "Resources"
                if not all((resources / name).is_file() for name in required):
                    raise ValueError("Incomplete macOS app resources")
                packaged_binary = app / "Contents" / "MacOS" / "vyber"
                verify_binary_arch(packaged_binary, target)
                if run(str(packaged_binary), "--version") != f"vyber {version()}":
                    raise ValueError("DMG contains a different executable version")
                run("codesign", "--verify", "--deep", "--strict", str(app))
            finally:
                run("hdiutil", "detach", mount)
    data = json.loads((path.parent / (path.name + ".metadata.json")).read_text(encoding="utf-8"))
    if data["sha256"] != digest(path) or data["target"] != target or data["version"] != version():
        raise ValueError("Package metadata mismatch")
    print(f"Verified {path.name}")

def preflight(tag):
    release_version = validate_tag(tag)
    run("git", "fetch", "--no-tags", "origin", "main")
    main_sha = run("git", "rev-parse", "FETCH_HEAD")
    run("git", "fetch", "--no-tags", "origin", f"refs/tags/{tag}:refs/tags/{tag}")
    sha = run("git", "rev-parse", f"refs/tags/{tag}^{{commit}}")
    run("git", "merge-base", "--is-ancestor", sha, main_sha)
    cargo = tomllib.loads(run("git", "show", f"{sha}:Cargo.toml"))
    if cargo["package"]["version"] != release_version:
        raise ValueError("Tag and Cargo version differ")
    release = get_release(tag)
    if release and not release["draft"]:
        raise ValueError("This release is already published; create a new version")
    values = {"sha": sha, "tag": tag, "version": release_version}
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            for key, value in values.items():
                stream.write(f"{key}={value}\n")
    print(json.dumps(values))

def get_release(tag):
    result = subprocess.run(["gh", "api", f"repos/{REPO}/releases/tags/{tag}"], cwd=ROOT, text=True, encoding="utf-8", capture_output=True)
    if result.returncode == 0:
        return json.loads(result.stdout)
    if "HTTP 404" in result.stderr:
        return None
    raise RuntimeError(result.stderr)

def validate_packages(directory, release_version, sha):
    expected = {asset_name(target, release_version) for target in TARGETS}
    actual = {path.name for path in Path(directory).iterdir() if path.name.endswith((".zip", ".dmg", ".tar.gz"))}
    if actual != expected:
        raise ValueError(f"Expected all five packages; missing={expected - actual}, unexpected={actual - expected}")
    packages = []
    for target in TARGETS:
        path = Path(directory) / asset_name(target, release_version)
        metadata = json.loads((path.parent / (path.name + ".metadata.json")).read_text(encoding="utf-8"))
        if metadata["file"] != path.name or metadata["target"] != target or metadata["version"] != release_version or metadata["commit"] != sha or metadata["sha256"] != digest(path) or metadata["bytes"] != path.stat().st_size:
            raise ValueError(f"Release metadata mismatch: {path.name}")
        packages.append(metadata)
    return packages

def assemble(tag, sha):
    release_version = validate_tag(tag)
    if version() != release_version or run("git", "rev-parse", "HEAD") != sha:
        raise ValueError("Release checkout does not match tag/commit")
    directory = release_directory()
    packages = validate_packages(directory, release_version, sha)
    (directory / "release-manifest.json").write_text(json.dumps({"repository": REPO, "tag": tag, "commit": sha, "packages": packages}, indent=2) + "\n", encoding="utf-8")
    for package in packages:
        (directory / (package["file"] + ".metadata.json")).unlink()
    files = sorted([directory / p["file"] for p in packages] + [directory / "release-manifest.json"])
    (directory / "SHA256SUMS.txt").write_text("".join(f"{digest(path)}  {path.name}\n" for path in files), encoding="utf-8")
    notes = f"""Vyber {release_version}

Native packages for Windows x64, macOS Apple Silicon and Intel, and Linux x64/ARM64.

### Installation and verification
Windows: extract the ZIP and run Vyber.exe. macOS: open the DMG and drag Vyber.app to Applications. Linux: extract the tar.gz and run bin/vyber; see INSTALL.md for desktop integration.

Git is needed for source-control features; ripgrep is optional for content search.
Verify SHA256SUMS.txt and build provenance with `gh attestation verify <package> --repo {REPO}`.

### Preview limitations
Windows packages are unsigned. macOS packages are ad-hoc-signed and not Apple-notarized. OS security prompts or policies may prevent first launch. CI build/test success does not establish all desktop/GPU combinations. macOS 12 is a deployment target, not a verified minimum. Linux ARM64 and desktop integration require manual acceptance before claiming full support.

### Desktop acceptance — complete before publishing
- [ ] Clean Windows 11 x64 launch, Git Bash/PowerShell, keyboard and clipboard
- [ ] Apple Silicon Mac: Finder/Dock launch, shell PATH and window lifecycle
- [ ] Intel Mac: Finder/Dock launch, shell PATH and window lifecycle
- [ ] Linux x64: X11 and Wayland launch, fonts, CWD, file opening and notifications
- [ ] Linux ARM64 desktop/GPU acceptance, or explicitly retain experimental status

Source commit: `{sha}`. All five artifacts must be present. This draft is never published automatically.
"""
    (ROOT / "dist" / "release-notes.md").write_text(notes, encoding="utf-8")
    print(f"Assembled {len(packages)} packages for {tag}")

def draft(tag, sha):
    validate_tag(tag)
    directory = release_directory()
    manifest = json.loads((directory / "release-manifest.json").read_text(encoding="utf-8"))
    if manifest["tag"] != tag or manifest["commit"] != sha:
        raise ValueError("Draft metadata differs from requested release")
    release = get_release(tag)
    if release and (not release["draft"] or release["target_commitish"] != sha):
        raise ValueError("Refusing to overwrite a published or different-source release")
    generated = json.loads(run("gh", "api", "--method", "POST", f"repos/{REPO}/releases/generate-notes", "-f", f"tag_name={tag}", "-f", f"target_commitish={sha}"))
    notes_path = ROOT / "dist" / "release-notes.md"
    if generated.get("body"):
        with notes_path.open("a", encoding="utf-8") as notes:
            notes.write("\n## Changes\n\n" + generated["body"] + "\n")
    if not release:
        args = ["gh", "release", "create", tag, "--repo", REPO, "--verify-tag", "--target", sha, "--draft", "--title", f"Vyber {tag[1:]}", "--notes-file", str(ROOT / "dist" / "release-notes.md")]
        if "-" in tag.split("+", 1)[0]:
            args.append("--prerelease")
        run(*args)
    else:
        run("gh", "release", "edit", tag, "--repo", REPO, "--notes-file", str(ROOT / "dist" / "release-notes.md"))
    if bundle := os.environ.get("ATTESTATION_BUNDLE"):
        shutil.copyfile(bundle, directory / "build-provenance.sigstore.json")
    run("gh", "release", "upload", tag, "--repo", REPO, "--clobber", *[str(path) for path in sorted(directory.iterdir()) if path.is_file()])
    print(run("gh", "release", "view", tag, "--repo", REPO, "--json", "url", "--jq", ".url"))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("version")
    for command in ("asset-name", "binary-path", "record", "verify-package", "stage", "zip"):
        command_parser = sub.add_parser(command)
        command_parser.add_argument("--target", choices=TARGETS, required=True)
        if command in ("stage", "zip"):
            command_parser.add_argument("--directory", required=True)
    for command in ("preflight", "assemble", "draft"):
        command_parser = sub.add_parser(command)
        command_parser.add_argument("--tag", required=True)
        if command != "preflight":
            command_parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    if args.command == "version":
        print(version())
    elif args.command == "asset-name":
        print(asset_name(args.target))
    elif args.command == "binary-path":
        print(binary_path(args.target))
    elif args.command == "stage":
        stage(args.target, args.directory)
    elif args.command == "zip":
        zip_package(args.target, args.directory)
    elif args.command == "record":
        record(args.target)
    elif args.command == "verify-package":
        verify_package(args.target)
    elif args.command == "preflight":
        preflight(args.tag)
    elif args.command == "assemble":
        assemble(args.tag, args.sha)
    else:
        draft(args.tag, args.sha)

if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
