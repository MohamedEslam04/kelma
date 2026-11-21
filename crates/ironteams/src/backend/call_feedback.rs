//! The post-call quality survey ("How was the call quality?"): when to ask, and what Send keeps.
//!
//! Teams asks after some calls. IronTeams asks when a call or meeting that lasted at least
//! [`MIN_CALL`] ends, always or one time in [`SOMETIMES_ONE_IN`] by the Settings choice. Microsoft's
//! call-quality feedback endpoint is not known (none of the reference clients documents one), so
//! nothing is posted: Send appends one JSON line to [`FILE`] in IronTeams's data dir and logs it. A line
//! holds no names and no ids.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::time::Duration;

/// A call shorter than this is never asked about.
pub const MIN_CALL: Duration = Duration::from_secs(30);
/// "Sometimes" asks one time in this many.
pub const SOMETIMES_ONE_IN: u32 = 3;
/// The ratings, in the data dir.
pub const FILE: &str = "call-feedback.jsonl";

/// Settings › Calls › "Ask about call quality after calls".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ask {
    Always,
    #[default]
    Sometimes,
    Never,
}

impl Ask {
    pub fn slug(self) -> &'static str {
        match self {
            Ask::Always => "always",
            Ask::Sometimes => "sometimes",
            Ask::Never => "never",
        }
    }

    pub fn from_slug(s: &str) -> Option<Ask> {
        [Ask::Always, Ask::Sometimes, Ask::Never]
            .into_iter()
            .find(|a| a.slug() == s)
    }
}

/// The call lasted [`MIN_CALL`] or more.
pub fn long_enough(call: Duration) -> bool {
    call >= MIN_CALL
}

/// Whether this call gets the survey, by the Settings choice. `roll` is a random number; `demo`
/// (demo mode and tests) makes "Sometimes" ask every time so a demo run always shows it.
pub fn sampled(ask: Ask, roll: u32, demo: bool) -> bool {
    match ask {
        Ask::Always => true,
        Ask::Sometimes => demo || roll.is_multiple_of(SOMETIMES_ONE_IN),
        Ask::Never => false,
    }
}

/// A random number for [`sampled`], from the standard library's randomly seeded hasher.
pub fn roll() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish() as u32
}

/// What the survey sent: the rating and the issues ticked, each with its category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feedback {
    pub good: bool,
    /// `(category, issue)`: audio | video | screenshare, and the issue's text.
    pub issues: Vec<(String, String)>,
}

impl Feedback {
    /// Reads the survey's argument: `good` or `issues`, then one `category=issue` line per box
    /// ticked. `None` for anything else.
    pub fn parse(arg: &str) -> Option<Feedback> {
        let mut lines = arg.lines();
        let good = match lines.next()? {
            "good" => true,
            "issues" => false,
            _ => return None,
        };
        let issues = lines
            .filter_map(|l| l.split_once('='))
            .map(|(c, i)| (c.to_owned(), i.to_owned()))
            .collect();
        Some(Feedback {
            good,
            issues: if good { Vec::new() } else { issues },
        })
    }

    /// The JSON line to keep: time, call kind, length, rating, categories and issues.
    pub fn line(&self, at: chrono::DateTime<chrono::Utc>, kind: &str, call: Duration) -> String {
        let mut categories: Vec<&str> = Vec::new();
        for (c, _) in &self.issues {
            if !categories.contains(&c.as_str()) {
                categories.push(c);
            }
        }
        serde_json::json!({
            "time": at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            "kind": kind,
            "seconds": call.as_secs(),
            "rating": if self.good { "good" } else { "issues" },
            "categories": categories,
            "issues": self.issues.iter().map(|(_, i)| i).collect::<Vec<_>>(),
        })
        .to_string()
    }
}

/// Appends `line` and a newline to the ratings file, creating it and its directory.
pub fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    const LONG: Duration = Duration::from_secs(95);

    #[test]
    fn the_settings_choice_round_trips_and_defaults_to_sometimes() {
        assert_eq!(Ask::default(), Ask::Sometimes);
        for a in [Ask::Always, Ask::Sometimes, Ask::Never] {
            assert_eq!(Ask::from_slug(a.slug()), Some(a));
        }
        assert_eq!(Ask::from_slug("maybe"), None);
        let a: Ask = serde_json::from_str(r#""never""#).unwrap();
        assert_eq!(a, Ask::Never);
    }

    #[test]
    fn a_short_call_is_never_asked_about() {
        assert!(!long_enough(MIN_CALL - Duration::from_millis(1)));
        assert!(!long_enough(Duration::ZERO));
        assert!(long_enough(MIN_CALL), "30 s is enough");
        assert!(long_enough(LONG));
    }

    #[test]
    fn never_asks_never_and_always_asks_always() {
        for roll in 0..9 {
            assert!(!sampled(Ask::Never, roll, true));
            assert!(sampled(Ask::Always, roll, false));
        }
    }

    #[test]
    fn sometimes_asks_one_time_in_three_but_every_time_in_the_demo() {
        let asked = (0..300)
            .filter(|r| sampled(Ask::Sometimes, *r, false))
            .count();
        assert_eq!(asked, 100);
        assert!((0..9).all(|r| sampled(Ask::Sometimes, r, true)));
    }

    #[test]
    fn rolls_are_not_all_the_same() {
        let first = roll();
        assert!((0..64).any(|_| roll() != first));
    }

    #[test]
    fn the_survey_argument_reads_the_rating_and_the_ticked_issues() {
        assert_eq!(
            Feedback::parse("good"),
            Some(Feedback {
                good: true,
                issues: vec![]
            })
        );
        let f =
            Feedback::parse("issues\naudio=I couldn't hear any sound\nvideo=Video was blurry\n")
                .unwrap();
        assert!(!f.good);
        assert_eq!(
            f.issues,
            [
                ("audio".to_owned(), "I couldn't hear any sound".to_owned()),
                ("video".to_owned(), "Video was blurry".to_owned()),
            ]
        );
        let none = Feedback::parse("issues").unwrap();
        assert!(!none.good && none.issues.is_empty(), "no box needs ticking");
        assert_eq!(Feedback::parse(""), None);
        assert_eq!(Feedback::parse("great"), None);
    }

    #[test]
    fn a_line_has_the_time_kind_length_rating_categories_and_issues_only() {
        let at = chrono::Utc
            .with_ymd_and_hms(2026, 11, 20, 9, 30, 5)
            .unwrap();
        let f = Feedback::parse(
            "issues\naudio=Volume was low\naudio=Speech was distorted\nvideo=Video kept freezing",
        )
        .unwrap();
        let line = f.line(at, "direct", LONG);
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "time": "2026-11-20T09:30:05Z",
                "kind": "direct",
                "seconds": 95,
                "rating": "issues",
                "categories": ["audio", "video"],
                "issues": ["Volume was low", "Speech was distorted", "Video kept freezing"],
            })
        );
        let good = Feedback::parse("good")
            .unwrap()
            .line(at, "meeting", MIN_CALL);
        let v: serde_json::Value = serde_json::from_str(&good).unwrap();
        assert_eq!(v["rating"], "good");
        assert_eq!(v["categories"], serde_json::json!([]));
        assert_eq!(v["issues"], serde_json::json!([]));
    }

    #[test]
    fn append_adds_one_line_per_rating_and_creates_the_directory() {
        let dir = std::env::temp_dir().join(format!("ironteams-feedback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("data").join(FILE);
        append(&path, "{\"a\":1}").unwrap();
        append(&path, "{\"a\":2}").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
