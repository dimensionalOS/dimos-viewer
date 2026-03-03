"""Tests for the dimos_viewer Python wrapper."""

from __future__ import annotations

import os
import sys

import pytest


def test_version():
    """Package exposes version strings."""
    from dimos_viewer import __version__, __rerun_version__

    assert __version__ == "0.1.0"
    assert "0.30.0" in __rerun_version__


def test_find_viewer_binary_returns_path():
    """_find_viewer_binary returns a valid path when binary exists."""
    from dimos_viewer import _find_viewer_binary

    try:
        binary = _find_viewer_binary()
        assert os.path.isfile(binary)
        assert os.access(binary, os.X_OK)
    except FileNotFoundError:
        # If binary isn't installed, that's expected in some test environments
        pytest.skip("dimos-viewer binary not installed")


def test_find_viewer_binary_raises_when_missing(monkeypatch):
    """_find_viewer_binary raises FileNotFoundError when binary is absent."""
    import dimos_viewer

    # Mock out all search paths to return nothing
    monkeypatch.setattr("shutil.which", lambda _name: None)
    monkeypatch.setattr(
        "sysconfig.get_path",
        lambda _name: "/nonexistent/path",
    )
    # Force sys.prefix == sys.base_prefix (no venv)
    monkeypatch.setattr(sys, "prefix", sys.base_prefix)

    with pytest.raises(FileNotFoundError, match="dimos-viewer binary not found"):
        dimos_viewer._find_viewer_binary()


def test_main_entry_point_exists():
    """The main() function is importable and callable."""
    from dimos_viewer import main

    assert callable(main)


def test_launch_function_exists():
    """The launch() function is importable and callable."""
    from dimos_viewer import launch

    assert callable(launch)


def test_module_runnable():
    """python -m dimos_viewer is supported (has __main__.py)."""
    import importlib

    mod = importlib.import_module("dimos_viewer.__main__")
    assert mod is not None
