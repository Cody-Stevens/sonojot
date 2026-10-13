//! Bounded recorder diagnostics. Active gaps belong to SourceState, so retaining
//! or coalescing completed issues can never invalidate an open gap.
use super::{clock_text, source_index, IssueKind, IssueSource, SourceIssue};
use std::collections::VecDeque;

const RETAINED_ISSUES: usize = 50;
const ISSUE_KINDS: usize = 4;
const COALESCE_SECONDS: f64 = 2.0;

#[derive(Default)]
pub(super) struct IssueCollector {
    recent: VecDeque<SourceIssue>,
    remainder: [Option<SourceIssue>; 2 * ISSUE_KINDS],
    // Preserve every substantial interval before short-gap aggregation can
    // erase its boundaries. Storage grows only with actual long outages.
    outages: Vec<SourceIssue>,
}

fn kind_index(kind: IssueKind) -> usize {
    match kind {
        IssueKind::Disconnected => 0,
        IssueKind::Overflow => 1,
        IssueKind::InvalidBuffer => 2,
        IssueKind::Discontinuity => 3,
    }
}

fn describe(kind: IssueKind) -> &'static str {
    match kind {
        IssueKind::Disconnected => "disconnects",
        IssueKind::Overflow => "queue overflows",
        IssueKind::InvalidBuffer => "invalid buffers",
        IssueKind::Discontinuity => "gaps",
    }
}

fn summary_message(issue: &mut SourceIssue, earlier: bool) {
    issue.message = format!(
        "{}{} {}: {} occurrences, {:.3} seconds of audio lost from {} to {}; recording continued.",
        if earlier { "Earlier " } else { "" },
        issue.source.label(),
        describe(issue.kind),
        issue.occurrences,
        issue.lost_duration_seconds,
        clock_text(issue.started_at_seconds),
        issue.ended_at_seconds.map(clock_text).unwrap_or_else(|| "Stop".into()),
    );
}

fn merge(into: &mut SourceIssue, next: SourceIssue, earlier: bool) {
    into.started_at_seconds = into.started_at_seconds.min(next.started_at_seconds);
    into.ended_at_seconds = match (into.ended_at_seconds, next.ended_at_seconds) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => None,
    };
    into.occurrences = into.occurrences.saturating_add(next.occurrences);
    // Endpoints describe the covered range, not the missing duration. In
    // particular, healthy audio between two recurring gaps is never added.
    into.lost_duration_seconds += next.lost_duration_seconds;
    summary_message(into, earlier);
}

impl IssueCollector {
    pub(super) fn push(&mut self, mut issue: SourceIssue) {
        if issue.is_outage() {
            self.outages.push(issue);
            return;
        }
        if let Some(previous) = self.recent.iter_mut().rev().find(|previous| {
            previous.source == issue.source && previous.kind == issue.kind
        }) {
            if previous.ended_at_seconds.is_some_and(|end| {
                issue.started_at_seconds >= end
                    && issue.started_at_seconds - end <= COALESCE_SECONDS
            }) {
                merge(previous, issue, false);
                return;
            }
        }
        if self.recent.len() == RETAINED_ISSUES {
            let mut evicted = self.recent.pop_front().expect("retained issue exists");
            let slot = source_index(evicted.source) * ISSUE_KINDS + kind_index(evicted.kind);
            match &mut self.remainder[slot] {
                Some(summary) => merge(summary, evicted, true),
                empty => {
                    summary_message(&mut evicted, true);
                    *empty = Some(evicted);
                }
            }
        }
        if issue.occurrences > 1 {
            summary_message(&mut issue, false);
        }
        self.recent.push_back(issue);
    }

    pub(super) fn finish(self) -> Vec<SourceIssue> {
        let mut issues: Vec<_> = self.remainder.into_iter().flatten().chain(self.recent).chain(self.outages).collect();
        issues.sort_by(|a, b| a.started_at_seconds.total_cmp(&b.started_at_seconds));
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gap(source: IssueSource, kind: IssueKind, start: f64) -> SourceIssue {
        SourceIssue {
            source,
            kind,
            started_at_seconds: start,
            ended_at_seconds: Some(start + 0.125),
            occurrences: 1,
            lost_duration_seconds: 0.125,
            message: "A single gap.".into(),
        }
    }

    #[test]
    fn thousands_of_recurring_discontinuities_count_only_missing_audio() {
        let mut issues = IssueCollector::default();
        for index in 0..10_000 {
            issues.push(gap(IssueSource::System, IssueKind::Discontinuity, index as f64));
            assert_eq!(issues.recent.len(), 1);
        }
        let issues = issues.finish();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].occurrences, 10_000);
        assert_eq!(issues[0].lost_duration_seconds, 1250.0);
        assert_eq!(issues[0].ended_at_seconds, Some(9999.125));
        assert!(issues[0].message.len() < 256);
    }

    #[test]
    fn retained_remainders_keep_every_source_kind_count_and_duration() {
        let mut issues = IssueCollector::default();
        for index in 0..10_000 {
            for source in [IssueSource::Microphone, IssueSource::System] {
                for kind in [IssueKind::Disconnected, IssueKind::Overflow,
                    IssueKind::InvalidBuffer, IssueKind::Discontinuity] {
                    issues.push(gap(source, kind, index as f64 * 3.0));
                }
            }
            assert!(issues.recent.len() <= RETAINED_ISSUES);
        }
        assert!(issues.remainder.iter().all(Option::is_some));
        let issues = issues.finish();
        assert_eq!(issues.len(), RETAINED_ISSUES + 8);
        for source in [IssueSource::Microphone, IssueSource::System] {
            for kind in [IssueKind::Disconnected, IssueKind::Overflow,
                IssueKind::InvalidBuffer, IssueKind::Discontinuity] {
                let matching: Vec<_> = issues.iter().filter(|issue| {
                    issue.source == source && issue.kind == kind
                }).collect();
                assert_eq!(matching.iter().map(|issue| issue.occurrences).sum::<u64>(), 10_000);
                assert_eq!(matching.iter().map(|issue| issue.lost_duration_seconds).sum::<f64>(), 1250.0);
                assert!(matching.iter().any(|issue| issue.message.starts_with("Earlier ")));
            }
        }
    }

    #[test]
    fn long_and_unrecovered_outages_keep_individual_bounds_after_eviction() {
        let mut issues = IssueCollector::default();
        let mut long = gap(IssueSource::Microphone, IssueKind::Disconnected, 12.0);
        long.ended_at_seconds = Some(26.0);
        long.lost_duration_seconds = 14.0;
        issues.push(long);
        for index in 0..500 {
            issues.push(gap(IssueSource::Microphone, IssueKind::Disconnected, 30.0 + index as f64 * 3.0));
        }
        let mut open = gap(IssueSource::System, IssueKind::Disconnected, 1600.0);
        open.ended_at_seconds = None;
        open.lost_duration_seconds = 1.0;
        issues.push(open);
        let issues = issues.finish();
        let marked: Vec<_> = issues.iter().filter(|issue| issue.is_outage()).collect();
        assert_eq!(marked.len(), 2);
        assert_eq!(marked[0].started_at_seconds, 12.0);
        assert_eq!(marked[0].ended_at_seconds, Some(26.0));
        assert_eq!(marked[1].started_at_seconds, 1600.0);
        assert_eq!(marked[1].ended_at_seconds, None);
        assert_eq!(issues.iter().map(|issue| issue.lost_duration_seconds).sum::<f64>(), 77.5);
    }

    #[test]
    fn coalescing_includes_two_second_boundary_but_keeps_other_kinds_separate() {
        let mut issues = IssueCollector::default();
        issues.push(gap(IssueSource::System, IssueKind::Overflow, 0.0));
        issues.push(gap(IssueSource::System, IssueKind::Disconnected, 1.0));
        issues.push(gap(IssueSource::System, IssueKind::Overflow, 2.125));
        issues.push(gap(IssueSource::System, IssueKind::Overflow, 4.251));
        let issues = issues.finish();
        assert_eq!(issues.len(), 3);
        assert_eq!(issues[0].occurrences, 2);
        assert_eq!(issues[0].lost_duration_seconds, 0.25);
    }
}
