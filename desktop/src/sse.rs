//! Server-sent events (`GET /api/events`), parsed incrementally from the chunks of the response: Home sends
//! `retry:`, `id: <boot>:<bus>` and `data:` lines, and a `: keepalive` comment every 15 s.

/// One complete event: its `id:` (when it had one) and its `data:` lines joined.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct Parser {
    buffer: String,
}

impl Parser {
    /// Adds a chunk and returns the events it completed. Comments (keepalives) and `retry:` are dropped; a block
    /// without data is not an event.
    pub fn push(&mut self, chunk: &str) -> Vec<Frame> {
        self.buffer.push_str(chunk);
        // Home sends `\n`; `\r\n` is folded (a `\r` at the end waits for its `\n` in the next chunk).
        if self.buffer.contains("\r\n") {
            self.buffer = self.buffer.replace("\r\n", "\n");
        }
        let mut frames = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let block: String = self.buffer.drain(..end + 2).collect();
            let (mut id, mut data) = (None, Vec::new());
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
                } else if let Some(rest) = line.strip_prefix("id:") {
                    id = Some(rest.trim().to_string());
                }
            }
            if !data.is_empty() {
                frames.push(Frame { id, data: data.join("\n") });
            }
        }
        frames
    }
}

/// `<boot>:<bus>` of an event id, the value `Last-Event-ID` sends back.
pub fn position(id: &str) -> Option<(String, u64)> {
    let (boot, bus) = id.rsplit_once(':')?;
    Some((boot.to_string(), bus.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_across_chunks_comments_and_crlf() {
        let mut p = Parser::default();
        assert!(p.push("retry: 3000\ndata: {\"type\":\"hel").is_empty());
        assert_eq!(p.push("lo\"}\n\n"), vec![Frame { id: None, data: "{\"type\":\"hello\"}".into() }]);
        assert!(p.push(": keepalive\n\n").is_empty());
        assert!(p.push("id: b1:42\r").is_empty());
        let got = p.push("\ndata: {\"a\":1}\r\n\r\nid: b1:43\ndata:x\ndata: y\n\n");
        assert_eq!(got, vec![Frame { id: Some("b1:42".into()), data: "{\"a\":1}".into() }, Frame { id: Some("b1:43".into()), data: "x\ny".into() }]);
        assert_eq!(position("b1:43"), Some(("b1".into(), 43)));
        assert_eq!(position("nonsense"), None);
    }
}
