"""DimOS Interactive Viewer — custom Rerun viewer with click-to-navigate.

This package provides the DimOS interactive viewer, a customized Rerun viewer
that publishes click events over LCM for click-to-navigate robotics control.

Usage:
    # Command line (binary installed by pip):
    $ dimos-viewer

    # Python module:
    $ python -m dimos_viewer

    # Programmatic launch:
    >>> from dimos_viewer import launch
    >>> proc = launch(port=9877, background=True)
"""

from __future__ import annotations

__version__ = "0.1.0"
# Based on Rerun 0.30.0-alpha.1+dev (https://github.com/rerun-io/rerun)
__rerun_version__ = "0.30.0-alpha.1+dev"

import os
import shutil
import subprocess
import sys
import sysconfig


def _find_viewer_binary() -> str:
    """Locate the dimos-viewer binary installed alongside this package.

    The binary is installed by maturin into the Python scripts directory
    (e.g., <venv>/bin/dimos-viewer).

    Returns:
        Path to the viewer binary.

    Raises:
        FileNotFoundError: If the binary cannot be found.
    """
    # 1. Check the scripts directory (where pip/maturin installs binaries)
    scripts_dir = sysconfig.get_path("scripts")
    if scripts_dir:
        candidate = os.path.join(scripts_dir, "dimos-viewer")
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate

    # 2. Fall back to PATH lookup
    found = shutil.which("dimos-viewer")
    if found:
        return found

    # 3. Check common venv locations
    if sys.prefix != sys.base_prefix:
        venv_bin = os.path.join(sys.prefix, "bin", "dimos-viewer")
        if os.path.isfile(venv_bin) and os.access(venv_bin, os.X_OK):
            return venv_bin

    raise FileNotFoundError(
        "dimos-viewer binary not found. "
        "Reinstall with: pip install --force-reinstall dimos-viewer"
    )


def main() -> None:
    """Launch the DimOS interactive viewer (CLI entry point)."""
    binary = _find_viewer_binary()
    os.execv(binary, [binary] + sys.argv[1:])


def launch(
    *,
    port: int = 9877,
    background: bool = True,
) -> subprocess.Popen | None:
    """Launch the viewer programmatically.

    Args:
        port: gRPC port for Rerun SDK connections (default 9877).
        background: If True, launch in background and return the Popen handle.
                    If False, block until the viewer exits.

    Returns:
        subprocess.Popen handle if background=True, None otherwise.
    """
    binary = _find_viewer_binary()
    env = os.environ.copy()
    # Pass the gRPC port as environment variable
    env["RERUN_GRPC_PORT"] = str(port)

    if background:
        return subprocess.Popen(
            [binary],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    else:
        subprocess.run([binary], env=env, check=True)
        return None
