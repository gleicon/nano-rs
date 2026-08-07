//! WebSocket upgrade and relay functions for the HTTP router
//!
//! Handles WebSocket connection upgrading and bidirectional relay
//! between axum WebSocket connections and the worker runtime.

use crate::http::router::{AppState, HandlerType};
use crate::http::NanoRequest;
use crate::worker::{HandlerTask, QueueError, WsChannels};
use axum::{
    body::Body,
    extract::ws::{Message as AxumWsMessage, WebSocket, WebSocketUpgrade},
    http::{Request, Response, StatusCode},
    response::IntoResponse,
};
use std::sync::Arc;
use uuid::Uuid;

/// Perform the WebSocket upgrade handshake per RFC 6455 §4.2.2.
pub(crate) async fn handle_ws_upgrade(
    state: Arc<AppState>,
    request: Request<Body>,
    host: String,
) -> Response<Body> {
    use axum::extract::FromRequestParts;

    let target = state.router.read().await.resolve(&host).clone();
    let entrypoint = match &target.handler_type {
        HandlerType::WinterTCHandler(path) => path.clone(),
        HandlerType::WinterTCSliverHandler {
            entrypoint: path, ..
        } => path.clone(),
        _ => {
            return Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header("content-type", "text/plain")
                .body(Body::from("WebSocket not supported for static handlers"))
                .unwrap();
        }
    };

    let (mut parts, _body) = request.into_parts();
    let method = parts.method.clone();
    let uri = parts.uri.clone();
    let headers_clone = parts.headers.clone();

    let ws_upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => ws,
        Err(rejection) => {
            tracing::warn!(
                "WS upgrade extraction failed for host {:?}: {:?}",
                host,
                rejection
            );
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("content-type", "text/plain")
                .body(Body::from("WebSocket upgrade failed"))
                .unwrap();
        }
    };

    let nano_request =
        match NanoRequest::from_axum_parts(&method, &uri, &host, &headers_clone, None) {
            Ok(r) => r,
            Err(_) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .header("content-type", "text/plain")
                    .body(Body::from("Bad Request"))
                    .unwrap();
            }
        };

    let (inbound_tx, inbound_rx) = std::sync::mpsc::sync_channel::<tungstenite::Message>(128);
    let (outbound_tx, outbound_rx) = std::sync::mpsc::sync_channel::<tungstenite::Message>(128);

    let (response_tx, _response_rx) = tokio::sync::oneshot::channel();
    let cpu_time_limit_ms = state.get_cpu_time_limit_ms(&host);
    let request_id = format!("ws_{}", &Uuid::new_v4().to_string()[..8]);
    let ws_channels = WsChannels {
        inbound_rx,
        outbound_tx,
    };
    let task = HandlerTask {
        entrypoint,
        request: nano_request,
        response_tx,
        hostname: host.clone(),
        start_time: std::time::Instant::now(),
        cpu_time_limit_ms,
        request_id,
        memory_limit_mb: 0,
        ws: Some(ws_channels),
    };

    {
        let mut queue = state.work_queue.lock().await;
        match queue.dispatch(&host, task).await {
            Ok(()) => {}
            Err(QueueError::ChannelFull) => {
                tracing::warn!("WS connection limit reached for hostname: {:?}", host);
                return Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .header("Retry-After", "1")
                    .header("content-type", "text/plain")
                    .body(Body::from("WebSocket connection limit reached"))
                    .unwrap();
            }
            Err(e) => {
                tracing::error!("WS dispatch error for host {:?}: {}", host, e);
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header("content-type", "text/plain")
                    .body(Body::from("Internal Server Error"))
                    .unwrap();
            }
        }
    }

    tracing::debug!("WebSocket upgrade accepted for host: {}", host);
    ws_upgrade
        .on_upgrade(move |socket| ws_relay_task(socket, inbound_tx, outbound_rx))
        .into_response()
}

const MAX_WS_MESSAGE_BYTES: usize = 32 * 1024 * 1024;

async fn ws_relay_task(
    mut socket: WebSocket,
    inbound_tx: std::sync::mpsc::SyncSender<tungstenite::Message>,
    outbound_rx: std::sync::mpsc::Receiver<tungstenite::Message>,
) {
    let (outbound_notify_tx, mut outbound_notify_rx) =
        tokio::sync::mpsc::channel::<tungstenite::Message>(128);
    tokio::task::spawn_blocking(move || {
        while let Ok(msg) = outbound_rx.recv() {
            if outbound_notify_tx.blocking_send(msg).is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            maybe_msg = socket.recv() => {
                match maybe_msg {
                    Some(Ok(axum_msg)) => {
                        let payload_len = match &axum_msg {
                            AxumWsMessage::Text(t) => t.len(),
                            AxumWsMessage::Binary(b) => b.len(),
                            _ => 0,
                        };
                        if payload_len > MAX_WS_MESSAGE_BYTES {
                            tracing::warn!(
                                "WS message too large ({} bytes), closing 1009",
                                payload_len
                            );
                            let _ = socket
                                .send(AxumWsMessage::Close(Some(
                                    axum::extract::ws::CloseFrame {
                                        code: 1009,
                                        reason: "Message too large".into(),
                                    },
                                )))
                                .await;
                            break;
                        }
                        match axum_to_tungstenite(axum_msg) {
                            Some(m) => {
                                match inbound_tx.try_send(m) {
                                    Ok(()) => {}
                                    Err(std::sync::mpsc::TrySendError::Full(_)) => {
                                        let _ = socket
                                            .send(AxumWsMessage::Close(Some(
                                                axum::extract::ws::CloseFrame {
                                                    code: 1008,
                                                    reason: "Backpressure limit".into(),
                                                },
                                            )))
                                            .await;
                                        break;
                                    }
                                    Err(std::sync::mpsc::TrySendError::Disconnected(_)) => break,
                                }
                            }
                            None => continue,
                        }
                    }
                    Some(Err(e)) => {
                        tracing::debug!("WS recv error: {}", e);
                        break;
                    }
                    None => break,
                }
            }
            maybe_out = outbound_notify_rx.recv() => {
                match maybe_out {
                    Some(tung_msg) => {
                        if socket.send(tungstenite_to_axum(tung_msg)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

fn axum_to_tungstenite(msg: AxumWsMessage) -> Option<tungstenite::Message> {
    match msg {
        AxumWsMessage::Text(utf8) => Some(tungstenite::Message::Text(utf8.as_str().to_string())),
        AxumWsMessage::Binary(bytes) => Some(tungstenite::Message::Binary(bytes.to_vec())),
        AxumWsMessage::Close(Some(frame)) => Some(tungstenite::Message::Close(Some(
            tungstenite::protocol::CloseFrame {
                code: (frame.code as u16).into(),
                reason: std::borrow::Cow::Owned(frame.reason.as_str().to_string()),
            },
        ))),
        AxumWsMessage::Close(None) => Some(tungstenite::Message::Close(None)),
        AxumWsMessage::Ping(_) | AxumWsMessage::Pong(_) => None,
    }
}

fn tungstenite_to_axum(msg: tungstenite::Message) -> AxumWsMessage {
    match msg {
        tungstenite::Message::Text(s) => AxumWsMessage::Text(s.into()),
        tungstenite::Message::Binary(b) => AxumWsMessage::Binary(bytes::Bytes::from(b)),
        tungstenite::Message::Close(Some(frame)) => {
            AxumWsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: u16::from(frame.code),
                reason: frame.reason.as_ref().into(),
            }))
        }
        tungstenite::Message::Close(None) => AxumWsMessage::Close(None),
        tungstenite::Message::Ping(p) => AxumWsMessage::Ping(bytes::Bytes::from(p)),
        tungstenite::Message::Pong(p) => AxumWsMessage::Pong(bytes::Bytes::from(p)),
        tungstenite::Message::Frame(_) => AxumWsMessage::Close(None),
    }
}
