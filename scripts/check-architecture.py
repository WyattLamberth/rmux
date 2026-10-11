#!/usr/bin/env python3
"""Keep the domain crate isolated from framework and adapter dependencies."""
import json
import subprocess
import sys
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--no-deps", "--locked", "--format-version", "1"],
    text=True, cwd=root
))
core = next((package for package in metadata["packages"] if package["name"] == "rmux-core"), None)
if core is None:
    sys.exit("Architecture violation: the rmux-core domain crate is missing")
if core["id"] not in metadata["workspace_members"]:
    sys.exit("Architecture violation: rmux-core must be a workspace member")
if core["dependencies"]:
    sys.exit("Architecture violation: rmux-core must remain dependency-free")
workspace = tomllib.loads((root / "Cargo.toml").read_text()).get("workspace", {})
for package in metadata["packages"]:
    if package["id"] not in metadata["workspace_members"]:
        continue
    manifest = tomllib.loads(Path(package["manifest_path"]).read_text())
    lints = manifest.get("lints", {})
    if lints.get("workspace") is True:
        lints = workspace.get("lints", {})
    if lints.get("rust", {}).get("unsafe_code") != "forbid":
        sys.exit(f"Architecture violation: {package['name']} must forbid unsafe_code")
runtime = next((p for p in metadata["packages"] if p["name"] == "rmux"), None)
if runtime is None or not any(t["name"] == "rmux" and "bin" in t["kind"] for t in runtime["targets"]):
    sys.exit("Architecture violation: root runtime binary rmux is missing")
print("Domain dependency boundary verified; Rust's unsafe_code lint checks safety")
