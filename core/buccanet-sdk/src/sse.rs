//! Splits a `text/event-stream` body into records, tolerant of chunk boundaries and comments.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseRecord {
    pub id: Option<String>,
    pub data: String,
}

pub fn parse_record(block: &str) -> Option<SseRecord> {
    let mut id = None;
    let mut data: Vec<&str> = Vec::new();
    for line in block.split('\n') {
        if line.starts_with(':') || line.trim().is_empty() {
            continue;
        }
        let (field, value) = match line.find(':') {
            Some(colon) => (&line[..colon], line[colon + 1..].strip_prefix(' ').unwrap_or(&line[colon + 1..])),
            None => (line, ""),
        };
        match field {
            "id" => id = Some(value.to_string()),
            "data" => data.push(value),
            _ => {}
        }
    }
    (!data.is_empty()).then(|| SseRecord { id, data: data.join("\n") })
}

/// Largest record the stream may send; the biggest inbox event, a Welcome to a full room,
/// stays far below it.
pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct RecordTooLarge;

/// Accumulates byte chunks and yields complete records; a UTF-8 character split across
/// chunks is only decoded once its record is complete.
#[derive(Default)]
pub struct SseBuffer {
    buffer: Vec<u8>,
    /// Bytes already searched for a record boundary.
    scanned: usize,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseRecord>, RecordTooLarge> {
        self.buffer.extend(chunk.iter().copied().filter(|byte| *byte != b'\r'));
        let mut records = Vec::new();
        while let Some(boundary) = self.next_boundary() {
            let block: Vec<u8> = self.buffer.drain(..boundary + 2).collect();
            self.scanned = 0;
            if let Some(record) = parse_record(&String::from_utf8_lossy(&block)) {
                records.push(record);
            }
        }
        if self.buffer.len() > MAX_RECORD_BYTES {
            return Err(RecordTooLarge);
        }
        Ok(records)
    }

    fn next_boundary(&mut self) -> Option<usize> {
        let start = self.scanned.saturating_sub(1);
        let found = self.buffer[start..].windows(2).position(|pair| pair == b"\n\n").map(|at| start + at);
        if found.is_none() {
            self.scanned = self.buffer.len();
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_across_chunks_and_skips_comments() {
        let mut buffer = SseBuffer::default();
        assert!(buffer.push(b"id: 1\ndata: {\"a\":\xc3").unwrap().is_empty());
        let records = buffer.push(b"\xa9}\n\n: ping\n\nid: 2\ndata: x\ndata: y\n\n").unwrap();
        assert_eq!(
            records,
            vec![
                SseRecord { id: Some("1".into()), data: "{\"a\":\u{e9}}".into() },
                SseRecord { id: Some("2".into()), data: "x\ny".into() },
            ]
        );
    }

    #[test]
    fn finds_a_boundary_split_across_chunks_and_caps_records() {
        let mut buffer = SseBuffer::default();
        assert!(buffer.push(b"data: a\n").unwrap().is_empty());
        assert_eq!(buffer.push(b"\n").unwrap(), vec![SseRecord { id: None, data: "a".into() }]);
        assert_eq!(buffer.push(&vec![b'x'; MAX_RECORD_BYTES + 1]), Err(RecordTooLarge));
    }
}
