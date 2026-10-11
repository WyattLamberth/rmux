#!/usr/bin/env python3
"""Require an exact v<Cargo package version> tag before packaging."""
import os
import sys
import tomllib
from pathlib import Path

version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
expected = f"v{version}"
actual = os.environ.get("GITHUB_REF_NAME", "")
if actual != expected:
    sys.exit(f"Release tag {actual!r} does not match Cargo.toml: {expected!r}")
print(f"Releasing {expected}")
