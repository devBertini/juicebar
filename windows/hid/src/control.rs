// Copyright (c) 2026 Claudio Bertini (devBertini). MIT License.
//! The control queue the app thread and the device-notification callback use to talk to the
//! I/O thread, and the padding of outgoing reports.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;

use crate::endpoint::EndpointId;

/// A message for the I/O thread.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Control {
    /// Write one report, already padded to the endpoint's output length.
    Send { id: EndpointId, report: Vec<u8> },
    /// A HID interface appeared (device interface path).
    Arrive(String),
    /// A HID interface went away (device interface path).
    Remove(String),
    /// Close everything and end the thread.
    Stop,
}

/// First in, first out; many producers, one consumer. The caller signals the I/O thread's
/// control event when `push` says the queue was empty.
#[derive(Default)]
pub(crate) struct ControlQueue {
    inner: Mutex<VecDeque<Control>>,
}

impl ControlQueue {
    /// Queue a message. Returns true when the queue was empty before, so the consumer needs a signal.
    pub(crate) fn push(&self, message: Control) -> bool {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let was_empty = q.is_empty();
        q.push_back(message);
        was_empty
    }

    /// Take every queued message, oldest first.
    pub(crate) fn drain(&self) -> Vec<Control> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.drain(..).collect()
    }
}

/// Why `Transport::send` refused a report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    /// No open endpoint has this id (never seen, or already removed).
    UnknownEndpoint,
    /// The collection takes no output reports.
    NoOutputReport,
    /// The report is longer than the endpoint's output report.
    TooLong { len: usize, max: usize },
    /// An empty report has no report ID.
    Empty,
    /// The transport was stopped.
    Stopped,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::UnknownEndpoint => write!(f, "unknown or closed endpoint"),
            SendError::NoOutputReport => write!(f, "the endpoint takes no output reports"),
            SendError::TooLong { len, max } => {
                write!(f, "report of {len} bytes exceeds the output length {max}")
            }
            SendError::Empty => write!(f, "empty report"),
            SendError::Stopped => write!(f, "transport stopped"),
        }
    }
}

impl std::error::Error for SendError {}

/// The exact-length output report for `bytes`: zero-padded to `output_len`.
pub(crate) fn pad_report(bytes: &[u8], output_len: usize) -> Result<Vec<u8>, SendError> {
    if output_len == 0 {
        return Err(SendError::NoOutputReport);
    }
    if bytes.is_empty() {
        return Err(SendError::Empty);
    }
    if bytes.len() > output_len {
        return Err(SendError::TooLong {
            len: bytes.len(),
            max: output_len,
        });
    }
    let mut report = Vec::with_capacity(output_len);
    report.extend_from_slice(bytes);
    report.resize(output_len, 0);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_a_short_request_to_the_long_report() {
        let r = pad_report(&[0x11, 0x01, 0x00, 0x1D, 0x00, 0x00, 0xAA], 20).unwrap();
        assert_eq!(r.len(), 20);
        assert_eq!(&r[..7], &[0x11, 0x01, 0x00, 0x1D, 0x00, 0x00, 0xAA]);
        assert!(r[7..].iter().all(|b| *b == 0));
    }

    #[test]
    fn an_exact_length_report_is_unchanged() {
        let frame = [0x10, 0x01, 0x00, 0x1D, 0x00, 0x00, 0xAA];
        assert_eq!(pad_report(&frame, 7).unwrap(), frame.to_vec());
    }

    #[test]
    fn refuses_a_report_longer_than_the_endpoint() {
        assert_eq!(
            pad_report(&[0u8; 8], 7),
            Err(SendError::TooLong { len: 8, max: 7 })
        );
    }

    #[test]
    fn refuses_an_endpoint_without_output() {
        assert_eq!(pad_report(&[0x10], 0), Err(SendError::NoOutputReport));
    }

    #[test]
    fn refuses_an_empty_report() {
        assert_eq!(pad_report(&[], 7), Err(SendError::Empty));
    }

    #[test]
    fn the_queue_keeps_order_and_empties_on_drain() {
        let q = ControlQueue::default();
        assert!(q.push(Control::Arrive("a".into())));
        assert!(!q.push(Control::Remove("b".into())));
        assert!(!q.push(Control::Send {
            id: EndpointId(3),
            report: vec![1, 2],
        }));
        assert!(!q.push(Control::Stop));
        assert_eq!(
            q.drain(),
            vec![
                Control::Arrive("a".into()),
                Control::Remove("b".into()),
                Control::Send {
                    id: EndpointId(3),
                    report: vec![1, 2],
                },
                Control::Stop,
            ]
        );
        assert!(q.drain().is_empty());
    }

    #[test]
    fn push_reports_empty_again_after_a_drain() {
        let q = ControlQueue::default();
        assert!(q.push(Control::Stop));
        q.drain();
        assert!(q.push(Control::Stop));
    }

    #[test]
    fn many_producers_lose_nothing() {
        use std::sync::Arc;
        let q = Arc::new(ControlQueue::default());
        let handles: Vec<_> = (0..4u32)
            .map(|t| {
                let q = q.clone();
                std::thread::spawn(move || {
                    for i in 0..100u32 {
                        q.push(Control::Send {
                            id: EndpointId(t),
                            report: vec![i as u8],
                        });
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let all = q.drain();
        assert_eq!(all.len(), 400);
        for t in 0..4u32 {
            let seq: Vec<u8> = all
                .iter()
                .filter_map(|c| match c {
                    Control::Send { id, report } if id.0 == t => Some(report[0]),
                    _ => None,
                })
                .collect();
            assert_eq!(seq, (0..100u32).map(|i| i as u8).collect::<Vec<_>>());
        }
    }
}
