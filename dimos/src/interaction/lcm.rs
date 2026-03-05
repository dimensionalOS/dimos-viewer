//! LCM (Lightweight Communications and Marshalling) publisher for click events and twist commands.
//!
//! Publishes `geometry_msgs/PointStamped` and `geometry_msgs/TwistStamped` messages over UDP multicast,
//! following the same convention as RViz's `/clicked_point` and `/cmd_vel` topics.
//!
//! ## LCM Wire Protocol (short message)
//! ```text
//! [4B magic "LC02"] [4B seqno] [channel\0] [LCM-encoded payload]
//! ```
//!
//! ## PointStamped Binary Layout
//! ```text
//! [8B fingerprint hash] [Header (no hash)] [Point (no hash)]
//!
//! Header:
//!   [4B seq: i32] [4B stamp.sec: i32] [4B stamp.nsec: i32]
//!   [4B frame_id_len: i32 (including null)] [frame_id bytes] [null]
//!
//! Point:
//!   [8B x: f64] [8B y: f64] [8B z: f64]
//! ```
//!
//! ## TwistStamped Binary Layout
//! ```text
//! [8B fingerprint hash] [Header (no hash)] [Twist (no hash)]
//!
//! Header: (same as above)
//!
//! Twist:
//!   Vector3 linear:  [8B x: f64] [8B y: f64] [8B z: f64]
//!   Vector3 angular: [8B x: f64] [8B y: f64] [8B z: f64]
//! ```

use std::net::UdpSocket;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::SystemTime;

/// LCM multicast address and port (default LCM configuration).
const LCM_MULTICAST_ADDR: &str = "239.255.76.67:7667";

/// LCM short message magic number: "LC02" in ASCII.
const LCM_MAGIC_SHORT: u32 = 0x4c433032;

/// Pre-computed fingerprint hash for `geometry_msgs/PointStamped`.
///
/// Computed from the recursive hash chain:
/// - Time:          base=0xde1d24a3a8ecb648 → rot → 0xbc3a494751d96c91
/// - Header:        base=0xdbb33f5b4c19b8ea + Time → rot → 0x2fdb11453be64af7
/// - Point:         base=0x573f2fdd2f76508f → rot → 0xae7e5fba5eeca11e
/// - PointStamped:  base=0xf012413a2c8028c2 + Header + Point → rot → 0x9cd764738ea629af
const POINT_STAMPED_HASH: u64 = 0x9cd764738ea629af;

/// Pre-computed fingerprint hash for `geometry_msgs/TwistStamped`.
///
/// Computed from the recursive hash chain:
/// - Time:          base=0xde1d24a3a8ecb648 → rot → 0xbc3a494751d96c91
/// - Header:        base=0xdbb33f5b4c19b8ea + Time → rot → 0x2fdb11453be64af7
/// - Vector3:       base=0x573f2fdd2f76508f → rot → 0xae7e5fba5eeca11e
/// - Twist:         base=0x3a4144772922add7 + Vector3 + Vector3 → rot → 0x2e7c07d7cdf7e027
/// - TwistStamped:  base=0xf01245422c7b28c2 + Header + Twist → rot → 0x9cd2bcbe6cb2a7c0
const TWIST_STAMPED_HASH: u64 = 0x9cd2bcbe6cb2a7c0;

/// A click event with world-space coordinates and entity info.
#[derive(Debug, Clone)]
pub struct ClickEvent {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Rerun entity path (stored in frame_id per our convention).
    pub entity_path: String,
    /// Unix timestamp in seconds.
    pub timestamp_sec: i32,
    /// Nanosecond remainder.
    pub timestamp_nsec: i32,
}

/// A velocity command (maps to geometry_msgs/TwistStamped).
#[derive(Debug, Clone)]
pub struct TwistCommand {
    pub linear_x: f64,   // forward/backward
    pub linear_y: f64,   // strafe left/right
    pub linear_z: f64,   // up/down (unused for ground robots)
    pub angular_x: f64,  // roll (unused)
    pub angular_y: f64,  // pitch (unused)
    pub angular_z: f64,  // yaw left/right
    pub frame_id: String,
    pub timestamp_sec: i32,
    pub timestamp_nsec: i32,
}

/// Encodes a \`PointStamped\` LCM message (with fingerprint hash prefix).
///
/// Binary layout:
/// - 8 bytes: fingerprint hash (big-endian i64)
/// - Header (no hash): seq(i32) + stamp.sec(i32) + stamp.nsec(i32) + frame_id(len-prefixed string)
/// - Point (no hash): x(f64) + y(f64) + z(f64)
pub fn encode_point_stamped(event: &ClickEvent) -> Vec<u8> {
    let frame_id_bytes = event.entity_path.as_bytes();
    // LCM string encoding: i32 length (including null terminator) + bytes + null
    let string_len = (frame_id_bytes.len() + 1) as i32;

    // Calculate total size:
    // 8 (hash) + 4 (seq) + 4 (sec) + 4 (nsec) + 4 (string_len) + frame_id_bytes + 1 (null) + 24 (3 doubles)
    let total_size = 8 + 4 + 4 + 4 + 4 + frame_id_bytes.len() + 1 + 24;
    let mut buf = Vec::with_capacity(total_size);

    // Fingerprint hash (big-endian)
    buf.extend_from_slice(&POINT_STAMPED_HASH.to_be_bytes());

    // Header._encodeNoHash:
    //   seq (i32, big-endian) — always 0 for click events
    buf.extend_from_slice(&0i32.to_be_bytes());
    //   stamp.sec (i32)
    buf.extend_from_slice(&event.timestamp_sec.to_be_bytes());
    //   stamp.nsec (i32)
    buf.extend_from_slice(&event.timestamp_nsec.to_be_bytes());
    //   frame_id: string = i32 length (incl null) + bytes + null
    buf.extend_from_slice(&string_len.to_be_bytes());
    buf.extend_from_slice(frame_id_bytes);
    buf.push(0); // null terminator

    // Point._encodeNoHash:
    buf.extend_from_slice(&event.x.to_be_bytes());
    buf.extend_from_slice(&event.y.to_be_bytes());
    buf.extend_from_slice(&event.z.to_be_bytes());

    buf
}

/// Encodes a \`TwistStamped\` LCM message (with fingerprint hash prefix).
///
/// Binary layout:
/// - 8 bytes: fingerprint hash (big-endian u64)
/// - Header (no hash): seq(i32) + stamp.sec(i32) + stamp.nsec(i32) + frame_id(len-prefixed string)
/// - Twist (no hash): linear(Vector3: x,y,z f64) + angular(Vector3: x,y,z f64)
pub fn encode_twist_stamped(cmd: &TwistCommand) -> Vec<u8> {
    let frame_id_bytes = cmd.frame_id.as_bytes();
    // LCM string encoding: i32 length (including null terminator) + bytes + null
    let string_len = (frame_id_bytes.len() + 1) as i32;

    // Calculate total size:
    // 8 (hash) + 4 (seq) + 4 (sec) + 4 (nsec) + 4 (string_len) + frame_id_bytes + 1 (null) + 48 (6 doubles)
    let total_size = 8 + 4 + 4 + 4 + 4 + frame_id_bytes.len() + 1 + 48;
    let mut buf = Vec::with_capacity(total_size);

    // Fingerprint hash (big-endian)
    buf.extend_from_slice(&TWIST_STAMPED_HASH.to_be_bytes());

    // Header._encodeNoHash:
    //   seq (i32, big-endian) — always 0 for twist commands
    buf.extend_from_slice(&0i32.to_be_bytes());
    //   stamp.sec (i32)
    buf.extend_from_slice(&cmd.timestamp_sec.to_be_bytes());
    //   stamp.nsec (i32)
    buf.extend_from_slice(&cmd.timestamp_nsec.to_be_bytes());
    //   frame_id: string = i32 length (incl null) + bytes + null
    buf.extend_from_slice(&string_len.to_be_bytes());
    buf.extend_from_slice(frame_id_bytes);
    buf.push(0); // null terminator

    // Twist._encodeNoHash:
    // Vector3 linear:
    buf.extend_from_slice(&cmd.linear_x.to_be_bytes());
    buf.extend_from_slice(&cmd.linear_y.to_be_bytes());
    buf.extend_from_slice(&cmd.linear_z.to_be_bytes());
    // Vector3 angular:
    buf.extend_from_slice(&cmd.angular_x.to_be_bytes());
    buf.extend_from_slice(&cmd.angular_y.to_be_bytes());
    buf.extend_from_slice(&cmd.angular_z.to_be_bytes());

    buf
}

/// Builds a complete LCM UDP packet (short message format).
///
/// Format: \`[4B magic] [4B seqno] [channel\0] [payload]\`
pub fn build_lcm_packet(channel: &str, payload: &[u8], seq: u32) -> Vec<u8> {
    let channel_bytes = channel.as_bytes();
    let total = 4 + 4 + channel_bytes.len() + 1 + payload.len();
    let mut pkt = Vec::with_capacity(total);

    pkt.extend_from_slice(&LCM_MAGIC_SHORT.to_be_bytes());
    pkt.extend_from_slice(&seq.to_be_bytes());
    pkt.extend_from_slice(channel_bytes);
    pkt.push(0); // null terminator
    pkt.extend_from_slice(payload);

    pkt
}

/// LCM publisher that sends PointStamped and TwistStamped messages via UDP multicast.
pub struct LcmPublisher {
    socket: UdpSocket,
    seq: AtomicU32,
    channel: String,
}

impl LcmPublisher {
    /// Create a new LCM publisher.
    ///
    /// \`channel\` is the LCM channel name, e.g.
    /// \`"/clicked_point#geometry_msgs.PointStamped"\` or 
    /// \`"/cmd_vel#geometry_msgs.TwistStamped"\`.
    pub fn new(channel: String) -> std::io::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        // TTL=0 means local machine only; TTL=1 for same subnet
        socket.set_multicast_ttl_v4(0)?;
        Ok(Self {
            socket,
            seq: AtomicU32::new(0),
            channel,
        })
    }

    /// Publish a click event as a PointStamped LCM message.
    pub fn publish(&self, event: &ClickEvent) -> std::io::Result<usize> {
        let payload = encode_point_stamped(event);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let packet = build_lcm_packet(&self.channel, &payload, seq);
        self.socket.send_to(&packet, LCM_MULTICAST_ADDR)
    }

    /// Publish a twist command as a TwistStamped LCM message.
    pub fn publish_twist(&self, cmd: &TwistCommand) -> std::io::Result<usize> {
        let payload = encode_twist_stamped(cmd);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let packet = build_lcm_packet(&self.channel, &payload, seq);
        self.socket.send_to(&packet, LCM_MULTICAST_ADDR)
    }
}

impl std::fmt::Debug for LcmPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcmPublisher")
            .field("channel", &self.channel)
            .field("seq", &self.seq.load(Ordering::Relaxed))
            .finish()
    }
}

/// Create a \`ClickEvent\` from position, entity path, and a millisecond timestamp.
pub fn click_event_from_ms(
    position: [f32; 3],
    entity_path: &str,
    timestamp_ms: u64,
) -> ClickEvent {
    let total_secs = (timestamp_ms / 1000) as i32;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as i32;
    ClickEvent {
        x: position[0] as f64,
        y: position[1] as f64,
        z: position[2] as f64,
        entity_path: entity_path.to_string(),
        timestamp_sec: total_secs,
        timestamp_nsec: nanos,
    }
}

/// Create a \`ClickEvent\` from position and entity path, using the current time.
pub fn click_event_now(position: [f32; 3], entity_path: &str) -> ClickEvent {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    ClickEvent {
        x: position[0] as f64,
        y: position[1] as f64,
        z: position[2] as f64,
        entity_path: entity_path.to_string(),
        timestamp_sec: now.as_secs() as i32,
        timestamp_nsec: now.subsec_nanos() as i32,
    }
}

/// Create a \`TwistCommand\` from velocity values using current timestamp.
pub fn twist_command_now(
    linear: [f64; 3],
    angular: [f64; 3],
    frame_id: &str,
) -> TwistCommand {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    TwistCommand {
        linear_x: linear[0],
        linear_y: linear[1],
        linear_z: linear[2],
        angular_x: angular[0],
        angular_y: angular[1],
        angular_z: angular[2],
        frame_id: frame_id.to_string(),
        timestamp_sec: now.as_secs() as i32,
        timestamp_nsec: now.subsec_nanos() as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_stamped_fingerprint() {
        // Verify our pre-computed hash matches the LCM spec computation
        fn rot(h: u64) -> u64 {
            (h.wrapping_shl(1)).wrapping_add((h >> 63) & 1)
        }
        let time_hash = rot(0xde1d24a3a8ecb648);
        let header_hash = rot(0xdbb33f5b4c19b8ea_u64.wrapping_add(time_hash));
        let point_hash = rot(0x573f2fdd2f76508f);
        let ps_hash =
            rot(0xf012413a2c8028c2_u64
                .wrapping_add(header_hash)
                .wrapping_add(point_hash));
        assert_eq!(ps_hash, POINT_STAMPED_HASH);
    }

    #[test]
    fn test_twist_stamped_fingerprint() {
        // Verify our pre-computed hash matches the LCM spec computation
        fn rot(h: u64) -> u64 {
            (h.wrapping_shl(1)).wrapping_add((h >> 63) & 1)
        }
        let time_hash = rot(0xde1d24a3a8ecb648);
        let header_hash = rot(0xdbb33f5b4c19b8ea_u64.wrapping_add(time_hash));
        let vector3_hash = rot(0x573f2fdd2f76508f);
        let twist_hash = rot(0x3a4144772922add7_u64
            .wrapping_add(vector3_hash)
            .wrapping_add(vector3_hash));
        let ts_hash =
            rot(0xf01245422c7b28c2_u64
                .wrapping_add(header_hash)
                .wrapping_add(twist_hash));
        assert_eq!(ts_hash, TWIST_STAMPED_HASH);
    }

    #[test]
    fn test_encode_twist_stamped_matches_python() {
        // Test with known values verified against Python reference
        let cmd = TwistCommand {
            linear_x: 0.5,
            linear_y: 0.0,
            linear_z: 0.0,
            angular_x: 0.0,
            angular_y: 0.0,
            angular_z: 0.3,
            frame_id: "base_link".to_string(),
            timestamp_sec: 1234,
            timestamp_nsec: 5678,
        };

        let encoded = encode_twist_stamped(&cmd);

        // Expected from Python LCM encoding (82 bytes, verified):
        let expected_hex = "9cd2bcbe6cb2a7c000000000000004d20000162e0000000a626173655f6c696e6b003fe000000000000000000000000000000000000000000000000000000000000000000000000000003fd3333333333333";
        let expected: Vec<u8> = (0..expected_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&expected_hex[i..i + 2], 16).unwrap())
            .collect();

        assert_eq!(encoded, expected, "Encoded bytes must match Python LCM output");
        assert_eq!(encoded.len(), 82, "Encoded length must be 82 bytes");
    }

    #[test]
    fn test_encode_twist_stamped_empty_frame_id() {
        let cmd = TwistCommand {
            linear_x: 0.0,
            linear_y: 0.0,
            linear_z: 0.0,
            angular_x: 0.0,
            angular_y: 0.0,
            angular_z: 0.0,
            frame_id: String::new(),
            timestamp_sec: 0,
            timestamp_nsec: 0,
        };
        let encoded = encode_twist_stamped(&cmd);

        // Hash(8) + seq(4) + sec(4) + nsec(4) + strlen(4) + null(1) + 6*f64(48) = 73
        assert_eq!(encoded.len(), 73);

        // String length field should be 1 (just the null terminator)
        let str_len = i32::from_be_bytes([encoded[20], encoded[21], encoded[22], encoded[23]]);
        assert_eq!(str_len, 1);
    }

    #[test]
    fn test_encode_point_stamped_matches_python() {
        // Test with known values verified against Python lcm_msgs
        let event = ClickEvent {
            x: 1.5,
            y: 2.5,
            z: 3.5,
            entity_path: "/world/grid".to_string(),
            timestamp_sec: 1234,
            timestamp_nsec: 5678,
        };

        let encoded = encode_point_stamped(&event);

        // Expected from Python LCM encoding (verified):
        let expected_hex = "9cd764738ea629af00000000000004d20000162e0000000c2f776f726c642f67726964003ff80000000000004004000000000000400c000000000000";
        let expected: Vec<u8> = (0..expected_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&expected_hex[i..i + 2], 16).unwrap())
            .collect();

        assert_eq!(encoded, expected, "Encoded bytes must match Python LCM output");
    }

    #[test]
    fn test_encode_empty_frame_id() {
        let event = ClickEvent {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            entity_path: String::new(),
            timestamp_sec: 0,
            timestamp_nsec: 0,
        };
        let encoded = encode_point_stamped(&event);

        // Hash(8) + seq(4) + sec(4) + nsec(4) + strlen(4) + null(1) + 3*f64(24) = 49
        assert_eq!(encoded.len(), 49);

        // String length field should be 1 (just the null terminator)
        let str_len = i32::from_be_bytes([encoded[20], encoded[21], encoded[22], encoded[23]]);
        assert_eq!(str_len, 1);
    }

    #[test]
    fn test_build_lcm_packet_format() {
        let payload = vec![0xAA, 0xBB];
        let channel = "/test";
        let packet = build_lcm_packet(channel, &payload, 42);

        // Magic
        assert_eq!(&packet[0..4], &LCM_MAGIC_SHORT.to_be_bytes());
        // Sequence number
        assert_eq!(&packet[4..8], &42u32.to_be_bytes());
        // Channel (null-terminated)
        let null_pos = packet[8..].iter().position(|&b| b == 0).unwrap() + 8;
        let channel_bytes = &packet[8..null_pos];
        assert_eq!(channel_bytes, b"/test");
        // Payload follows null terminator
        assert_eq!(&packet[null_pos + 1..], &[0xAA, 0xBB]);
    }

    #[test]
    fn test_build_lcm_packet_with_typed_channel() {
        let payload = vec![0x01];
        let channel = "/clicked_point#geometry_msgs.PointStamped";
        let packet = build_lcm_packet(channel, &payload, 0);

        // Find the channel in the packet
        let null_pos = packet[8..].iter().position(|&b| b == 0).unwrap() + 8;
        let extracted_channel = std::str::from_utf8(&packet[8..null_pos]).unwrap();
        assert_eq!(extracted_channel, channel);
    }

    #[test]
    fn test_click_event_from_ms() {
        let event = click_event_from_ms([1.0, 2.0, 3.0], "/world", 1234567);
        assert_eq!(event.timestamp_sec, 1234);
        assert_eq!(event.timestamp_nsec, 567_000_000);
        assert_eq!(event.x, 1.0f64);
        assert_eq!(event.entity_path, "/world");
    }

    #[test]
    fn test_click_event_now() {
        let event = click_event_now([0.0, 0.0, 0.0], "/test");
        let now_sec = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i32;
        assert!((event.timestamp_sec - now_sec).abs() < 10);
    }

    #[test]
    fn test_twist_command_now() {
        let cmd = twist_command_now([1.0, 0.0, 0.0], [0.0, 0.0, 0.5], "base_link");
        assert_eq!(cmd.linear_x, 1.0);
        assert_eq!(cmd.angular_z, 0.5);
        assert_eq!(cmd.frame_id, "base_link");
        let now_sec = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i32;
        assert!((cmd.timestamp_sec - now_sec).abs() < 10);
    }

    #[test]
    fn test_lcm_publisher_creation() {
        let publisher = LcmPublisher::new("/clicked_point#geometry_msgs.PointStamped".to_string());
        assert!(publisher.is_ok());
        
        let publisher_twist = LcmPublisher::new("/cmd_vel#geometry_msgs.TwistStamped".to_string());
        assert!(publisher_twist.is_ok());
    }

    #[test]
    fn test_full_packet_structure() {
        let event = ClickEvent {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            entity_path: "/world/robot".to_string(),
            timestamp_sec: 100,
            timestamp_nsec: 200,
        };
        let payload = encode_point_stamped(&event);
        let channel = "/clicked_point#geometry_msgs.PointStamped";
        let packet = build_lcm_packet(channel, &payload, 7);

        // Verify magic
        let magic = u32::from_be_bytes([packet[0], packet[1], packet[2], packet[3]]);
        assert_eq!(magic, LCM_MAGIC_SHORT);

        // Verify seqno
        let seqno = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
        assert_eq!(seqno, 7);

        // Extract channel
        let null_pos = packet[8..].iter().position(|&b| b == 0).unwrap() + 8;
        let ch = std::str::from_utf8(&packet[8..null_pos]).unwrap();
        assert_eq!(ch, channel);

        // Verify payload hash
        let data_start = null_pos + 1;
        let hash_bytes: [u8; 8] = packet[data_start..data_start + 8].try_into().unwrap();
        let hash = u64::from_be_bytes(hash_bytes);
        assert_eq!(hash, POINT_STAMPED_HASH);
    }

    #[test]
    fn test_sequence_number_increments() {
        let publisher =
            LcmPublisher::new("/test#geometry_msgs.PointStamped".to_string()).unwrap();
        assert_eq!(publisher.seq.load(Ordering::Relaxed), 0);

        let seq1 = publisher.seq.fetch_add(1, Ordering::Relaxed);
        assert_eq!(seq1, 0);
        let seq2 = publisher.seq.fetch_add(1, Ordering::Relaxed);
        assert_eq!(seq2, 1);
    }
}
