//! Scrubbing of secrets, identities and ids from text before it goes into a report bundle.
//!
//! The rules favour over-redaction: a bundle that hides too much is a nuisance, one that leaks
//! a token is an incident. Redaction is idempotent, so a file can safely be passed through twice.

use regex::Regex;
use std::sync::LazyLock;

/// Header lines whose whole value is secret.
const HEADER_LINES: &str =
    r"(?im)\b(authorization|proxy-authorization|(?:set-)?cookie)[ \t]*:[^\n]*";

/// Token-like keys, as `key=value`, `key: value` or `"key":"value"` (also JSON-escaped).
const SECRET_KEYS: &str = r"access_token|refresh_token|id_token|skype_?token(?:_asm)?|x-skypetoken|token|client_secret|client_assertion|assertion|password|passwd|secret|device_code|user_code|sig";

/// JSON fields that carry names or message content.
const PERSONAL_KEYS: &str = r"displayname|imdisplayname|givenname|surname|mail|userprincipalname|upn|content|subject|topic|preview";

struct Rule {
    re: Regex,
    to: &'static str,
}

fn rule(pattern: &str, to: &'static str) -> Rule {
    Rule {
        re: Regex::new(pattern).expect("valid redaction regex"),
        to,
    }
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(HEADER_LINES, "$1: <redacted>"),
        rule(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]+", "Bearer <redacted>"),
        rule(
            r"\beyJ[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]{2,}\.[A-Za-z0-9_-]*",
            "<jwt>",
        ),
        rule(
            &format!(r#"(?i)(\\?"?\b(?:{SECRET_KEYS})\b\\?"?\s*[:=]\s*\\?"?)[^\s"&,;}}\\]+"#),
            "${1}<redacted>",
        ),
        rule(
            &format!(r#"(?i)("(?:{PERSONAL_KEYS})"\s*:\s*")(?:[^"\\]|\\.)*"#),
            "${1}<redacted>",
        ),
        rule(
            &format!(r#"(?i)(\\"(?:{PERSONAL_KEYS})\\"\s*:\s*\\")(?:[^"\\]|\\[^"])*"#),
            "${1}<redacted>",
        ),
        // Conversation ids, also URL-encoded: 19:<id>@thread.v2, 19:<guid>_<guid>@unq.gbl.spaces.
        rule(
            r#"(?i)\b19(?::|%3a)[^\s"'<>,;\\/]+?(?:@|%40)(?:thread|unq)\.[A-Za-z0-9.]+"#,
            "19:<conversation>",
        ),
        // MRIs such as 8:orgid:<guid>.
        rule(
            r#"(?i)\b(\d{1,2}):(orgid|live|teamsvisitor|gid|bot|msteams|skype|cid):[^\s"'<>,;\\)\]]+"#,
            "$1:$2:<redacted>",
        ),
        rule(r"\b4:\+?\d{6,}", "4:<phone>"),
        rule(
            r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}",
            "<email>",
        ),
        rule(
            r"(?i)\b[a-z0-9][a-z0-9-]*\.onmicrosoft\.com\b",
            "<tenant>.onmicrosoft.com",
        ),
        rule(
            r"(?i)\b[a-z0-9][a-z0-9-]*?(?:-my)?\.sharepoint\.com\b",
            "<tenant>.sharepoint.com",
        ),
        rule(r#"/home/[^/\s"']+"#, "/home/<user>"),
        rule(
            r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}",
            "<guid>",
        ),
        // Any long opaque run is most likely a token we did not recognise by name.
        rule(r"[A-Za-z0-9_+-]{80,}", "<blob>"),
    ]
});

/// Returns `text` with tokens, cookies, emails, MRIs, conversation ids, GUIDs (which includes
/// tenant ids), tenant domains, names and message content replaced by placeholders.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for r in RULES.iter() {
        out = r.re.replace_all(&out, r.to).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "0b1c2d3e-4f50-6172-8394-a5b6c7d8e9f0";
    const GUID2: &str = "FFEEDDCC-BBAA-9988-7766-554433221100";

    fn gone(input: &str, secret: &str) {
        let out = redact(input);
        assert!(!out.contains(secret), "{secret:?} survived in {out:?}");
    }

    #[test]
    fn bearer_token() {
        let out = redact("GET /v1/x Authorization: Bearer abc.def-ghi_jkl/mno+pq==");
        assert!(!out.contains("abc"), "{out}");
        let out = redact("header bearer ZmFrZXRva2Vu next");
        assert!(!out.contains("ZmFrZXRva2Vu"), "{out}");
        assert!(out.contains("next"));
    }

    #[test]
    fn authorization_header_line_is_dropped_to_end_of_line() {
        let out = redact("authorization: Basic dXNlcjpwYXNz\nkept line");
        assert!(!out.contains("dXNlcjpwYXNz"));
        assert!(out.contains("kept line"));
    }

    #[test]
    fn cookies() {
        let out = redact("Cookie: sid=AAA111; other=BBB222\nSet-Cookie: x=CCC333; Path=/\nok");
        for s in ["AAA111", "BBB222", "CCC333"] {
            assert!(!out.contains(s), "{out}");
        }
        assert!(out.contains("ok"));
    }

    #[test]
    fn skypetoken_in_every_shape() {
        gone("skypetoken=abcDEF123456", "abcDEF123456");
        gone("X-Skypetoken: abcDEF123456", "abcDEF123456");
        gone(r#"{"skypeToken":"abcDEF123456","x":1}"#, "abcDEF123456");
        gone("url?skypetoken_asm=abcDEF123456&b=2", "abcDEF123456");
        let out = redact(r#"{"skypetoken":"abcDEF123456","x":1}"#);
        assert!(out.contains("\"x\":1"), "{out}");
    }

    #[test]
    fn refresh_access_id_tokens() {
        gone(
            r#"{"refresh_token":"0.AAAAkeepSecret-_123","access_token":"zzzSECRETzzz"}"#,
            "keepSecret",
        );
        gone(
            "refresh_token=0.AAAAkeepSecret&grant_type=refresh_token",
            "keepSecret",
        );
        gone(r#"{"access_token":"zzzSECRETzzz"}"#, "zzzSECRETzzz");
        gone(r#"{"id_token": "idSecret99"}"#, "idSecret99");
        // escaped JSON inside a log line
        gone(
            r#"body="{\"refresh_token\":\"0.escapedSecret\"}""#,
            "escapedSecret",
        );
        gone(
            "device_code=DEVSECRET1234 user_code=AB12CD",
            "DEVSECRET1234",
        );
        gone("client_secret=shh-secret", "shh-secret");
    }

    #[test]
    fn grant_type_value_is_not_eaten() {
        let out = redact("grant_type=refresh_token&scope=x");
        assert_eq!(out, "grant_type=refresh_token&scope=x");
    }

    #[test]
    fn jwt() {
        let jwt = "eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.c2lnbmF0dXJl";
        let out = redact(&format!("token was {jwt} end"));
        assert!(!out.contains("eyJ"), "{out}");
        assert!(out.contains("end"));
    }

    #[test]
    fn long_opaque_blob() {
        let blob = "A".repeat(120);
        gone(&format!("value {blob}"), &blob);
    }

    #[test]
    fn emails() {
        for e in [
            "jane.doe@contoso.com",
            "j+tag@sub.example.org",
            "A_B-C@x.co",
            "first.last@contoso.onmicrosoft.com",
        ] {
            let out = redact(&format!("from {e} to"));
            assert!(!out.contains('@'), "{out}");
            assert!(out.contains("<email>"));
        }
    }

    #[test]
    fn orgid_mri() {
        let out = redact(&format!("self 8:orgid:{GUID} sent"));
        assert!(!out.contains(GUID), "{out}");
        assert!(out.contains("8:orgid:<redacted>"), "{out}");
        gone(&format!("8:ORGID:{GUID2}"), GUID2);
    }

    #[test]
    fn other_mris() {
        gone("8:live:some.person_1", "some.person_1");
        gone("8:teamsvisitor:abc123", "abc123");
        gone("4:+15551234567", "15551234567");
        gone(&format!("8:gid:{GUID}"), GUID);
    }

    #[test]
    fn conversation_ids() {
        gone("19:abcDEF123456@thread.v2", "abcDEF123456");
        gone("19:abc_def@unq.gbl.spaces", "abc_def");
        gone(
            "/conversations/19:meeting_ZZZ@thread.v2/messages",
            "meeting_ZZZ",
        );
        // the sentinel self-chat id is not sensitive
        assert_eq!(redact("send to 48:notes"), "send to 48:notes");
    }

    #[test]
    fn guids_and_tenant_ids() {
        let out = redact(&format!("tenant {GUID} and {GUID2}"));
        assert!(!out.contains(GUID) && !out.to_lowercase().contains(&GUID2.to_lowercase()));
        assert!(out.contains("<guid>"));
        gone(
            &format!("https://login.microsoftonline.com/{GUID}/oauth2/v2.0/token"),
            GUID,
        );
        gone(&format!(r#"{{"tid":"{GUID}"}}"#), GUID);
        // a bare GUID with no label
        gone(GUID, GUID);
    }

    #[test]
    fn tenant_domains() {
        gone("host contoso.onmicrosoft.com ok", "contoso");
        gone("https://contoso-my.sharepoint.com/personal/x", "contoso");
        gone("https://contoso.sharepoint.com/sites/y", "contoso");
    }

    #[test]
    fn display_name_and_content_fields() {
        gone(r#"{"displayName":"Jane Q Doe","x":1}"#, "Jane");
        gone(r#"{"content":"<p>hello secret</p>"}"#, "hello secret");
        gone(r#"{"userPrincipalName":"jane"}"#, "jane");
    }

    #[test]
    fn home_directory_user() {
        let out = redact("open /home/alice/.config/ironteams/config.json");
        assert!(!out.contains("alice"), "{out}");
        assert!(out.contains("/.config/ironteams/config.json"));
    }

    #[test]
    fn harmless_text_is_untouched() {
        let s = "INFO ironteams: connected in 120 ms, status=200 code 403 grant_type=x";
        assert_eq!(redact(s), s);
    }

    #[test]
    fn idempotent() {
        let input = format!(
            "Bearer abc.def 8:orgid:{GUID} jane@contoso.com refresh_token=0.AAA 19:x@thread.v2 {GUID2}"
        );
        let once = redact(&input);
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn realistic_multiline_log() {
        let log = format!(
            "2026-01-01T00:00:00Z INFO trouter: connected epid={GUID}\n\
             POST https://teams.microsoft.com/api/authsvc/v1.0/authz Authorization: Bearer eyJhbGciOiJSUzI1NiJ9.eyJhIjoiYiJ9.c2ln\n\
             user 8:orgid:{GUID2} <jane.doe@contoso.com> in 19:thread123@thread.v2\n\
             Set-Cookie: skypetoken_asm=tokvalue123; Path=/\n"
        );
        let out = redact(&log);
        for s in [
            GUID,
            GUID2,
            "eyJ",
            "jane",
            "contoso",
            "thread123",
            "tokvalue123",
        ] {
            assert!(!out.contains(s), "{s} in {out}");
        }
        assert!(out.contains("INFO trouter: connected"));
        assert_eq!(out.lines().count(), 4);
    }
}
