//! A localhost bridge for gateway harnesses that bring their own chat loop (Hermes): this
//! process owns the profile, streams incoming messages and sends what the harness answers.

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use buccanet_sdk::{Client, ClientEvent};
use futures::Stream;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::host::Events;

#[derive(Clone)]
struct Bridge {
    client: Client,
    token: Arc<str>,
    feed: broadcast::Sender<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendBody {
    room_id: String,
    text: String,
}

#[derive(Deserialize)]
struct JoinBody {
    invite: String,
}

/// Compares every byte whatever the first difference, so the time taken says nothing about
/// how much of the token a guess got right.
fn same_secret(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len() && given.iter().zip(expected).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

fn authorized(bridge: &Bridge, headers: &HeaderMap) -> Result<(), StatusCode> {
    let expected = format!("Bearer {}", bridge.token);
    let given = headers.get("authorization").map(|value| value.as_bytes()).unwrap_or_default();
    same_secret(given, expected.as_bytes()).then_some(()).ok_or(StatusCode::UNAUTHORIZED)
}

pub async fn serve(client: Client, mut events: Events, port: u16, token: String) -> std::io::Result<()> {
    let (feed, _) = broadcast::channel(256);
    let bridge = Bridge { client: client.clone(), token: Arc::from(token), feed: feed.clone() };
    tokio::spawn(async move { client.run().await });
    let relay_client = bridge.client.clone();
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Some(value) = feed_item(&relay_client, event).await {
                let _ = feed.send(value);
            }
        }
    });
    let app = Router::new()
        .route("/events", get(stream))
        .route("/rooms", get(rooms))
        .route("/send", post(send))
        .route("/join", post(join))
        .with_state(bridge);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!("buccanet bridge on http://127.0.0.1:{}", listener.local_addr()?.port());
    axum::serve(listener, app).await
}

async fn feed_item(client: &Client, event: ClientEvent) -> Option<Value> {
    match event {
        ClientEvent::Message { message } if !message.own && !message.shared => {
            let title = client.room(&message.room_id).await.ok().and_then(|room| room.title);
            Some(json!({
                "type": "message", "roomId": message.room_id, "roomTitle": title, "seq": message.seq,
                "from": { "deviceId": message.sender_device_id, "name": message.author_name, "kind": message.author_kind },
                "text": message.text,
            }))
        }
        ClientEvent::JoinedRoom { room } => Some(json!({ "type": "joined", "roomId": room.room_id, "roomTitle": room.title })),
        _ => None,
    }
}

async fn stream(
    State(bridge): State<Bridge>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    authorized(&bridge, &headers)?;
    let mut feed = bridge.feed.subscribe();
    let events = async_stream::stream! {
        while let Ok(item) = feed.recv().await {
            yield Ok(Event::default().data(item.to_string()));
        }
    };
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

async fn rooms(State(bridge): State<Bridge>, headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    authorized(&bridge, &headers)?;
    Ok(Json(json!({ "rooms": bridge.client.rooms().await })))
}

async fn send(State(bridge): State<Bridge>, headers: HeaderMap, Json(body): Json<SendBody>) -> Result<Json<Value>, (StatusCode, String)> {
    authorized(&bridge, &headers).map_err(|status| (status, "unauthorized".into()))?;
    let message = bridge.client.send_text(&body.room_id, &body.text).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    Ok(Json(json!({ "seq": message.seq })))
}

async fn join(State(bridge): State<Bridge>, headers: HeaderMap, Json(body): Json<JoinBody>) -> Result<Json<Value>, (StatusCode, String)> {
    authorized(&bridge, &headers).map_err(|status| (status, "unauthorized".into()))?;
    let pending = bridge.client.join(&body.invite).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    Ok(Json(json!({ "roomId": pending.room_id, "requestId": pending.request_id })))
}
