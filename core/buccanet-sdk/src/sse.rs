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

/// Accumulates byte chunks and yields complete records; a UTF-8 character split across
/// chunks is only decoded once its record is complete.
#[derive(Default)]
pub struct SseBuffer {
    buffer: Vec<u8>,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseRecord> {
        self.buffer.extend(chunk.iter().copied().filter(|byte| *byte != b'\r'));
        let mut records = Vec::new();
        while let Some(boundary) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
            let block: Vec<u8> = self.buffer.drain(..boundary + 2).collect();
            if let Some(record) = parse_record(&String::from_utf8_lossy(&block)) {
                records.push(record);
            }
        }
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_across_chunks_and_skips_comments() {
        let mut buffer = SseBuffer::default();
        assert!(buffer.push(b"id: 1\ndata: {\"a\":\xc3").is_empty());
        let records = buffer.push(b"\xa9}\n\n: ping\n\nid: 2\ndata: x\ndata: y\n\n");
        assert_eq!(
            records,
            vec![
                SseRecord { id: Some("1".into()), data: "{\"a\":\u{e9}}".into() },
                SseRecord { id: Some("2".into()), data: "x\ny".into() },
            ]
        );
    }
}
