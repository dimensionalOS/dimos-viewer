"""ViewerBridge - Python client for interactive Rerun viewer (bincode protocol).

Connects to the custom Rerun viewer over TCP and provides a callback-based API
for handling click events, waypoint completions, and sending commands back to
the viewer.

Uses bincode serialization to match the Rust viewer protocol exactly.

Example usage:
    from bridge_bincode import ViewerBridge
    from bincode_codec import ClickEvent
    
    bridge = ViewerBridge(port=8888)
    
    @bridge.on_click
    def handle_click(event: ClickEvent):
        print(f"Clicked at {event.position}")
    
    bridge.start()  # Blocks until viewer disconnects
"""

import socket
import struct
import threading
from typing import Callable, Optional
from dataclasses import dataclass
from enum import Enum

from bincode_codec import (
    ViewerEvent, ClickEvent, WaypointCompleteEvent, ModeChangedEvent, DisconnectEvent,
    AppCommand, SetModeCommand, ClearWaypointsCommand, SetCursorCommand,
    decode_viewer_event, encode_app_command,
)


class InteractionMode(Enum):
    """Viewer interaction modes."""
    NORMAL = "normal"  # Standard rerun behavior (no events sent)
    CLICK = "click"  # Single click reports position
    WAYPOINT = "waypoint"  # Sequential clicks build a route


@dataclass
class WaypointRoute:
    """Convenience wrapper for waypoint completion events."""
    waypoints: list[tuple[float, float, float]]
    
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


class ViewerBridge:
    """TCP client for bidirectional communication with the interactive Rerun viewer.
    
    Connects to the custom viewer, receives click/waypoint events (bincode), and can
    send commands back to the viewer (bincode).
    
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
    
    def send_command(self, command: AppCommand):
        """Send a command to the viewer (bincode-encoded).
        
        Args:
            command: AppCommand instance (SetModeCommand, etc.)
        
        Raises:
            RuntimeError: If not connected
        """
        if not self.socket:
            raise RuntimeError("Not connected to viewer")
        
        try:
            data = encode_app_command(command)
            length = struct.pack('>I', len(data))  # Big-endian u32 for length prefix
            self.socket.sendall(length + data)
        except (socket.error, BrokenPipeError) as e:
            print(f"Failed to send command: {e}")
            self.running = False
    
    def set_mode(self, mode: InteractionMode):
        """Change the viewer interaction mode.
        
        Args:
            mode: Target interaction mode
        """
        self.send_command(SetModeCommand(mode=mode.value))
    
    def clear_waypoints(self):
        """Clear all waypoint markers in the viewer."""
        self.send_command(ClearWaypointsCommand())
    
    def set_cursor(self, cursor: str):
        """Set the viewer cursor style.
        
        Args:
            cursor: Cursor name ("default", "crosshair", "pointer")
        """
        self.send_command(SetCursorCommand(cursor=cursor))
    
    def start(self, blocking: bool = True):
        """Start the bridge (listen for viewer connections and handle events).
        
        Args:
            blocking: If True, blocks until viewer disconnects.
                      If False, runs in background thread.
        
        Raises:
            ConnectionError: If cannot bind the server socket
        """
        # Create and bind the server socket
        self._server_socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._server_socket.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            self._server_socket.bind((self.host, self.port))
            self._server_socket.listen(1)
            print(f"Waiting for viewer connection on {self.host}:{self.port}...")
        except socket.error as e:
            raise ConnectionError(f"Failed to start server: {e}")
        
        self.running = True
        
        if blocking:
            self._accept_and_run()
        else:
            self.thread = threading.Thread(target=self._accept_and_run, daemon=True)
            self.thread.start()
    
    def _accept_and_run(self):
        """Accept a viewer connection, then enter the event loop."""
        try:
            self._server_socket.settimeout(30.0)  # Don't block forever
            self.socket, addr = self._server_socket.accept()
            print(f"Viewer connected from {addr}")
            self._run_loop()
        except socket.timeout:
            print("No viewer connected within timeout")
        except OSError:
            pass  # Socket closed during shutdown
        finally:
            if self._server_socket:
                try:
                    self._server_socket.close()
                except Exception:
                    pass
    
    def stop(self):
        """Stop the bridge and disconnect."""
        self.running = False
        if hasattr(self, '_server_socket') and self._server_socket:
            try:
                self._server_socket.close()
            except Exception:
                pass
            self._server_socket = None
        if self.socket:
            try:
                self.socket.close()
            except Exception:
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
                
                # Read message body (bincode-encoded ViewerEvent)
                message_data = self._recv_exact(length)
                if not message_data:
                    break
                
                # Decode and dispatch
                self._handle_message(message_data)
        
        except Exception as e:
            print(f"Bridge error: {e}")
            import traceback
            traceback.print_exc()
        
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
        """Parse and dispatch a viewer event message (bincode-encoded)."""
        try:
            event = decode_viewer_event(data)
            
            if isinstance(event, ClickEvent):
                for handler in self._click_handlers:
                    try:
                        handler(event)
                    except Exception as e:
                        print(f"Click handler error: {e}")
                        import traceback
                        traceback.print_exc()
            
            elif isinstance(event, WaypointCompleteEvent):
                route = WaypointRoute(waypoints=event.waypoints)
                
                for handler in self._waypoint_handlers:
                    try:
                        handler(route)
                    except Exception as e:
                        print(f"Waypoint handler error: {e}")
                        import traceback
                        traceback.print_exc()
            
            elif isinstance(event, ModeChangedEvent):
                for handler in self._mode_changed_handlers:
                    try:
                        handler(event.mode)
                    except Exception as e:
                        print(f"Mode handler error: {e}")
                        import traceback
                        traceback.print_exc()
            
            elif isinstance(event, DisconnectEvent):
                self.running = False
            
            else:
                print(f"Unknown event type: {type(event)}")
        
        except Exception as e:
            print(f"Failed to parse message: {e}")
            import traceback
            traceback.print_exc()


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
