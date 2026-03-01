"""Unit tests for ViewerBridge with bincode protocol."""

import unittest
import socket
import struct
import threading
import time
import os
from bincode_codec import (
    BincodeEncoder, BincodeDecoder, ClickEvent, WaypointCompleteEvent,
    SetModeCommand, ClearWaypointsCommand, SetCursorCommand,
    encode_app_command, decode_viewer_event,
)
from bridge_bincode import ViewerBridge, WaypointRoute, InteractionMode


def _free_port():
    """Get a free port from the OS."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class MockViewerClient:
    """Mock TCP client that simulates the Rust viewer connecting to the Python bridge.
    
    In the real architecture:
    - Python ViewerBridge is the SERVER (listens on port)
    - Rust viewer is the CLIENT (connects to bridge)
    
    So the mock simulates the Rust side: connect, send events, receive commands.
    """
    
    def __init__(self):
        self.socket = None
        self.received_commands = []
    
    def connect(self, host='127.0.0.1', port=8888, timeout=5.0):
        """Connect to the bridge server."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                self.socket.connect((host, port))
                return
            except ConnectionRefusedError:
                self.socket.close()
                time.sleep(0.05)
        raise ConnectionError(f"Could not connect to bridge at {host}:{port}")
    
    def send_click_event(self, x, y, z, entity_path=None, view_id="view_123", is_2d=False):
        """Send a Click event to the bridge."""
        encoder = BincodeEncoder()
        encoder.write_u32(0)  # Click variant
        encoder.write_f32_array3(x, y, z)
        
        if entity_path is None:
            encoder.buffer.append(0)  # Option::None
        else:
            encoder.buffer.append(1)  # Option::Some
            encoder.write_string(entity_path)
        
        encoder.write_string(view_id)
        encoder.write_u64(int(time.time() * 1000))
        encoder.write_bool(is_2d)
        
        data = encoder.bytes()
        length = struct.pack('>I', len(data))
        self.socket.sendall(length + data)
    
    def send_waypoint_complete_event(self, waypoints):
        """Send a WaypointComplete event to the bridge."""
        encoder = BincodeEncoder()
        encoder.write_u32(1)  # WaypointComplete variant
        encoder.write_vec_f32_array3(waypoints)
        
        data = encoder.bytes()
        length = struct.pack('>I', len(data))
        self.socket.sendall(length + data)
    
    def recv_command(self, timeout=2.0):
        """Receive a command from the bridge."""
        self.socket.settimeout(timeout)
        
        length_data = self._recv_exact(4)
        if not length_data:
            return None
        
        length = struct.unpack('>I', length_data)[0]
        
        data = self._recv_exact(length)
        self.received_commands.append(data)
        return data
    
    def _recv_exact(self, n):
        data = b''
        while len(data) < n:
            chunk = self.socket.recv(n - len(data))
            if not chunk:
                return None
            data += chunk
        return data
    
    def close(self):
        if self.socket:
            try:
                self.socket.close()
            except Exception:
                pass
            self.socket = None


class TestViewerBridgeBincode(unittest.TestCase):
    """Test suite for ViewerBridge with bincode protocol."""
    
    def setUp(self):
        self.port = _free_port()
        self.bridge = None
        self.mock = None
    
    def tearDown(self):
        if self.mock:
            self.mock.close()
        if self.bridge:
            self.bridge.stop()
        # Give sockets time to release
        time.sleep(0.05)
    
    def _start_bridge_and_connect(self):
        """Helper: start bridge in background, connect mock client."""
        self.bridge = ViewerBridge(port=self.port)
        # Start bridge (server) in background
        self.bridge.start(blocking=False)
        time.sleep(0.1)
        
        # Connect mock viewer client
        self.mock = MockViewerClient()
        self.mock.connect(port=self.port)
        time.sleep(0.1)
    
    def test_click_event_reception(self):
        """Test receiving and handling Click events."""
        click_received = threading.Event()
        received_event = None
        
        self.bridge = ViewerBridge(port=self.port)
        
        def handle_click(event):
            nonlocal received_event
            received_event = event
            click_received.set()
        
        self.bridge.on_click(handle_click)
        self.bridge.start(blocking=False)
        time.sleep(0.1)
        
        self.mock = MockViewerClient()
        self.mock.connect(port=self.port)
        time.sleep(0.1)
        
        # Send click event from mock viewer
        self.mock.send_click_event(1.5, 2.5, 3.5, entity_path="world/robot", view_id="3d_view")
        
        self.assertTrue(click_received.wait(timeout=2.0), "Click event not received")
        
        self.assertIsNotNone(received_event)
        self.assertAlmostEqual(received_event.position[0], 1.5, places=1)
        self.assertAlmostEqual(received_event.position[1], 2.5, places=1)
        self.assertAlmostEqual(received_event.position[2], 3.5, places=1)
        self.assertEqual(received_event.entity_path, "world/robot")
        self.assertEqual(received_event.view_id, "3d_view")
        self.assertFalse(received_event.is_2d)
    
    def test_waypoint_complete_reception(self):
        """Test receiving and handling WaypointComplete events."""
        waypoint_received = threading.Event()
        received_route = None
        
        self.bridge = ViewerBridge(port=self.port)
        
        def handle_waypoints(route):
            nonlocal received_route
            received_route = route
            waypoint_received.set()
        
        self.bridge.on_waypoint_complete(handle_waypoints)
        self.bridge.start(blocking=False)
        time.sleep(0.1)
        
        self.mock = MockViewerClient()
        self.mock.connect(port=self.port)
        time.sleep(0.1)
        
        waypoints = [(1.0, 2.0, 3.0), (4.0, 5.0, 6.0), (7.0, 8.0, 9.0)]
        self.mock.send_waypoint_complete_event(waypoints)
        
        self.assertTrue(waypoint_received.wait(timeout=2.0), "Waypoint event not received")
        
        self.assertIsNotNone(received_route)
        self.assertEqual(len(received_route.waypoints), 3)
        self.assertAlmostEqual(received_route.waypoints[0][0], 1.0, places=1)
        self.assertAlmostEqual(received_route.waypoints[1][0], 4.0, places=1)
        self.assertAlmostEqual(received_route.waypoints[2][0], 7.0, places=1)
        self.assertGreater(received_route.total_distance, 0)
    
    def test_send_set_mode_command(self):
        """Test sending SetMode command."""
        self._start_bridge_and_connect()
        
        self.bridge.set_mode(InteractionMode.WAYPOINT)
        
        cmd_data = self.mock.recv_command(timeout=2.0)
        self.assertIsNotNone(cmd_data, "Command not received")
        
        decoder = BincodeDecoder(cmd_data)
        variant = decoder.read_u32()
        mode = decoder.read_string()
        
        self.assertEqual(variant, 0)  # SetMode variant
        self.assertEqual(mode, "waypoint")
    
    def test_send_clear_waypoints_command(self):
        """Test sending ClearWaypoints command."""
        self._start_bridge_and_connect()
        
        self.bridge.clear_waypoints()
        
        cmd_data = self.mock.recv_command(timeout=2.0)
        self.assertIsNotNone(cmd_data)
        
        decoder = BincodeDecoder(cmd_data)
        variant = decoder.read_u32()
        self.assertEqual(variant, 1)  # ClearWaypoints variant
    
    def test_send_set_cursor_command(self):
        """Test sending SetCursor command."""
        self._start_bridge_and_connect()
        
        self.bridge.set_cursor("crosshair")
        
        cmd_data = self.mock.recv_command(timeout=2.0)
        self.assertIsNotNone(cmd_data)
        
        decoder = BincodeDecoder(cmd_data)
        variant = decoder.read_u32()
        cursor = decoder.read_string()
        
        self.assertEqual(variant, 2)  # SetCursor variant
        self.assertEqual(cursor, "crosshair")
    
    def test_multiple_click_handlers(self):
        """Test registering multiple click handlers."""
        clicks = []
        done = threading.Event()
        
        self.bridge = ViewerBridge(port=self.port)
        
        @self.bridge.on_click
        def handler1(event):
            clicks.append(("handler1", event.position))
            if len(clicks) >= 2:
                done.set()
        
        @self.bridge.on_click
        def handler2(event):
            clicks.append(("handler2", event.position))
            if len(clicks) >= 2:
                done.set()
        
        self.bridge.start(blocking=False)
        time.sleep(0.1)
        
        self.mock = MockViewerClient()
        self.mock.connect(port=self.port)
        time.sleep(0.1)
        
        self.mock.send_click_event(1.0, 2.0, 3.0)
        
        self.assertTrue(done.wait(timeout=2.0), "Handlers not called")
        self.assertEqual(len(clicks), 2)
        self.assertEqual(clicks[0][0], "handler1")
        self.assertEqual(clicks[1][0], "handler2")


if __name__ == '__main__':
    unittest.main()
