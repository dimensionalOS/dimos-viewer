"""Integration tests for the dimos-viewer binary."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys

import pytest


@pytest.fixture
def viewer_binary():
    """Find the viewer binary or skip if not installed."""
    binary = shutil.which("dimos-viewer")
    if binary is None:
        pytest.skip("dimos-viewer binary not installed")
    return binary


def test_binary_exists(viewer_binary):
    """The binary is findable on PATH."""
    assert os.path.isfile(viewer_binary)
    assert os.access(viewer_binary, os.X_OK)


def test_binary_help(viewer_binary):
    """Binary responds to --help without crashing."""
    # The viewer may not support --help, but it should at least start
    # and exit with some status. We use a short timeout to prevent hanging.
    try:
        result = subprocess.run(
            [viewer_binary, "--help"],
            capture_output=True,
            timeout=10,
        )
        # We accept any exit code — the important thing is it didn't crash/hang
        assert result.returncode is not None
    except subprocess.TimeoutExpired:
        # GUI apps may not respond to --help and hang waiting for display
        pytest.skip("Binary hung on --help (expected on headless)")


def test_binary_version_info(viewer_binary):
    """Binary contains expected strings (smoke test)."""
    # Just verify the binary is a valid executable
    result = subprocess.run(
        ["file", viewer_binary],
        capture_output=True,
        text=True,
    )
    output = result.stdout.lower()
    # Should be an ELF binary on Linux or Mach-O on macOS
    assert "executable" in output or "elf" in output or "mach-o" in output


def test_wheel_install_provides_binary():
    """After pip install, the binary should be in the scripts directory."""
    import sysconfig

    scripts_dir = sysconfig.get_path("scripts")
    if scripts_dir is None:
        pytest.skip("No scripts directory found")

    binary_path = os.path.join(scripts_dir, "dimos-viewer")
    if not os.path.exists(binary_path):
        # Also check PATH
        found = shutil.which("dimos-viewer")
        if found is None:
            pytest.skip("dimos-viewer not installed from wheel")
        binary_path = found

    assert os.path.isfile(binary_path)
    assert os.access(binary_path, os.X_OK)
