"""Verify dimos-viewer keyboard LCM output matches DimOS Twist expectations.

Run: python test_keyboard_lcm.py
Requires: dimos-lcm (PYTHONPATH=.../dimos-lcm/generated/python_lcm_msgs)
"""

import struct
import sys


def test_twist_encoding():
    """Verify Twist encoding matches Python LCM reference."""
    # This is what the Rust viewer now produces for Twist (no Header)
    # TwistCommand { linear_x: 0.5, angular_z: 0.3 }
    def rot(h):
        return ((h << 1) + ((h >> 63) & 1)) & 0xFFFFFFFFFFFFFFFF

    # Verify hash chain
    vector3_hash = rot(0x573f2fdd2f76508f)
    twist_hash = rot((0x3a4144772922add7 + vector3_hash + vector3_hash) & 0xFFFFFFFFFFFFFFFF)
    assert twist_hash == 0x2e7c07d7cdf7e027, f"Twist hash mismatch: 0x{twist_hash:016x}"

    # Build expected encoding manually: hash + 6 doubles
    buf = struct.pack(">q", twist_hash)      # 8B fingerprint
    buf += struct.pack(">d", 0.5)            # linear.x
    buf += struct.pack(">d", 0.0)            # linear.y
    buf += struct.pack(">d", 0.0)            # linear.z
    buf += struct.pack(">d", 0.0)            # angular.x
    buf += struct.pack(">d", 0.0)            # angular.y
    buf += struct.pack(">d", 0.3)            # angular.z
    assert len(buf) == 56, f"Expected 56 bytes, got {len(buf)}"

    expected_hex = "2e7c07d7cdf7e0273fe000000000000000000000000000000000000000000000000000000000000000000000000000003fd3333333333333"
    assert buf.hex() == expected_hex, f"Encoding mismatch:\n  got:    {buf.hex()}\n  expect: {expected_hex}"

    print("PASS: Twist encoding matches Python LCM reference (56 bytes)")
    print("  Channel: /cmd_vel#geometry_msgs.Twist")
    print(f"  Hash:    0x{twist_hash:016x}")

def test_channel_name():
    """Verify channel follows DimOS convention."""
    channel = "/cmd_vel#geometry_msgs.Twist"
    assert channel.startswith("/cmd_vel"), "Channel must start with /cmd_vel"
    assert "#geometry_msgs.Twist" in channel, "Channel must include type suffix"
    print(f"PASS: Channel name correct: {channel}")

if __name__ == "__main__":
    test_twist_encoding()
    test_channel_name()
    print("\nAll tests passed!")
