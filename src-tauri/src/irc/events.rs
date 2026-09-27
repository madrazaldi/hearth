//! Events sent to the frontend. They are coalesced into arrays so a burst of
//! traffic (NAMES, history, netsplits) costs one IPC round instead of hundreds.

use super::message::Message;
use serde::Serialize;
use std::collections::BTreeMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Connecting,
    Registering,
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", rename_all_fields = "camelCase")]
pub enum Event {
    ConnAdded {
        conn: String,
        server: String,
        parent: Option<String>,
        netid: Option<String>,
    },
    ConnRemoved {
        conn: String,
    },
    Status {
        conn: String,
        status: Status,
        error: Option<String>,
        retry_in: Option<u64>,
    },
    Registered {
        conn: String,
        nick: String,
        caps: Vec<String>,
    },
    Caps {
        conn: String,
        caps: Vec<String>,
    },
    Isupport {
        conn: String,
        isupport: BTreeMap<String, String>,
    },
    Message {
        conn: String,
        msg: Message,
    },
}

#[derive(Clone)]
pub struct Emitter(mpsc::UnboundedSender<Event>);

impl Emitter {
    /// `sink` receives each coalesced batch of events.
    pub fn spawn(sink: impl Fn(&[Event]) + Send + 'static) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
        tauri::async_runtime::spawn(async move {
            let mut batch = Vec::with_capacity(64);
            while let Some(first) = rx.recv().await {
                batch.push(first);
                // Let a burst accumulate briefly so it crosses IPC as one payload.
                tokio::time::sleep(std::time::Duration::from_millis(8)).await;
                while batch.len() < 2000 {
                    match rx.try_recv() {
                        Ok(ev) => batch.push(ev),
                        Err(_) => break,
                    }
                }
                sink(&batch);
                batch.clear();
            }
        });
        Emitter(tx)
    }

    pub fn emit(&self, ev: Event) {
        let _ = self.0.send(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc as std_mpsc;
    use std::time::Duration;

    fn status(i: u64) -> Event {
        Event::Status { conn: "c".into(), status: Status::Connecting, error: None, retry_in: Some(i) }
    }

    fn recorder() -> (Emitter, std_mpsc::Receiver<usize>) {
        let (tx, rx) = std_mpsc::channel();
        let emitter = Emitter::spawn(move |evs| tx.send(evs.len()).unwrap());
        (emitter, rx)
    }

    #[test]
    fn a_burst_crosses_as_one_batch() {
        let (emitter, rx) = recorder();
        for i in 0..100 {
            emitter.emit(status(i));
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 100);
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn batches_are_capped() {
        let (emitter, rx) = recorder();
        for i in 0..2500 {
            emitter.emit(status(i));
        }
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2000);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 500);
    }

    #[test]
    fn serializes_as_the_frontend_expects() {
        let v = serde_json::to_value(Event::Status {
            conn: "c".into(),
            status: Status::Disconnected,
            error: Some("boom".into()),
            retry_in: Some(4),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type": "status", "conn": "c", "status": "disconnected", "error": "boom", "retryIn": 4})
        );
        let v = serde_json::to_value(Event::ConnAdded {
            conn: "s/1".into(),
            server: "s".into(),
            parent: Some("s".into()),
            netid: Some("1".into()),
        })
        .unwrap();
        assert_eq!(v["type"], "conn-added");
        assert_eq!(v["netid"], "1");
    }
}
