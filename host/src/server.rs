use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, sleep, timeout, timeout_at};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{WebSocketStream, accept_hdr_async_with_config};

use crate::auth;
use crate::protocol::{ClientFrame, HostFrame, PROTO, code};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

type Ws = WebSocketStream<TcpStream>;

pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))
}

pub async fn serve(listener: TcpListener, token: String) {
    let token: Arc<str> = token.into();
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(serve_connection(stream, Arc::clone(&token)));
            }
            // Running out of file descriptors fails here repeatedly, so do not spin on it.
            Err(error) => {
                eprintln!("term-host: accept failed: {error}");
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

// The Callback trait in tungstenite fixes the return type, so the error cannot be boxed.
#[allow(clippy::result_large_err)]
async fn serve_connection(stream: TcpStream, token: Arc<str>) {
    // A delayed small packet would add typing latency.
    if stream.set_nodelay(true).is_err() {
        return;
    }

    // The deadline covers the upgrade as well, so a client that never upgrades cannot hold
    // a connection open either.
    let deadline = Instant::now() + HELLO_TIMEOUT;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let upgrade = accept_hdr_async_with_config(
        stream,
        |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            if origin_passes(request) {
                Ok(response)
            } else {
                let mut refusal = ErrorResponse::new(None);
                *refusal.status_mut() = StatusCode::FORBIDDEN;
                Err(refusal)
            }
        },
        Some(config),
    );
    let Ok(Ok(mut ws)) = timeout_at(deadline, upgrade).await else {
        return;
    };

    let first = match timeout_at(deadline, ws.next()).await {
        Ok(Some(Ok(frame))) => Some(frame),
        _ => None,
    };
    match judge_hello(first, &token) {
        Hello::Accepted => {}
        Hello::WrongProto => {
            let message = format!("this host speaks protocol {PROTO}");
            let refusal = HostFrame::error(code::PROTO_MISMATCH, &message);
            let _ = timeout(CLOSE_TIMEOUT, send_frame(&mut ws, &refusal)).await;
            close(&mut ws).await;
            return;
        }
        // Says nothing about why, so a caller cannot tell a wrong token from a bad frame.
        Hello::Refused => {
            close(&mut ws).await;
            return;
        }
    }

    let welcome = HostFrame::HelloOk {
        proto: PROTO,
        host_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if send_frame(&mut ws, &welcome).await.is_ok() {
        answer_frames(&mut ws).await;
    }
}

/// A missing Origin is allowed: browsers always send one on a WebSocket, and the token still
/// guards a non-browser client. More than one is never sent honestly, so it is refused.
fn origin_passes(request: &Request) -> bool {
    let mut origins = request.headers().get_all(ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(origin), None) => origin.to_str().is_ok_and(auth::origin_allowed),
        (Some(_), Some(_)) => false,
    }
}

enum Hello {
    Accepted,
    WrongProto,
    Refused,
}

/// The token is checked before the version, so only a caller holding the token learns
/// anything from the reply.
fn judge_hello(first: Option<Message>, token: &str) -> Hello {
    let Some(Message::Text(text)) = first else {
        return Hello::Refused;
    };
    let Ok(ClientFrame::Hello {
        token: given,
        proto,
    }) = serde_json::from_str(text.as_str())
    else {
        return Hello::Refused;
    };
    if !auth::tokens_match(&given, token) {
        Hello::Refused
    } else if proto != PROTO {
        Hello::WrongProto
    } else {
        Hello::Accepted
    }
}

async fn answer_frames(ws: &mut Ws) {
    while let Some(frame) = ws.next().await {
        let reply = match frame {
            Ok(Message::Text(text)) => answer_text(text.as_str()),
            Ok(Message::Binary(_)) => not_implemented(),
            Ok(Message::Close(_)) | Err(_) => break,
            // tungstenite answers pings itself.
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
        };
        if send_frame(ws, &reply).await.is_err() {
            break;
        }
    }
}

fn answer_text(text: &str) -> HostFrame {
    match serde_json::from_str(text) {
        Ok(ClientFrame::Hello { .. }) => HostFrame::error(code::BAD_REQUEST, "already said hello"),
        Ok(_) => not_implemented(),
        Err(_) => HostFrame::error(code::BAD_REQUEST, "not a frame this host understands"),
    }
}

fn not_implemented() -> HostFrame {
    HostFrame::error(code::NOT_IMPLEMENTED, "sessions are not served yet")
}

async fn send_frame(ws: &mut Ws, frame: &HostFrame) -> Result<()> {
    let text = serde_json::to_string(frame)?;
    ws.send(Message::text(text)).await?;
    Ok(())
}

async fn close(ws: &mut Ws) {
    let _ = timeout(CLOSE_TIMEOUT, ws.close(None)).await;
}
