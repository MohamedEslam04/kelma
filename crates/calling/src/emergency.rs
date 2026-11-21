//! Emergency numbers (plans/calls.md K3.3).
//!
//! IronTeams is not a compliant emergency endpoint: it sends no location and cannot promise the call
//! reaches the right dispatch centre. So a number that is an emergency short code anywhere is
//! refused before dialling, and the user is told to use a phone.

/// Emergency short codes in common use. Matching is on the whole dialled number, so a longer
/// number that merely starts or ends with one of these is never caught.
const EMERGENCY: &[&str] = &[
    "000", // Australia
    "08",  // Afghanistan, others
    "100", "101", "102", "108", // India, Israel, Greece
    "110", "112", "113", "114", "115", "117", "118", "119", // EU, Japan, China, Korea, others
    "120", "122", "123", "125", "126", "180", // Egypt (122, 123, 180), China (120)
    "190", "191", "192", "193", "199", // Brazil, Thailand
    "911", "933", "999", // North America, Philippines, UK and others
    "15", "17", "18", "197", // France, Tunisia
];

/// The help text shown when a dial is refused.
pub const REFUSAL: &str =
    "IronTeams can't place emergency calls. Use a phone to call your local emergency number.";

/// Whether `dialled` is an emergency number. Spaces, dashes, dots and brackets are ignored;
/// a leading `+` or `00` (an international number) is never an emergency short code.
pub fn is_emergency_number(dialled: &str) -> bool {
    let s: String = dialled
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '.' | '(' | ')' | '\u{a0}'))
        .collect();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    EMERGENCY.contains(&s.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_codes_are_emergency_numbers() {
        for n in ["911", "112", "999", "122", "123", "180", "000", "110", "15"] {
            assert!(is_emergency_number(n), "{n}");
        }
    }

    #[test]
    fn formatting_is_ignored() {
        assert!(is_emergency_number(" 9-1-1 "));
        assert!(is_emergency_number("(112)"));
        assert!(is_emergency_number("1.2.2"));
    }

    #[test]
    fn ordinary_numbers_and_people_are_not() {
        for n in [
            "+201221234567",
            "01221234567",
            "+1 911 555 0100",
            "9110",
            "1122",
            "+112",
            "00112",
            "8:orgid:abc",
            "",
            "user@example.com",
        ] {
            assert!(!is_emergency_number(n), "{n}");
        }
    }
}
