//! Who is outside the user's organization: Teams' "(External)" and "(Guest)" labels on people in
//! meetings, the lobby, hover cards and 1:1 chats.
//!
//! Decided from the MRI prefix first (`8:live:` consumer accounts, `8:teamsvisitor:` anonymous
//! meeting guests), then a guest flag (the roster's, or a `#EXT#` sign-in name), then the tenant:
//! an organization user whose tenant id differs from ours is external. Without a tenant id for
//! them nothing is claimed.

/// How someone relates to our organization when they are not part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outsider {
    /// Another organization's user or a consumer Microsoft account.
    External,
    /// A guest of our organization or an anonymous meeting participant.
    Guest,
}

impl Outsider {
    /// The word of the label: `External` or `Guest`.
    pub fn label(self) -> &'static str {
        match self {
            Outsider::External => "External",
            Outsider::Guest => "Guest",
        }
    }
}

/// Whether `mri` (with their `tenant`, our `own_tenant` and a roster `guest` flag, where known)
/// is outside our organization.
pub fn classify(
    mri: &str,
    tenant: Option<&str>,
    own_tenant: Option<&str>,
    guest: bool,
) -> Option<Outsider> {
    if mri.starts_with("8:teamsvisitor:") {
        return Some(Outsider::Guest);
    }
    if mri.starts_with("8:live:") {
        return Some(Outsider::External);
    }
    if !mri.starts_with("8:orgid:") {
        return None;
    }
    if guest {
        return Some(Outsider::Guest);
    }
    match (tenant, own_tenant) {
        (Some(t), Some(own)) if !t.eq_ignore_ascii_case(own) => Some(Outsider::External),
        _ => None,
    }
}

/// A guest's sign-in name carries `#EXT#` (`name_home.com#EXT#@tenant.onmicrosoft.com`).
pub fn is_guest_upn(email: &str) -> bool {
    email.to_ascii_uppercase().contains("#EXT#")
}

/// `name` with its label, as a chat title shows it: `Sara Example (External)`.
pub fn tagged(name: &str, outsider: Option<Outsider>) -> String {
    match outsider {
        // A title the chat list already tagged and the header was opened with.
        Some(o) if !name.ends_with(&format!(" ({})", o.label())) => {
            format!("{name} ({})", o.label())
        }
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORG: &str = "8:orgid:00000000-0000-0000-0000-0000000000a1";

    #[test]
    fn a_colleague_has_no_label() {
        assert_eq!(classify(ORG, Some("t-1"), Some("t-1"), false), None);
        assert_eq!(classify(ORG, Some("T-1"), Some("t-1"), false), None);
    }

    #[test]
    fn another_tenants_user_is_external() {
        assert_eq!(
            classify(ORG, Some("t-2"), Some("t-1"), false),
            Some(Outsider::External)
        );
    }

    #[test]
    fn nothing_is_claimed_without_both_tenant_ids() {
        assert_eq!(classify(ORG, None, Some("t-1"), false), None);
        assert_eq!(classify(ORG, Some("t-2"), None, false), None);
        assert_eq!(classify(ORG, None, None, false), None);
    }

    #[test]
    fn a_consumer_account_is_external() {
        assert_eq!(
            classify("8:live:.cid.0123456789abcdef", None, Some("t-1"), false),
            Some(Outsider::External)
        );
    }

    #[test]
    fn an_anonymous_participant_is_a_guest() {
        assert_eq!(
            classify(
                "8:teamsvisitor:00000000000000000000000000000c01",
                None,
                None,
                false
            ),
            Some(Outsider::Guest)
        );
    }

    #[test]
    fn a_guest_flag_makes_a_guest_even_of_our_tenant() {
        assert_eq!(
            classify(ORG, Some("t-1"), Some("t-1"), true),
            Some(Outsider::Guest)
        );
    }

    #[test]
    fn bots_and_unknown_prefixes_have_no_label() {
        assert_eq!(classify("28:bot", Some("t-2"), Some("t-1"), true), None);
        assert_eq!(classify("", None, None, false), None);
    }

    #[test]
    fn a_sign_in_name_with_ext_is_a_guest() {
        assert!(is_guest_upn(
            "sara_home.example#EXT#@contoso.onmicrosoft.com"
        ));
        assert!(is_guest_upn(
            "sara_home.example#ext#@contoso.onmicrosoft.com"
        ));
        assert!(!is_guest_upn("sara@contoso.example"));
    }

    #[test]
    fn a_title_shows_the_label() {
        assert_eq!(
            tagged("Sara Example", Some(Outsider::External)),
            "Sara Example (External)"
        );
        assert_eq!(
            tagged("Sara Example", Some(Outsider::Guest)),
            "Sara Example (Guest)"
        );
        assert_eq!(tagged("Sara Example", None), "Sara Example");
        assert_eq!(
            tagged("Sara Example (External)", Some(Outsider::External)),
            "Sara Example (External)",
            "never tagged twice"
        );
    }
}
