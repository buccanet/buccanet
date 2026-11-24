//! HTTP client of the relay API v1. Bearer token in a header, no query strings.

use buccanet_protocol::API_PREFIX;
use buccanet_protocol::api::{
    Accepted, Approve, CreateRoom, DeviceRegistered, ErrorBody, InboxPage, InviteCreated, InvitePreview, JoinRequested, PendingRequest,
    PostMessage, RegisterDevice, RemoveMember, RequestJoin, RoomView, SetAdmins,
};
use reqwest::{RequestBuilder, Response};
use serde::de::DeserializeOwned;

use crate::error::SdkError;

#[derive(Clone)]
pub struct RelayClient {
    base: String,
    token: Option<String>,
    http: reqwest::Client,
}

fn http_client() -> reqwest::Client {
    #[cfg(not(target_arch = "wasm32"))]
    {
        reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)).build().expect("the HTTP client always builds")
    }
    #[cfg(target_arch = "wasm32")]
    {
        reqwest::Client::new()
    }
}

/// The token of an invite link (`<origin>/join/<token>`), or the token itself.
pub fn invite_token(link: &str) -> Result<String, SdkError> {
    let token = link.trim().rsplit("/join/").next().unwrap_or_default().trim_end_matches('/');
    let valid = !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| token.to_string()).ok_or_else(|| SdkError::Invalid("not a Buccanet invite link".into()))
}

/// A relay the device may send its token to: HTTPS anywhere, plain HTTP only on this machine
/// or a private network, where a development relay runs.
pub fn check_relay_url(base: &str) -> Result<(), SdkError> {
    let url = reqwest::Url::parse(base).map_err(|_| SdkError::Invalid(format!("{base} is not a relay URL")))?;
    let local = match url.host() {
        Some(url::Host::Domain(domain)) => domain == "localhost",
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_private(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unique_local(),
        None => false,
    };
    match url.scheme() {
        "https" => Ok(()),
        "http" if local => Ok(()),
        _ => Err(SdkError::Invalid(format!("{base} must use https: the device token would travel in clear"))),
    }
}

impl RelayClient {
    pub fn new(base: &str, token: Option<String>) -> Self {
        Self { base: base.trim_end_matches('/').to_string(), token, http: http_client() }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{API_PREFIX}{path}", self.base)
    }

    fn authorized(&self, request: RequestBuilder) -> RequestBuilder {
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    async fn checked(&self, request: RequestBuilder) -> Result<Response, SdkError> {
        let response = self.authorized(request).send().await?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let body: ErrorBody = response.json().await.map_err(|_| SdkError::Relay {
            status,
            code: "unreadable".into(),
            message: "The relay answered without a body.".into(),
        })?;
        Err(SdkError::Relay { status, code: body.error, message: body.message })
    }

    async fn json<R: DeserializeOwned>(&self, request: RequestBuilder) -> Result<R, SdkError> {
        Ok(self.checked(request).await?.json().await?)
    }

    pub async fn register(&self, body: &RegisterDevice) -> Result<DeviceRegistered, SdkError> {
        self.json(self.http.post(self.url("/devices")).json(body)).await
    }

    pub async fn create_room(&self, body: &CreateRoom) -> Result<RoomView, SdkError> {
        self.json(self.http.post(self.url("/rooms")).json(body)).await
    }

    pub async fn room(&self, room_id: &str) -> Result<RoomView, SdkError> {
        self.json(self.http.get(self.url(&format!("/rooms/{room_id}")))).await
    }

    pub async fn create_invite(&self, room_id: &str) -> Result<InviteCreated, SdkError> {
        self.json(self.http.post(self.url(&format!("/rooms/{room_id}/invites")))).await
    }

    pub async fn preview(&self, token: &str) -> Result<InvitePreview, SdkError> {
        self.json(self.http.get(self.url(&format!("/invites/{token}")))).await
    }

    pub async fn request_join(&self, token: &str, key_package: Vec<u8>) -> Result<JoinRequested, SdkError> {
        self.json(self.http.post(self.url(&format!("/invites/{token}/requests"))).json(&RequestJoin { key_package })).await
    }

    pub async fn pending(&self, room_id: &str) -> Result<Vec<PendingRequest>, SdkError> {
        self.json(self.http.get(self.url(&format!("/rooms/{room_id}/requests")))).await
    }

    pub async fn approve(&self, room_id: &str, request_id: &str, body: &Approve) -> Result<Accepted, SdkError> {
        self.json(self.http.post(self.url(&format!("/rooms/{room_id}/requests/{request_id}/approve"))).json(body)).await
    }

    pub async fn reject(&self, room_id: &str, request_id: &str) -> Result<(), SdkError> {
        self.checked(self.http.post(self.url(&format!("/rooms/{room_id}/requests/{request_id}/reject")))).await.map(|_| ())
    }

    pub async fn post_message(&self, room_id: &str, body: &PostMessage) -> Result<Accepted, SdkError> {
        self.json(self.http.post(self.url(&format!("/rooms/{room_id}/messages"))).json(body)).await
    }

    pub async fn remove(&self, room_id: &str, device_id: &str, body: &RemoveMember) -> Result<Accepted, SdkError> {
        self.json(self.http.post(self.url(&format!("/rooms/{room_id}/members/{device_id}/remove"))).json(body)).await
    }

    pub async fn set_admins(&self, room_id: &str, body: &SetAdmins) -> Result<Accepted, SdkError> {
        self.json(self.http.post(self.url(&format!("/rooms/{room_id}/admins"))).json(body)).await
    }

    pub async fn leave(&self, room_id: &str) -> Result<(), SdkError> {
        self.checked(self.http.post(self.url(&format!("/rooms/{room_id}/leave")))).await.map(|_| ())
    }

    pub async fn inbox_after(&self, after: u64) -> Result<InboxPage, SdkError> {
        self.json(self.http.get(self.url(&format!("/inbox/{after}")))).await
    }

    /// Opens the live inbox stream after `after`.
    pub async fn stream(&self, after: u64) -> Result<Response, SdkError> {
        let request = self.http.get(self.url("/stream")).header("accept", "text/event-stream").header("last-event-id", after.to_string());
        self.checked(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::{check_relay_url, invite_token};

    #[test]
    fn plain_http_only_reaches_local_relays() {
        assert!(check_relay_url("https://buccanet.laboetie.io").is_ok());
        assert!(check_relay_url("http://127.0.0.1:8758").is_ok());
        assert!(check_relay_url("http://10.0.2.2:8758").is_ok());
        assert!(check_relay_url("http://localhost:8758").is_ok());
        assert!(check_relay_url("http://relay.example.com").is_err());
        assert!(check_relay_url("ftp://127.0.0.1").is_err());
    }

    #[test]
    fn reads_tokens_from_links() {
        assert_eq!(invite_token("https://buccanet.chat/join/abc_D-9").unwrap(), "abc_D-9");
        assert_eq!(invite_token("http://localhost:8759/join/abc/").unwrap(), "abc");
        assert_eq!(invite_token("abc").unwrap(), "abc");
        assert!(invite_token("https://x/join/a b").is_err());
    }
}
