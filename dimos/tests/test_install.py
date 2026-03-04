"""Tests that verify the wheel installation is correct."""

from __future__ import annotations

import importlib

import pytest


def test_package_importable():
    """dimos_viewer is importable after install."""
    mod = importlib.import_module("dimos_viewer")
    assert hasattr(mod, "__version__")
    assert hasattr(mod, "main")
    assert hasattr(mod, "launch")
    assert hasattr(mod, "_find_viewer_binary")


def test_version_format():
    """Version string follows semver format."""
    from dimos_viewer import __version__

    parts = __version__.split(".")
    assert len(parts) == 3
    for part in parts:
        assert part.isdigit()


def test_rerun_version_documented():
    """Base Rerun version is documented."""
    from dimos_viewer import __rerun_version__

    assert __rerun_version__  # non-empty
    assert "." in __rerun_version__  # looks like a version string
