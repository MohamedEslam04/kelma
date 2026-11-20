//! The signed-in user's own profile photo, through Microsoft Graph.
//!
//! UNVERIFIED (docs/protocol.md): neither request has been sent against a real account. Graph
//! documents `PUT /me/photo/$value` (JPEG or PNG, at most 4 MB) and needs `User.ReadWrite`, which
//! the Graph `.default` token kelma holds may not carry; removing a photo is only documented on
//! the beta endpoint. A refusal comes back as an `Error::Status`, which the UI shows as it is.

use crate::broker::Audience;
use crate::client::{Client, RawBody};
use crate::error::{Error, Result};
use reqwest::Method;

/// Graph's limit for a profile photo.
pub const MAX_PHOTO_BYTES: usize = 4 * 1024 * 1024;

/// `PUT` the picture here.
pub const PHOTO_VALUE_URL: &str = "https://graph.microsoft.com/v1.0/me/photo/$value";
/// `DELETE` this one (beta) to remove the photo.
pub const PHOTO_BETA_URL: &str = "https://graph.microsoft.com/beta/me/photo";

/// The content type of a picture Graph takes (`image/jpeg`, `image/png`), from its first bytes.
pub fn photo_content_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else {
        None
    }
}

/// The content type to send, or why the picture cannot be sent (in words for the user).
pub fn check_photo(bytes: &[u8]) -> std::result::Result<&'static str, String> {
    let Some(kind) = photo_content_type(bytes) else {
        return Err("Pick a JPEG or PNG picture.".into());
    };
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err("The picture is larger than 4 MB. Pick a smaller one.".into());
    }
    Ok(kind)
}

impl Client {
    /// Replaces my profile photo with `bytes` (checked with [`check_photo`] first).
    pub async fn set_my_photo(&self, bytes: Vec<u8>) -> Result<()> {
        let kind = check_photo(&bytes).map_err(Error::Protocol)?;
        let r = self
            .request_raw_as(
                &Audience::Graph,
                Method::PUT,
                PHOTO_VALUE_URL,
                RawBody::Bytes(kind, bytes),
                &[],
            )
            .await?;
        if (200..300).contains(&r.status) {
            Ok(())
        } else {
            Err(Error::status(
                "set photo",
                r.status,
                &String::from_utf8_lossy(&r.body),
            ))
        }
    }

    /// Removes my profile photo.
    pub async fn remove_my_photo(&self) -> Result<()> {
        let r = self
            .request_raw_as(
                &Audience::Graph,
                Method::DELETE,
                PHOTO_BETA_URL,
                RawBody::None,
                &[],
            )
            .await?;
        if (200..300).contains(&r.status) {
            Ok(())
        } else {
            Err(Error::status(
                "remove photo",
                r.status,
                &String::from_utf8_lossy(&r.body),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_and_png_are_recognised_by_their_first_bytes() {
        assert_eq!(
            photo_content_type(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]),
            Some("image/jpeg")
        );
        assert_eq!(
            photo_content_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some("image/png")
        );
        assert_eq!(photo_content_type(b"GIF89a"), None);
        assert_eq!(photo_content_type(&[]), None);
    }

    #[test]
    fn a_photo_over_four_megabytes_or_of_another_type_is_refused_with_words() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF];
        assert_eq!(check_photo(&jpeg), Ok("image/jpeg"));
        jpeg.resize(MAX_PHOTO_BYTES + 1, 0);
        assert!(check_photo(&jpeg).unwrap_err().contains("4 MB"));
        assert!(check_photo(b"not a picture").unwrap_err().contains("JPEG"));
    }

    #[test]
    fn the_endpoints_are_graph_hosts() {
        for u in [PHOTO_VALUE_URL, PHOTO_BETA_URL] {
            assert!(u.starts_with("https://graph.microsoft.com/"));
        }
    }
}
