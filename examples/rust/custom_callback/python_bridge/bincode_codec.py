"""Minimal bincode codec for ViewerEvent and AppCommand messages.

Implements the subset of bincode serialization needed to communicate with
the Rust viewer. Supports Rust enums, Option<T>, String, Vec<T>, arrays, and
primitives (u32, u64, f32, bool).

Bincode format (default config):
- Integers: Little-endian
- Strings: Length (u64 LE) + UTF-8 bytes
- Option<T>: 1 byte (0=None, 1=Some) + T if Some
- Vec<T>: Length (u64 LE) + elements
- Enums: Variant index (u32 LE) + variant fields
"""

import struct
from typing import Optional, Any
from dataclasses import dataclass


class BincodeError(Exception):
    """Bincode encoding/decoding error."""
    pass


class BincodeEncoder:
    """Encode Python objects to bincode format."""
    
    def __init__(self):
        self.buffer = bytearray()
    
    def write_u32(self, value: int):
        """Write a u32 (4 bytes, little-endian)."""
        self.buffer.extend(struct.pack('<I', value))
    
    def write_u64(self, value: int):
        """Write a u64 (8 bytes, little-endian)."""
        self.buffer.extend(struct.pack('<Q', value))
    
    def write_f32(self, value: float):
        """Write an f32 (4 bytes, little-endian)."""
        self.buffer.extend(struct.pack('<f', value))
    
    def write_bool(self, value: bool):
        """Write a bool (1 byte, 0 or 1)."""
        self.buffer.append(1 if value else 0)
    
    def write_string(self, value: str):
        """Write a String (length u64 + UTF-8 bytes)."""
        utf8 = value.encode('utf-8')
        self.write_u64(len(utf8))
        self.buffer.extend(utf8)
    
    def write_option_string(self, value: Optional[str]):
        """Write an Option<String> (1 byte tag + String if Some)."""
        if value is None:
            self.buffer.append(0)
        else:
            self.buffer.append(1)
            self.write_string(value)
    
    def write_vec_f32_array3(self, values: list[tuple[float, float, float]]):
        """Write a Vec<[f32; 3]> (length u64 + array elements)."""
        self.write_u64(len(values))
        for x, y, z in values:
            self.write_f32(x)
            self.write_f32(y)
            self.write_f32(z)
    
    def write_f32_array3(self, x: float, y: float, z: float):
        """Write a [f32; 3] array."""
        self.write_f32(x)
        self.write_f32(y)
        self.write_f32(z)
    
    def bytes(self) -> bytes:
        """Get the encoded bytes."""
        return bytes(self.buffer)


class BincodeDecoder:
    """Decode bincode format to Python objects."""
    
    def __init__(self, data: bytes):
        self.data = data
        self.offset = 0
    
    def read_u32(self) -> int:
        """Read a u32 (4 bytes, little-endian)."""
        if self.offset + 4 > len(self.data):
            raise BincodeError(f"Not enough data for u32 at offset {self.offset}")
        value = struct.unpack_from('<I', self.data, self.offset)[0]
        self.offset += 4
        return value
    
    def read_u64(self) -> int:
        """Read a u64 (8 bytes, little-endian)."""
        if self.offset + 8 > len(self.data):
            raise BincodeError(f"Not enough data for u64 at offset {self.offset}")
        value = struct.unpack_from('<Q', self.data, self.offset)[0]
        self.offset += 8
        return value
    
    def read_f32(self) -> float:
        """Read an f32 (4 bytes, little-endian)."""
        if self.offset + 4 > len(self.data):
            raise BincodeError(f"Not enough data for f32 at offset {self.offset}")
        value = struct.unpack_from('<f', self.data, self.offset)[0]
        self.offset += 4
        return value
    
    def read_bool(self) -> bool:
        """Read a bool (1 byte, 0 or 1)."""
        if self.offset + 1 > len(self.data):
            raise BincodeError(f"Not enough data for bool at offset {self.offset}")
        value = self.data[self.offset]
        self.offset += 1
        if value > 1:
            raise BincodeError(f"Invalid bool value: {value}")
        return value == 1
    
    def read_string(self) -> str:
        """Read a String (length u64 + UTF-8 bytes)."""
        length = self.read_u64()
        if self.offset + length > len(self.data):
            raise BincodeError(f"Not enough data for string of length {length}")
        utf8 = self.data[self.offset:self.offset + length]
        self.offset += length
        try:
            return utf8.decode('utf-8')
        except UnicodeDecodeError as e:
            raise BincodeError(f"Invalid UTF-8 in string: {e}")
    
    def read_option_string(self) -> Optional[str]:
        """Read an Option<String> (1 byte tag + String if Some)."""
        tag = self.data[self.offset]
        self.offset += 1
        
        if tag == 0:
            return None
        elif tag == 1:
            return self.read_string()
        else:
            raise BincodeError(f"Invalid Option tag: {tag}")
    
    def read_vec_f32_array3(self) -> list[tuple[float, float, float]]:
        """Read a Vec<[f32; 3]> (length u64 + array elements)."""
        length = self.read_u64()
        waypoints = []
        for _ in range(length):
            x = self.read_f32()
            y = self.read_f32()
            z = self.read_f32()
            waypoints.append((x, y, z))
        return waypoints
    
    def read_f32_array3(self) -> tuple[float, float, float]:
        """Read a [f32; 3] array."""
        x = self.read_f32()
        y = self.read_f32()
        z = self.read_f32()
        return (x, y, z)


@dataclass
class ViewerEvent:
    """Base class for viewer events."""
    pass


@dataclass
class ClickEvent(ViewerEvent):
    """User clicked in a spatial view."""
    position: tuple[float, float, float]
    entity_path: Optional[str]
    view_id: str
    timestamp_ms: int
    is_2d: bool
    
    @property
    def x(self) -> float:
        return self.position[0]
    
    @property
    def y(self) -> float:
        return self.position[1]
    
    @property
    def z(self) -> float:
        return self.position[2]
    
    @property
    def timestamp(self) -> float:
        """Unix timestamp in seconds."""
        return self.timestamp_ms / 1000.0


@dataclass
class WaypointCompleteEvent(ViewerEvent):
    """Waypoint sequence completed."""
    waypoints: list[tuple[float, float, float]]


@dataclass
class ModeChangedEvent(ViewerEvent):
    """Interaction mode changed."""
    mode: str


@dataclass
class DisconnectEvent(ViewerEvent):
    """Viewer is disconnecting."""
    pass


def decode_viewer_event(data: bytes) -> ViewerEvent:
    """Decode a ViewerEvent from bincode bytes.
    
    Rust enum ViewerEvent variants:
    - 0: Click
    - 1: WaypointComplete
    - 2: ModeChanged
    - 3: Disconnect
    """
    decoder = BincodeDecoder(data)
    variant_index = decoder.read_u32()
    
    if variant_index == 0:  # Click
        position = decoder.read_f32_array3()
        entity_path = decoder.read_option_string()
        view_id = decoder.read_string()
        timestamp_ms = decoder.read_u64()
        is_2d = decoder.read_bool()
        
        return ClickEvent(
            position=position,
            entity_path=entity_path,
            view_id=view_id,
            timestamp_ms=timestamp_ms,
            is_2d=is_2d,
        )
    
    elif variant_index == 1:  # WaypointComplete
        waypoints = decoder.read_vec_f32_array3()
        return WaypointCompleteEvent(waypoints=waypoints)
    
    elif variant_index == 2:  # ModeChanged
        mode = decoder.read_string()
        return ModeChangedEvent(mode=mode)
    
    elif variant_index == 3:  # Disconnect
        return DisconnectEvent()
    
    else:
        raise BincodeError(f"Unknown ViewerEvent variant: {variant_index}")


@dataclass
class AppCommand:
    """Base class for app commands."""
    pass


@dataclass
class SetModeCommand(AppCommand):
    """Change the interaction mode."""
    mode: str


@dataclass
class ClearWaypointsCommand(AppCommand):
    """Clear all waypoint markers."""
    pass


@dataclass
class SetCursorCommand(AppCommand):
    """Set the cursor style."""
    cursor: str


def encode_app_command(command: AppCommand) -> bytes:
    """Encode an AppCommand to bincode bytes.
    
    Rust enum AppCommand variants:
    - 0: SetMode
    - 1: ClearWaypoints
    - 2: SetCursor
    """
    encoder = BincodeEncoder()
    
    if isinstance(command, SetModeCommand):
        encoder.write_u32(0)  # Variant index
        encoder.write_string(command.mode)
    
    elif isinstance(command, ClearWaypointsCommand):
        encoder.write_u32(1)  # Variant index
        # No fields
    
    elif isinstance(command, SetCursorCommand):
        encoder.write_u32(2)  # Variant index
        encoder.write_string(command.cursor)
    
    else:
        raise BincodeError(f"Unknown AppCommand type: {type(command)}")
    
    return encoder.bytes()


if __name__ == "__main__":
    # Quick test
    import sys
    
    print("Testing bincode codec...")
    
    # Test Click event roundtrip (encode in Rust, decode in Python)
    # This would normally come from the Rust viewer
    print("\n1. Testing Click event decode:")
    encoder = BincodeEncoder()
    encoder.write_u32(0)  # Click variant
    encoder.write_f32_array3(1.5, 2.5, 3.5)  # position
    encoder.buffer.append(1)  # Option::Some
    encoder.write_string("world/robot")  # entity_path
    encoder.write_string("view_123")  # view_id
    encoder.write_u64(1234567890)  # timestamp_ms
    encoder.write_bool(False)  # is_2d
    
    event_bytes = encoder.bytes()
    print(f"Encoded {len(event_bytes)} bytes: {event_bytes.hex()}")
    
    event = decode_viewer_event(event_bytes)
    print(f"Decoded: {event}")
    assert isinstance(event, ClickEvent)
    assert event.position == (1.5, 2.5, 3.5)
    assert event.entity_path == "world/robot"
    assert event.view_id == "view_123"
    assert event.timestamp_ms == 1234567890
    assert event.is_2d == False
    print("✓ Click event OK")
    
    # Test SetMode command encode
    print("\n2. Testing SetMode command encode:")
    cmd = SetModeCommand(mode="waypoint")
    cmd_bytes = encode_app_command(cmd)
    print(f"Encoded {len(cmd_bytes)} bytes: {cmd_bytes.hex()}")
    
    # Verify structure
    decoder = BincodeDecoder(cmd_bytes)
    variant = decoder.read_u32()
    mode_str = decoder.read_string()
    assert variant == 0
    assert mode_str == "waypoint"
    print("✓ SetMode command OK")
    
    # Test WaypointComplete
    print("\n3. Testing WaypointComplete event:")
    encoder = BincodeEncoder()
    encoder.write_u32(1)  # WaypointComplete variant
    encoder.write_vec_f32_array3([(1.0, 2.0, 3.0), (4.0, 5.0, 6.0)])
    
    event_bytes = encoder.bytes()
    event = decode_viewer_event(event_bytes)
    print(f"Decoded: {event}")
    assert isinstance(event, WaypointCompleteEvent)
    assert len(event.waypoints) == 2
    assert event.waypoints[0] == (1.0, 2.0, 3.0)
    assert event.waypoints[1] == (4.0, 5.0, 6.0)
    print("✓ WaypointComplete OK")
    
    print("\n✅ All codec tests passed!")
