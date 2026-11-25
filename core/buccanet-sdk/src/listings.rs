//! Public listings: a room admin lists a room under a slug, and a device that knows the slug
//! asks to join without an invite. Every join still waits for a room admin.

use buccanet_protocol::api::{ListingView, SetListing};
use buccanet_protocol::listing::{check_text, valid_slug};

use crate::client::{Client, JoinPending};
use crate::error::SdkError;
use crate::events::RoomInfo;

fn checked_slug(slug: &str) -> Result<&str, SdkError> {
    if valid_slug(slug) {
        return Ok(slug);
    }
    Err(SdkError::Invalid(format!("{slug} is not a listing slug: use lowercase letters, digits and hyphens")))
}

impl Client {
    /// Lists a room this device administers under `slug`, or moves the slug to it. The relay
    /// accepts it from the devices its operator allows.
    pub async fn set_listing(&self, room_id: &str, slug: &str, title: &str, description: &str) -> Result<RoomInfo, SdkError> {
        check_text(title, description).map_err(SdkError::Invalid)?;
        if !self.inner.lock().await.room(room_id)?.admin {
            return Err(SdkError::Invalid("only a room admin can list it".into()));
        }
        let body = SetListing {
            slug: checked_slug(slug)?.to_string(),
            title: title.trim().to_string(),
            description: description.trim().to_string(),
        };
        self.relay.set_listing(room_id, &body).await?;
        self.refresh_room(room_id).await
    }

    /// What the relay shows of a listed room.
    pub async fn listing(&self, slug: &str) -> Result<ListingView, SdkError> {
        self.relay.listing(checked_slug(slug)?).await
    }

    /// Asks to join the room listed under `slug`; the Welcome arrives once a room admin approves.
    pub async fn join_listing(&self, slug: &str) -> Result<JoinPending, SdkError> {
        let slug = checked_slug(slug)?;
        self.request_join(|key_package| self.relay.request_listing_join(slug, key_package)).await
    }
}
