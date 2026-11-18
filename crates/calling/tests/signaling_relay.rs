//! TURN credentials from `trap/tokens` (offline).

use calling::relay::{DEFAULT_RELAY_HOST, TOKENS_URL, ice_servers, parse_tokens, relay_headers};
use std::time::Duration;

#[test]
fn parses_the_tokens_fixture_into_ice_servers() {
    let grant = parse_tokens(include_str!("fixtures/signaling/trap_tokens.json")).unwrap();
    assert_eq!(grant.expires_in, Duration::from_secs(604_800));
    assert_eq!(grant.credentials.len(), 1);
    let servers = ice_servers(&grant, "gateway-xx.example.invalid");
    assert_eq!(servers.len(), 1);
    let s = &servers[0];
    assert_eq!(
        s.urls,
        vec![
            "turn:gateway-xx.example.invalid:3478?transport=udp".to_string(),
            "turn:gateway-xx.example.invalid:443?transport=tcp".to_string(),
            "turns:gateway-xx.example.invalid:443".to_string(),
        ]
    );
    assert_eq!(s.username.as_deref(), Some("relay-user-placeholder"));
    assert_eq!(s.credential.as_deref(), Some("relay-pass-placeholder"));
}

#[test]
fn one_server_per_credential_and_errors() {
    let grant = parse_tokens(
        r#"{"tokens":[{"username":"a","password":"b"},{"realm":"r","username":"c","password":"d"}]}"#,
    )
    .unwrap();
    assert_eq!(grant.expires_in, Duration::ZERO);
    assert_eq!(ice_servers(&grant, DEFAULT_RELAY_HOST).len(), 2);
    assert!(parse_tokens(r#"{"tokens":[]}"#).is_err());
    assert!(parse_tokens("<html>").is_err());
}

#[test]
fn credentials_never_show_in_debug() {
    let grant = parse_tokens(include_str!("fixtures/signaling/trap_tokens.json")).unwrap();
    let text = format!("{grant:?}");
    assert!(!text.contains("relay-user-placeholder") && !text.contains("relay-pass-placeholder"));
}

#[test]
fn request_shape() {
    assert_eq!(TOKENS_URL, "https://teams.microsoft.com/trap/tokens");
    let h = relay_headers("IC3TOKEN");
    let get = |k: &str| {
        h.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(get("api-version"), Some("2"));
    assert_eq!(get("X-MS-Migration"), Some("True"));
    assert_eq!(get("Authorization"), Some("Bearer IC3TOKEN"));
    assert_eq!(get("X-Skypetoken"), None);
}
