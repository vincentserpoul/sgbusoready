//! Pure, incremental Server-Sent Events line parser — no IO. The blocking
//! stream client (`worker::client::run_stream`) feeds it one line at a time
//! from a `BufRead`; this module only knows about strings.

/// One dispatched SSE event: `event:` (defaults to `"message"` per the SSE
/// spec when unset) plus the accumulated, newline-joined `data:` lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Accumulates `event:`/`data:` fields across lines until a blank line
/// dispatches them. One parser per connection; state resets after every
/// dispatch (even an empty one).
#[derive(Debug, Default)]
pub struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Feed one line, without its trailing `\n` (a trailing `\r` from CRLF
    /// streams is stripped here). Returns `Some(event)` on the blank line that
    /// terminates an event with non-empty accumulated data; otherwise `None`.
    ///
    /// Field rules: `:`-prefixed lines are comments (ignored); `event:` sets
    /// the event name; `data:` lines accumulate (joined with `\n` on
    /// dispatch); an optional single leading space after the colon is
    /// stripped; unrecognised fields are ignored.
    pub fn push_line(&mut self, line: &str) -> Option<SseEvent> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            self.event = Some(strip_one_leading_space(rest).to_owned());
        } else if let Some(rest) = line.strip_prefix("data:") {
            self.data.push(strip_one_leading_space(rest).to_owned());
        }
        None
    }

    fn dispatch(&mut self) -> Option<SseEvent> {
        let event = self.event.take().unwrap_or_else(|| "message".to_owned());
        let data = self.data.join("\n");
        self.data.clear();
        if data.is_empty() {
            None
        } else {
            Some(SseEvent { event, data })
        }
    }
}

fn strip_one_leading_space(s: &str) -> &str {
    s.strip_prefix(' ').unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::{SseEvent, SseParser};

    fn feed(lines: &[&str]) -> Vec<SseEvent> {
        let mut parser = SseParser::default();
        lines
            .iter()
            .filter_map(|line| parser.push_line(line))
            .collect()
    }

    #[test]
    fn comment_lines_produce_no_event() {
        let events = feed(&[": this is a comment", ""]);
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn multi_line_data_is_newline_joined() {
        let events = feed(&["event: arrivals", "data: line one", "data: line two", ""]);
        assert_eq!(
            events,
            vec![SseEvent {
                event: "arrivals".to_owned(),
                data: "line one\nline two".to_owned(),
            }]
        );
    }

    #[test]
    fn crlf_line_endings_are_stripped() {
        let events = feed(&["event: arrivals\r", "data: hi\r", "\r"]);
        assert_eq!(
            events,
            vec![SseEvent {
                event: "arrivals".to_owned(),
                data: "hi".to_owned(),
            }]
        );
    }

    #[test]
    fn interleaved_events_dispatch_independently() {
        let events = feed(&["event: a", "data: 1", "", "event: b", "data: 2", ""]);
        assert_eq!(
            events,
            vec![
                SseEvent {
                    event: "a".to_owned(),
                    data: "1".to_owned(),
                },
                SseEvent {
                    event: "b".to_owned(),
                    data: "2".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn blank_line_without_data_dispatches_nothing() {
        let events = feed(&["event: arrivals", ""]);
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let events = feed(&["id: 123", "retry: 5000", "event: arrivals", "data: ok", ""]);
        assert_eq!(
            events,
            vec![SseEvent {
                event: "arrivals".to_owned(),
                data: "ok".to_owned(),
            }]
        );
    }

    #[test]
    fn event_without_a_type_defaults_to_message() {
        let events = feed(&["data: hi", ""]);
        assert_eq!(
            events,
            vec![SseEvent {
                event: "message".to_owned(),
                data: "hi".to_owned(),
            }]
        );
    }

    #[test]
    fn only_one_leading_space_is_stripped() {
        let events = feed(&["event: arrivals", "data:  two spaces", ""]);
        assert_eq!(
            events,
            vec![SseEvent {
                event: "arrivals".to_owned(),
                data: " two spaces".to_owned(),
            }]
        );
    }
}
