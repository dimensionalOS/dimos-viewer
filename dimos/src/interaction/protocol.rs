use serde::{Deserialize, Serialize};

/// Events sent from the viewer to the application.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ViewerEvent {
    /// User clicked in a spatial view.
    Click {
        position: [f32; 3],
        entity_path: Option<String>,
        view_id: String,
        timestamp_ms: u64,
        is_2d: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_viewer_event_click_roundtrip() {
        let event = ViewerEvent::Click {
            position: [1.0, 2.0, 3.0],
            entity_path: Some("world/robot".to_string()),
            view_id: "view_123".to_string(),
            timestamp_ms: 1234567890,
            is_2d: false,
        };

        let encoded = bincode::serialize(&event).unwrap();
        let decoded: ViewerEvent = bincode::deserialize(&encoded).unwrap();

        assert_eq!(event, decoded);
    }
}
