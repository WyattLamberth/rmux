#!/usr/bin/env python3
"""Package an already-built binary; never publish or change Cargo files."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--output", type=Path, default=Path("dist"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    if args.tag != f"v{version}":
        parser.error(f"tag {args.tag!r} must match Cargo.toml version v{version}")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        parser.error("unsupported release version")
    if not re.fullmatch(r"[a-z0-9_-]+", args.target):
        parser.error("invalid target")
    if not re.fullmatch(r"[0-9a-f]{40}", args.revision):
        parser.error("revision must be a full Git commit SHA")
    binary = args.binary.read_bytes()
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"rmux-{args.tag}-{args.target}"
    archive = args.output / f"{name}.tar.gz"
    metadata = json.dumps({
        "version": version, "target": args.target, "revision": args.revision,
        "cargo_lock_sha256": hashlib.sha256((root / "Cargo.lock").read_bytes()).hexdigest(),
        "flake_lock_sha256": hashlib.sha256((root / "flake.lock").read_bytes()).hexdigest(),
    }, indent=2, sort_keys=True).encode() + b"\n"
    # Stable order, ownership, permissions and timestamps make repackaging identical.
    with archive.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode="w", format=tarfile.USTAR_FORMAT) as tar:
                for filename, data, mode in (
                    ("rmux", binary, 0o755), ("BUILD.json", metadata, 0o644),
                    ("README.md", (root / "README.md").read_bytes(), 0o644),
                ):
                    info = tarfile.TarInfo(f"{name}/{filename}")
                    info.size, info.mode = len(data), mode
                    tar.addfile(info, io.BytesIO(data))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    checksum = args.output / f"{archive.name}.sha256"
    checksum.write_text(f"{digest}  {archive.name}\n")
    print(archive)
    print(checksum)


if __name__ == "__main__":
    main()
