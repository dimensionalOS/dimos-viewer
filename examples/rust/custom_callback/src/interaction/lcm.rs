//! LCM (Lightweight Communications and Marshalling) publisher for click events.
//!
//! Publishes `geometry_msgs/PointStamped` messages over UDP multicast,
//! following the same convention as RViz's `/clicked_point` topic.
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

/// Encodes a `PointStamped` LCM message (with fingerprint hash prefix).
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

/// Builds a complete LCM UDP packet (short message format).
///
/// Format: `[4B magic] [4B seqno] [channel\0] [payload]`
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

/// LCM publisher that sends PointStamped messages via UDP multicast.
pub struct LcmPublisher {
    socket: UdpSocket,
    seq: AtomicU32,
    channel: String,
}

impl LcmPublisher {
    /// Create a new LCM publisher.
    ///
    /// `channel` is the LCM channel name, e.g.
    /// `"/clicked_point#geometry_msgs.PointStamped"`.
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
}

impl std::fmt::Debug for LcmPublisher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LcmPublisher")
            .field("channel", &self.channel)
            .field("seq", &self.seq.load(Ordering::Relaxed))
            .finish()
    }
}

/// Create a `ClickEvent` from position, entity path, and a millisecond timestamp.
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

/// Create a `ClickEvent` from position and entity path, using the current time.
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
    fn test_lcm_publisher_creation() {
        let publisher = LcmPublisher::new("/clicked_point#geometry_msgs.PointStamped".to_string());
        assert!(publisher.is_ok());
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
