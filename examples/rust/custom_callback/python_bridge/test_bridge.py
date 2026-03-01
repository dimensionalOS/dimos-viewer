"""Unit tests for ViewerBridge."""

import unittest
import socket
import struct
import json
import threading
import time
from bridge import ViewerBridge, ClickEvent, WaypointRoute, InteractionMode


class MockViewerServer:
    """Mock TCP server simulating the viewer side."""
    
    def __init__(self, port: int = 8889):
        self.port = port
        self.server = None
        self.client = None
        self.thread = None
        self.running = False
        self.received_commands = []
    
    def start(self):
        """Start the mock server in a background thread."""
        self.server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.server.bind(("127.0.0.1", self.port))
        self.server.listen(1)
        
        self.running = True
        self.thread = threading.Thread(target=self._accept_loop, daemon=True)
        self.thread.start()
        time.sleep(0.1)  # Give server time to start
    
    def stop(self):
        """Stop the mock server."""
        self.running = False
        if self.client:
            try:
                self.client.close()
            except:
                pass
        if self.server:
            try:
                self.server.close()
            except:
                pass
    
    def _accept_loop(self):
        """Accept client connection."""
        try:
            self.client, _ = self.server.accept()
        except:
            return
    
    def send_click_event(self, x: float, y: float, z: float, entity_path: str = None):
        """Send a click event to the connected client."""
        event = {
            "type": "Click",
            "position": [x, y, z],
            "entity_path": entity_path,
            "view_id": "test_view",
            "timestamp_ms": int(time.time() * 1000),
            "is_2d": False,
        }
        self._send_json(event)
    
    def send_waypoint_complete(self, waypoints: list):
        """Send a waypoint complete event."""
        event = {
            "type": "WaypointComplete",
            "waypoints": waypoints,
        }
        self._send_json(event)
    
    def _send_json(self, data: dict):
        """Send JSON message with length prefix."""
        if not self.client:
            raise RuntimeError("No client connected")
        
        payload = json.dumps(data).encode('utf-8')
        length = struct.pack('>I', len(payload))
        self.client.sendall(length + payload)
    
    def read_command(self, timeout: float = 1.0) -> dict:
        """Read a command sent by the client."""
        if not self.client:
            raise RuntimeError("No client connected")
        
        self.client.settimeout(timeout)
        try:
            # Read length
            length_data = self.client.recv(4)
            if not length_data:
                return None
            
            length = struct.unpack('>I', length_data)[0]
            
            # Read payload
            payload = b''
            while len(payload) < length:
                chunk = self.client.recv(length - len(payload))
                if not chunk:
                    return None
                payload += chunk
            
            return json.loads(payload.decode('utf-8'))
        
        except socket.timeout:
            return None


class TestViewerBridge(unittest.TestCase):
    """Test cases for ViewerBridge."""
    
    def setUp(self):
        """Set up mock server and bridge for each test."""
        self.server = MockViewerServer(port=8889)
        self.server.start()
        
        self.bridge = ViewerBridge(port=8889)
        
        self.click_events = []
        self.waypoint_events = []
    
    def tearDown(self):
        """Clean up after each test."""
        self.bridge.stop()
        self.server.stop()
    
    def test_click_event_reception(self):
        """Test receiving a click event from the viewer."""
        @self.bridge.on_click
        def handle_click(event: ClickEvent):
            self.click_events.append(event)
        
        # Start bridge in non-blocking mode
        self.bridge.start(blocking=False)
        time.sleep(0.2)  # Wait for connection
        
        # Send click event from mock viewer
        self.server.send_click_event(1.0, 2.0, 3.0, "world/robot")
        
        # Wait for event to be processed
        time.sleep(0.2)
        
        # Verify event was received
        self.assertEqual(len(self.click_events), 1)
        event = self.click_events[0]
        self.assertAlmostEqual(event.x, 1.0)
        self.assertAlmostEqual(event.y, 2.0)
        self.assertAlmostEqual(event.z, 3.0)
        self.assertEqual(event.entity_path, "world/robot")
        self.assertEqual(event.position, (1.0, 2.0, 3.0))
    
    def test_waypoint_event_reception(self):
        """Test receiving a waypoint complete event."""
        @self.bridge.on_waypoint_complete
        def handle_route(route: WaypointRoute):
            self.waypoint_events.append(route)
        
        self.bridge.start(blocking=False)
        time.sleep(0.2)
        
        waypoints = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]]
        self.server.send_waypoint_complete(waypoints)
        
        time.sleep(0.2)
        
        self.assertEqual(len(self.waypoint_events), 1)
        route = self.waypoint_events[0]
        self.assertEqual(len(route.waypoints), 3)
        self.assertEqual(route.waypoints[0], (1.0, 2.0, 3.0))
        self.assertTrue(route.total_distance > 0)
    
    def test_send_command(self):
        """Test sending commands to the viewer."""
        self.bridge.start(blocking=False)
        time.sleep(0.2)
        
        # Send mode change command
        self.bridge.set_mode(InteractionMode.CLICK)
        
        # Read command from mock server
        cmd = self.server.read_command(timeout=1.0)
        
        self.assertIsNotNone(cmd)
        self.assertEqual(cmd["type"], "SetMode")
        self.assertEqual(cmd["mode"], "click")
    
    def test_multiple_handlers(self):
        """Test that multiple handlers can be registered for the same event."""
        calls_1 = []
        calls_2 = []
        
        @self.bridge.on_click
        def handler1(event):
            calls_1.append(event)
        
        @self.bridge.on_click
        def handler2(event):
            calls_2.append(event)
        
        self.bridge.start(blocking=False)
        time.sleep(0.2)
        
        self.server.send_click_event(1.0, 2.0, 3.0)
        time.sleep(0.2)
        
        self.assertEqual(len(calls_1), 1)
        self.assertEqual(len(calls_2), 1)


if __name__ == "__main__":
    unittest.main()
