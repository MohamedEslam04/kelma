//! A minimal Chrome DevTools Protocol client over one WebSocket (flattened sessions).

use crate::Error;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

/// A CDP event. `session_id` is set for events of an attached target.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Reply = oneshot::Sender<Result<Value, Error>>;

#[derive(Default)]
struct Pending {
    closed: bool,
    calls: HashMap<u64, (String, Reply)>,
}

/// A connection to the browser's DevTools endpoint. Cheap to share behind an `Arc`.
pub struct Cdp {
    out: mpsc::UnboundedSender<String>,
    pending: Arc<Mutex<Pending>>,
    next_id: AtomicU64,
}

impl Cdp {
    /// Connects to `ws_url`. Events arrive on the returned receiver, which ends when the
    /// connection closes.
    pub async fn connect(ws_url: &str) -> Result<(Cdp, mpsc::UnboundedReceiver<Event>), Error> {
        let (ws, _) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| Error::Ws(Box::new(e)))?;
        let (mut sink, mut stream) = ws.split();
        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        let (events, events_rx) = mpsc::unbounded_channel();
        let pending = Arc::new(Mutex::new(Pending::default()));

        tokio::spawn(async move {
            while let Some(text) = out_rx.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let reader_pending = pending.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = v["id"].as_u64() {
                    let call = reader_pending.lock().unwrap().calls.remove(&id);
                    if let Some((method, reply)) = call {
                        let result = match v.get("error") {
                            Some(e) => Err(Error::Cdp {
                                method,
                                message: e["message"].as_str().unwrap_or("").to_string(),
                            }),
                            None => Ok(v["result"].clone()),
                        };
                        let _ = reply.send(result);
                    }
                } else if let Some(method) = v["method"].as_str() {
                    let _ = events.send(Event {
                        method: method.to_string(),
                        params: v["params"].clone(),
                        session_id: v["sessionId"].as_str().map(str::to_string),
                    });
                }
            }
            let mut p = reader_pending.lock().unwrap();
            p.closed = true;
            p.calls.clear();
        });

        Ok((
            Cdp {
                out,
                pending,
                next_id: AtomicU64::new(1),
            },
            events_rx,
        ))
    }

    /// Sends a command (to the browser, or to the target of `session`) and waits for its result.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, Error> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut p = self.pending.lock().unwrap();
            if p.closed {
                return Err(Error::Closed);
            }
            p.calls.insert(id, (method.to_string(), tx));
        }
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.out.send(msg.to_string()).map_err(|_| Error::Closed)?;
        rx.await.map_err(|_| Error::Closed)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake DevTools endpoint: answers `Echo` with its params, fails `Fail`, emits one event
    /// after the first call and closes after `Bye`.
    async fn fake_devtools() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            let mut sent_event = false;
            while let Some(Ok(Message::Text(t))) = ws.next().await {
                let req: Value = serde_json::from_str(&t).unwrap();
                let id = req["id"].clone();
                let reply = match req["method"].as_str().unwrap() {
                    "Echo" => {
                        json!({"id": id, "result": {"params": req["params"], "session": req["sessionId"]}})
                    }
                    "Fail" => {
                        json!({"id": id, "error": {"code": -32000, "message": "no such target"}})
                    }
                    _ => break,
                };
                ws.send(Message::text(reply.to_string())).await.unwrap();
                if !sent_event {
                    sent_event = true;
                    let ev = json!({"method": "Target.targetDestroyed", "params": {"targetId": "T1"}, "sessionId": "S1"});
                    ws.send(Message::text(ev.to_string())).await.unwrap();
                }
            }
        });
        format!("ws://{addr}/devtools/browser/x")
    }

    #[tokio::test]
    async fn calls_get_their_results_and_events_are_forwarded() {
        let url = fake_devtools().await;
        let (cdp, mut events) = Cdp::connect(&url).await.unwrap();

        let r = cdp.call("Echo", json!({"a": 1}), Some("S1")).await.unwrap();
        assert_eq!(r, json!({"params": {"a": 1}, "session": "S1"}));

        let ev = events.recv().await.unwrap();
        assert_eq!(
            ev,
            Event {
                method: "Target.targetDestroyed".into(),
                params: json!({"targetId": "T1"}),
                session_id: Some("S1".into()),
            }
        );

        let r = cdp.call("Echo", json!({}), None).await.unwrap();
        assert_eq!(
            r["session"],
            Value::Null,
            "no sessionId sent for browser calls"
        );

        match cdp.call("Fail", json!({}), None).await {
            Err(Error::Cdp { method, message }) => {
                assert_eq!(method, "Fail");
                assert_eq!(message, "no such target");
            }
            other => panic!("expected a CDP error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_closed_connection_ends_events_and_fails_calls() {
        let url = fake_devtools().await;
        let (cdp, mut events) = Cdp::connect(&url).await.unwrap();
        assert!(matches!(
            cdp.call("Bye", json!({}), None).await,
            Err(Error::Closed)
        ));
        assert_eq!(events.recv().await, None);
        assert!(matches!(
            cdp.call("Echo", json!({}), None).await,
            Err(Error::Closed)
        ));
    }
}
