"""ViewerBridge - Python client for interactive Rerun viewer.

Connects to the custom Rerun viewer over TCP and provides a callback-based API
for handling click events, waypoint completions, and sending commands back to
the viewer.

Example usage:
    from viewer_bridge import ViewerBridge, ClickEvent
    
    bridge = ViewerBridge(port=8888)
    
    @bridge.on_click
    def handle_click(event: ClickEvent):
        print(f"Clicked at ({event.x}, {event.y}, {event.z})")
    
    bridge.start()  # Blocks until viewer disconnects
"""

import socket
import struct
import threading
from dataclasses import dataclass
from enum import Enum
from typing import Callable, Optional
import time


@dataclass
class ClickEvent:
    """Event emitted when the user clicks in a spatial view."""
    
    x: float  # World-space X coordinate
    y: float  # World-space Y coordinate
    z: float  # World-space Z coordinate
    entity_path: Optional[str]  # Rerun entity path that was clicked (if any)
    view_id: str  # Which spatial view the click occurred in
    timestamp: float  # Unix timestamp of the click (seconds)
    is_2d: bool  # Whether this was a 2D or 3D view click
    
    @property
    def position(self) -> tuple[float, float, float]:
        """Get position as a tuple."""
        return (self.x, self.y, self.z)


@dataclass
class WaypointRoute:
    """Event emitted when a waypoint sequence is completed."""
    
    waypoints: list[tuple[float, float, float]]  # Ordered list of (x, y, z)
    timestamp: float  # Unix timestamp of completion
    
    @property
    def total_distance(self) -> float:
        """Compute total path distance (sum of segment lengths)."""
        if len(self.waypoints) < 2:
            return 0.0
        
        distance = 0.0
        for i in range(len(self.waypoints) - 1):
            p1 = self.waypoints[i]
            p2 = self.waypoints[i + 1]
            dx = p2[0] - p1[0]
            dy = p2[1] - p1[1]
            dz = p2[2] - p1[2]
            distance += (dx**2 + dy**2 + dz**2) ** 0.5
        
        return distance


class InteractionMode(Enum):
    """Viewer interaction modes."""
    NORMAL = "normal"  # Standard rerun behavior (no events sent)
    CLICK = "click"  # Single click reports position
    WAYPOINT = "waypoint"  # Sequential clicks build a route


class ViewerBridge:
    """TCP client for bidirectional communication with the interactive Rerun viewer.
    
    Connects to the custom viewer, receives click/waypoint events, and can send
    commands back to the viewer (mode changes, cursor updates, etc.).
    
    Thread-safe: callbacks are invoked on a background thread.
    """
    
    def __init__(self, host: str = "127.0.0.1", port: int = 8888):
        """Initialize the bridge.
        
        Args:
            host: Viewer TCP server hostname/IP
            port: Viewer TCP server port
        """
        self.host = host
        self.port = port
        self.socket: Optional[socket.socket] = None
        self.running = False
        self.thread: Optional[threading.Thread] = None
        
        # Callback registrations
        self._click_handlers: list[Callable[[ClickEvent], None]] = []
        self._waypoint_handlers: list[Callable[[WaypointRoute], None]] = []
        self._mode_changed_handlers: list[Callable[[str], None]] = []
        self._disconnect_handlers: list[Callable[[], None]] = []
    
    def on_click(self, handler: Callable[[ClickEvent], None]):
        """Register a click event handler (decorator).
        
        Example:
            @bridge.on_click
            def handle_click(event: ClickEvent):
                print(f"Clicked at {event.position}")
        """
        self._click_handlers.append(handler)
        return handler
    
    def on_waypoint_complete(self, handler: Callable[[WaypointRoute], None]):
        """Register a waypoint completion handler (decorator).
        
        Example:
            @bridge.on_waypoint_complete
            def handle_route(route: WaypointRoute):
                print(f"Route: {len(route.waypoints)} waypoints")
        """
        self._waypoint_handlers.append(handler)
        return handler
    
    def on_mode_changed(self, handler: Callable[[str], None]):
        """Register a mode changed handler (decorator)."""
        self._mode_changed_handlers.append(handler)
        return handler
    
    def on_disconnect(self, handler: Callable[[], None]):
        """Register a disconnect handler (decorator)."""
        self._disconnect_handlers.append(handler)
        return handler
    
    def send_command(self, command: dict):
        """Send a command to the viewer.
        
        Args:
            command: Dict with "type" key and command-specific fields
                     e.g. {"type": "SetMode", "mode": "click"}
        
        Raises:
            RuntimeError: If not connected
        """
        if not self.socket:
            raise RuntimeError("Not connected to viewer")
        
        # For now, we'll implement a simple JSON protocol
        # In production, this should use the same bincode format as Rust
        import json
        data = json.dumps(command).encode('utf-8')
        length = struct.pack('>I', len(data))
        
        try:
            self.socket.sendall(length + data)
        except (socket.error, BrokenPipeError) as e:
            print(f"Failed to send command: {e}")
            self.running = False
    
    def set_mode(self, mode: InteractionMode):
        """Change the viewer interaction mode.
        
        Args:
            mode: Target interaction mode
        """
        self.send_command({
            "type": "SetMode",
            "mode": mode.value,
        })
    
    def clear_waypoints(self):
        """Clear all waypoint markers in the viewer."""
        self.send_command({"type": "ClearWaypoints"})
    
    def set_cursor(self, cursor: str):
        """Set the viewer cursor style.
        
        Args:
            cursor: Cursor name ("default", "crosshair", "pointer")
        """
        self.send_command({
            "type": "SetCursor",
            "cursor": cursor,
        })
    
    def start(self, blocking: bool = True):
        """Start the bridge (connect and listen for events).
        
        Args:
            blocking: If True, blocks until viewer disconnects.
                      If False, runs in background thread.
        
        Raises:
            ConnectionError: If cannot connect to viewer
        """
        # Connect to viewer
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.socket.connect((self.host, self.port))
            print(f"Connected to viewer at {self.host}:{self.port}")
        except socket.error as e:
            raise ConnectionError(f"Failed to connect to viewer: {e}")
        
        self.running = True
        
        if blocking:
            self._run_loop()
        else:
            self.thread = threading.Thread(target=self._run_loop, daemon=True)
            self.thread.start()
    
    def stop(self):
        """Stop the bridge and disconnect."""
        self.running = False
        if self.socket:
            try:
                self.socket.close()
            except:
                pass
            self.socket = None
    
    def _run_loop(self):
        """Main event loop (runs on background thread if non-blocking)."""
        try:
            while self.running:
                # Read message length (4 bytes, big-endian u32)
                length_data = self._recv_exact(4)
                if not length_data:
                    break
                
                length = struct.unpack('>I', length_data)[0]
                
                # Read message body
                message_data = self._recv_exact(length)
                if not message_data:
                    break
                
                # Parse and dispatch
                self._handle_message(message_data)
        
        except Exception as e:
            print(f"Bridge error: {e}")
        
        finally:
            self.running = False
            if self.socket:
                try:
                    self.socket.close()
                except:
                    pass
            
            # Notify disconnect handlers
            for handler in self._disconnect_handlers:
                try:
                    handler()
                except Exception as e:
                    print(f"Disconnect handler error: {e}")
    
    def _recv_exact(self, n: int) -> Optional[bytes]:
        """Receive exactly n bytes or return None on disconnect."""
        data = b''
        while len(data) < n:
            chunk = self.socket.recv(n - len(data))
            if not chunk:
                return None
            data += chunk
        return data
    
    def _handle_message(self, data: bytes):
        """Parse and dispatch a viewer event message.
        
        For now, uses JSON for simplicity. In production, should use bincode
        to match the Rust side exactly.
        """
        import json
        
        try:
            event = json.loads(data.decode('utf-8'))
            event_type = event.get("type")
            
            if event_type == "Click":
                click_event = ClickEvent(
                    x=event["position"][0],
                    y=event["position"][1],
                    z=event["position"][2],
                    entity_path=event.get("entity_path"),
                    view_id=event["view_id"],
                    timestamp=event["timestamp_ms"] / 1000.0,
                    is_2d=event["is_2d"],
                )
                
                for handler in self._click_handlers:
                    try:
                        handler(click_event)
                    except Exception as e:
                        print(f"Click handler error: {e}")
            
            elif event_type == "WaypointComplete":
                route = WaypointRoute(
                    waypoints=[tuple(wp) for wp in event["waypoints"]],
                    timestamp=time.time(),
                )
                
                for handler in self._waypoint_handlers:
                    try:
                        handler(route)
                    except Exception as e:
                        print(f"Waypoint handler error: {e}")
            
            elif event_type == "ModeChanged":
                mode = event["mode"]
                
                for handler in self._mode_changed_handlers:
                    try:
                        handler(mode)
                    except Exception as e:
                        print(f"Mode handler error: {e}")
            
            elif event_type == "Disconnect":
                self.running = False
            
            else:
                print(f"Unknown event type: {event_type}")
        
        except Exception as e:
            print(f"Failed to parse message: {e}")


if __name__ == "__main__":
    # Example usage
    bridge = ViewerBridge(port=8888)
    
    @bridge.on_click
    def handle_click(event: ClickEvent):
        print(f"Clicked at ({event.x:.2f}, {event.y:.2f}, {event.z:.2f})")
        if event.entity_path:
            print(f"  Entity: {event.entity_path}")
        print(f"  View: {event.view_id}")
    
    @bridge.on_waypoint_complete
    def handle_route(route: WaypointRoute):
        print(f"Route completed: {len(route.waypoints)} waypoints")
        print(f"Total distance: {route.total_distance:.2f} meters")
        for i, (x, y, z) in enumerate(route.waypoints):
            print(f"  {i+1}. ({x:.2f}, {y:.2f}, {z:.2f})")
    
    @bridge.on_disconnect
    def handle_disconnect():
        print("Viewer disconnected")
    
    print("Waiting for viewer connection...")
    try:
        bridge.start(blocking=True)
    except KeyboardInterrupt:
        print("\nShutting down...")
        bridge.stop()
