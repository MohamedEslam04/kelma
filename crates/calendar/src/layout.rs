//! Side-by-side columns for overlapping events in one day.

use crate::Event;

/// Assigns each event a column so overlapping events sit side by side. Returns
/// `(event_index, column, column_count)` sorted by `event_index`, where `column_count` is the
/// number of columns of the whole overlap cluster the event belongs to (so every event in a
/// cluster has the same width).
///
/// Events that only touch (one ends when the next starts) do not overlap. A zero-length event
/// counts as one millisecond long.
pub fn layout_columns(events: &[Event]) -> Vec<(usize, usize, usize)> {
    let end_of = |i: usize| events[i].end_ms.max(events[i].start_ms + 1);
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| (events[i].start_ms, std::cmp::Reverse(end_of(i))));

    let mut out = Vec::with_capacity(events.len());
    let mut cluster: Vec<(usize, usize)> = Vec::new(); // (event_index, column)
    let mut col_ends: Vec<i64> = Vec::new(); // end of the last event in each column
    let mut cluster_end = i64::MIN;

    let mut flush = |cluster: &mut Vec<(usize, usize)>, cols: usize| {
        out.extend(cluster.drain(..).map(|(i, c)| (i, c, cols)));
    };

    for i in order {
        if events[i].start_ms >= cluster_end {
            flush(&mut cluster, col_ends.len());
            col_ends.clear();
        }
        let col = match col_ends.iter().position(|&e| e <= events[i].start_ms) {
            Some(c) => c,
            None => {
                col_ends.push(0);
                col_ends.len() - 1
            }
        };
        col_ends[col] = end_of(i);
        cluster_end = cluster_end.max(end_of(i));
        cluster.push((i, col));
    }
    flush(&mut cluster, col_ends.len());
    out.sort_by_key(|&(i, _, _)| i);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_events;
    use serde_json::json;

    /// Events at the given (start, end) offsets in minutes from midnight 2026-11-18 UTC.
    fn events(spans: &[(i64, i64)]) -> Vec<Event> {
        let fmt = |m: i64| format!("2026-11-18T{:02}:{:02}:00", m / 60, m % 60);
        let items: Vec<_> = spans
            .iter()
            .enumerate()
            .map(|(i, &(s, e))| {
                json!({"id": format!("e{i}"),
                    "start": {"dateTime": fmt(s), "timeZone": "UTC"},
                    "end": {"dateTime": fmt(e), "timeZone": "UTC"}})
            })
            .collect();
        let evs = parse_events(&json!({ "value": items }));
        assert_eq!(evs.len(), spans.len());
        evs
    }

    #[test]
    fn three_overlapping_and_one_separate() {
        // 9:00-10:00, 9:30-10:30, 9:45-11:00 overlap; 12:00-13:00 stands alone
        let evs = events(&[(540, 600), (570, 630), (585, 660), (720, 780)]);
        assert_eq!(
            layout_columns(&evs),
            vec![(0, 0, 3), (1, 1, 3), (2, 2, 3), (3, 0, 1)]
        );
    }

    #[test]
    fn column_is_reused_when_free_and_cluster_width_is_shared() {
        // C starts when A ends, so it reuses column 0 but overlaps B
        let evs = events(&[(540, 600), (570, 630), (600, 660)]);
        assert_eq!(layout_columns(&evs), vec![(0, 0, 2), (1, 1, 2), (2, 0, 2)]);
    }

    #[test]
    fn touching_events_do_not_overlap() {
        let evs = events(&[(540, 600), (600, 660)]);
        assert_eq!(layout_columns(&evs), vec![(0, 0, 1), (1, 0, 1)]);
    }

    #[test]
    fn input_order_does_not_matter_and_result_is_by_input_index() {
        let evs = events(&[(585, 660), (540, 600), (570, 630)]);
        assert_eq!(layout_columns(&evs), vec![(0, 2, 3), (1, 0, 3), (2, 1, 3)]);
    }

    #[test]
    fn empty_input() {
        assert!(layout_columns(&[]).is_empty());
    }
}
