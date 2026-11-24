//! Event cursors: opaque to clients, a map of room id to the last seq seen.

use std::collections::BTreeMap;

use buccanet_protocol::b64;

pub type Positions = BTreeMap<String, u64>;

pub fn encode(positions: &Positions) -> String {
    b64::encode(&serde_json::to_vec(positions).expect("positions always serialize"))
}

pub fn decode(cursor: &str) -> Option<Positions> {
    serde_json::from_slice(&b64::decode(cursor).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_garbage() {
        let positions: Positions = [("r1".to_string(), 4), ("r2".to_string(), 9)].into();
        assert_eq!(decode(&encode(&positions)), Some(positions));
        assert_eq!(decode("not a cursor"), None);
    }
}
