"""Install reviewed CI tool archives, checking a pinned SHA-256 before extraction."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import urllib.request

TOOLS = {
    "actionlint": (
        "https://github.com/rhysd/actionlint/releases/download/v1.7.12/actionlint_1.7.12_linux_amd64.tar.gz",
        "8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8",
    ),
    "cargo-deny": (
        "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz",
        "9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f",
    ),
}

def main():
    tool, destination = sys.argv[1:]
    url, expected = TOOLS[tool]
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    if hashlib.sha256(data).hexdigest() != expected:
        raise SystemExit(f"Checksum mismatch for {tool}")
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as archive:
        matches = [entry for entry in archive if entry.isfile() and Path(entry.name).name == tool]
        if len(matches) != 1:
            raise SystemExit(f"Expected one {tool} executable")
        output = Path(destination) / tool
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(archive.extractfile(matches[0]).read())
        output.chmod(0o755)
    print(output)

if __name__ == "__main__":
    main()
