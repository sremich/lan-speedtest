//! `/metrics`, in Prometheus text exposition format.
//!
//! Off unless `server.metrics` is set. The endpoint names every client that has
//! ever run a test, which is more than an unauthenticated endpoint should hand
//! out unless someone has asked for it.
//!
//! What it exports is **the most recent run per client**, not the history. A
//! scrape is a question about now; the history is already a database and
//! re-exporting all of it every fifteen seconds would be the wrong shape for
//! both systems.
//!
//! Units follow the Prometheus convention rather than the ones the UI shows:
//! base units, so seconds rather than milliseconds and a ratio rather than a
//! percentage. A dashboard can scale for display; it cannot recover a unit that
//! was thrown away.

use std::fmt::Write as _;

use crate::history::StoredRun;

/// Escapes a label value: backslash, double quote and newline, per the
/// exposition format. Nothing else is special, and escaping more would
/// corrupt values that are legal as they stand.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// A run's identity as Prometheus labels.
///
/// The address is always present and always the same string, so a series
/// survives a client being renamed. The friendly name rides alongside as its
/// own label rather than replacing it — swapping the identity of a series when
/// someone types a nickname would break every dashboard built on it.
fn labels(run: &StoredRun) -> String {
    let name = run
        .client_name
        .as_deref()
        .or(run.hostname.as_deref())
        .unwrap_or("");
    format!(
        "client=\"{}\",name=\"{}\",profile=\"{}\"",
        escape(&run.client_ip),
        escape(name),
        escape(&run.profile)
    )
}

/// AIM's own worst-to-best ordering, restated as the ordinal Prometheus
/// wants. `bad` is 0 and `great` is 4 — the engine defines exactly these five
/// names (`speedtest.d.ts`'s `ClassificationName`), so anything else is a
/// value this build has never produced and is skipped rather than guessed
/// at.
fn classification_index(name: &str) -> Option<f64> {
    match name {
        "bad" => Some(0.0),
        "poor" => Some(1.0),
        "average" => Some(2.0),
        "good" => Some(3.0),
        "great" => Some(4.0),
        _ => None,
    }
}

/// `(loaded - idle) / idle`, or absent when either side is missing or idle
/// latency is zero.
///
/// A zero denominator is not "infinite bufferbloat" — on a LAN it is the
/// timing-resolution floor documented in DECISIONS.md, and dividing by it
/// would draw a nonsense spike rather than describe the link.
fn bufferbloat_factor(idle_ms: Option<f64>, loaded_ms: Option<f64>) -> Option<f64> {
    let idle_ms = idle_ms?;
    let loaded_ms = loaded_ms?;
    if idle_ms == 0.0 {
        return None;
    }
    Some((loaded_ms - idle_ms) / idle_ms)
}

/// One metric family: help, type, then a line per client that has the figure.
///
/// A client whose run did not measure something is **omitted** rather than
/// exported as zero. Zero packet loss and no packet-loss stage are different
/// claims, and a graph cannot tell them apart once they are both `0`.
fn family(
    out: &mut String,
    name: &str,
    help: &str,
    runs: &[StoredRun],
    value: impl Fn(&StoredRun) -> Option<f64>,
) {
    let mut wrote_header = false;
    for run in runs {
        let Some(v) = value(run).filter(|v| v.is_finite()) else {
            continue;
        };
        if !wrote_header {
            let _ = writeln!(out, "# HELP {name} {help}");
            let _ = writeln!(out, "# TYPE {name} gauge");
            wrote_header = true;
        }
        let _ = writeln!(out, "{name}{{{}}} {v}", labels(run));
    }
}

/// `speedtest_bufferbloat_factor`, one row per client per direction that has
/// both an idle and a loaded latency to compare.
///
/// A second, direction-labelled family rather than two calls to [`family`]:
/// the exposition format allows a metric name only one `# HELP`/`# TYPE`
/// block, so emitting `down` and `up` as separate families under the same
/// name would either duplicate the header or split it non-contiguously.
fn bufferbloat(out: &mut String, runs: &[StoredRun]) {
    let mut wrote_header = false;
    for run in runs {
        for (direction, loaded_ms) in [
            ("down", run.down_loaded_latency),
            ("up", run.up_loaded_latency),
        ] {
            let Some(factor) = bufferbloat_factor(run.latency, loaded_ms).filter(|v| v.is_finite())
            else {
                continue;
            };
            if !wrote_header {
                let _ = writeln!(
                    out,
                    "# HELP speedtest_bufferbloat_factor Fractional increase in round trip \
                     while saturating the link, most recent run: (loaded - idle) / idle."
                );
                let _ = writeln!(out, "# TYPE speedtest_bufferbloat_factor gauge");
                wrote_header = true;
            }
            let _ = writeln!(
                out,
                "speedtest_bufferbloat_factor{{{},direction=\"{direction}\"}} {factor}",
                labels(run)
            );
        }
    }
}

/// `speedtest_quality_score`, one row per client per AIM experience that
/// produced a rating.
///
/// The label is whatever the engine calls the experience (`streaming`,
/// `gaming`, `rtc` today, carried through [`StoredRun::scores`] unchanged)
/// rather than a hardcoded set, so an experience the engine adds later shows
/// up without a code change here.
fn quality_score(out: &mut String, runs: &[StoredRun]) {
    let mut wrote_header = false;
    for run in runs {
        for (service, classification) in &run.scores {
            let Some(idx) = classification_index(classification) else {
                continue;
            };
            if !wrote_header {
                let _ = writeln!(
                    out,
                    "# HELP speedtest_quality_score AIM quality classification of the most \
                     recent run, 0 (bad) to 4 (great)."
                );
                let _ = writeln!(out, "# TYPE speedtest_quality_score gauge");
                wrote_header = true;
            }
            let _ = writeln!(
                out,
                "speedtest_quality_score{{{},service=\"{}\"}} {idx}",
                labels(run),
                escape(service)
            );
        }
    }
}

/// The whole exposition body.
pub fn render(version: &str, git_sha: &str, total_runs: i64, latest: &[StoredRun]) -> String {
    let mut out = String::with_capacity(1024);

    let _ = writeln!(
        out,
        "# HELP speedtest_build_info Build information for this deployment."
    );
    let _ = writeln!(out, "# TYPE speedtest_build_info gauge");
    let _ = writeln!(
        out,
        "speedtest_build_info{{version=\"{}\",git_sha=\"{}\"}} 1",
        escape(version),
        escape(git_sha)
    );

    let _ = writeln!(
        out,
        "# HELP speedtest_history_runs_total Runs currently stored."
    );
    // A gauge, not a counter: pruning makes it go down, and calling a
    // decreasing series `_total` would make `rate()` produce nonsense.
    let _ = writeln!(out, "# TYPE speedtest_history_runs_total gauge");
    let _ = writeln!(out, "speedtest_history_runs_total {total_runs}");

    family(
        &mut out,
        "speedtest_download_bits_per_second",
        "Download throughput of the most recent run.",
        latest,
        |r| r.download,
    );
    family(
        &mut out,
        "speedtest_upload_bits_per_second",
        "Upload throughput of the most recent run.",
        latest,
        |r| r.upload,
    );
    family(
        &mut out,
        "speedtest_latency_seconds",
        "Idle round trip of the most recent run.",
        latest,
        |r| r.latency.map(ms_to_s),
    );
    family(
        &mut out,
        "speedtest_loaded_latency_download_seconds",
        "Round trip while saturating download, most recent run.",
        latest,
        |r| r.down_loaded_latency.map(ms_to_s),
    );
    family(
        &mut out,
        "speedtest_loaded_latency_upload_seconds",
        "Round trip while saturating upload, most recent run.",
        latest,
        |r| r.up_loaded_latency.map(ms_to_s),
    );
    family(
        &mut out,
        "speedtest_jitter_seconds",
        "Variation in round trip, most recent run.",
        latest,
        |r| r.jitter.map(ms_to_s),
    );
    family(
        &mut out,
        "speedtest_packet_loss_ratio",
        "Packet loss of the most recent run, 0 to 1.",
        latest,
        |r| r.packet_loss,
    );
    family(
        &mut out,
        "speedtest_run_timestamp_seconds",
        "When the most recent run was recorded, Unix time.",
        latest,
        |r| unix_seconds(&r.recorded_at),
    );
    bufferbloat(&mut out, latest);
    quality_score(&mut out, latest);

    out
}

fn ms_to_s(ms: f64) -> f64 {
    ms / 1000.0
}

/// RFC 3339 to Unix seconds. Unparseable timestamps are skipped rather than
/// exported as the epoch, which would draw a 1970 point on every dashboard.
fn unix_seconds(recorded_at: &str) -> Option<f64> {
    time::OffsetDateTime::parse(recorded_at, &time::format_description::well_known::Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ip: &str, name: Option<&str>) -> StoredRun {
        StoredRun {
            id: 1,
            recorded_at: "2026-08-26T10:00:00Z".into(),
            client_ip: ip.into(),
            client_name: name.map(str::to_string),
            hostname: None,
            user_agent: "ua".into(),
            profile: "lan-1g".into(),
            download: Some(940e6),
            upload: Some(880e6),
            latency: Some(0.6),
            jitter: Some(0.2),
            down_loaded_latency: Some(1.4),
            up_loaded_latency: Some(1.1),
            packet_loss: None,
            total_duration_ms: Some(20_000.0),
            scores: Default::default(),
            note: None,
            app_version: Some("1.5.0".into()),
        }
    }

    #[test]
    fn milliseconds_become_seconds() {
        // Prometheus convention is base units. A dashboard can scale for
        // display; it cannot recover a unit that was thrown away.
        let out = render("1.5.0", "abc", 1, &[run("10.0.0.1", None)]);
        assert!(
            out.contains("speedtest_latency_seconds{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\"} 0.0006"),
            "{out}"
        );
    }

    #[test]
    fn a_figure_that_was_never_measured_is_absent_not_zero() {
        // Zero packet loss and no packet-loss stage are different claims, and
        // a graph cannot tell them apart once both are `0`.
        let out = render("1.5.0", "abc", 1, &[run("10.0.0.1", None)]);
        assert!(!out.contains("speedtest_packet_loss_ratio"), "{out}");

        let mut measured = run("10.0.0.1", None);
        measured.packet_loss = Some(0.0);
        let out = render("1.5.0", "abc", 1, &[measured]);
        assert!(out.contains("speedtest_packet_loss_ratio"), "{out}");
    }

    #[test]
    fn a_name_rides_alongside_the_address_rather_than_replacing_it() {
        // Renaming a client must not silently become a different time series.
        let out = render("1.5.0", "abc", 1, &[run("10.0.0.1", Some("stevie-pc"))]);
        assert!(out.contains("client=\"10.0.0.1\""), "{out}");
        assert!(out.contains("name=\"stevie-pc\""), "{out}");
    }

    #[test]
    fn label_values_are_escaped_so_a_hostile_name_cannot_forge_a_label() {
        // A client name is user-supplied. Unescaped, `a" evil="1` would close
        // the quote and inject a label — the exposition equivalent of an
        // injection bug.
        let out = render("1.5.0", "abc", 1, &[run("10.0.0.1", Some("a\" evil=\"1"))]);
        assert!(out.contains(r#"name="a\" evil=\"1""#), "{out}");
        assert!(!out.contains(r#" evil="1""#), "{out}");
    }

    #[test]
    fn the_header_is_written_once_per_family_and_only_when_it_has_a_value() {
        let runs = [run("10.0.0.1", None), run("10.0.0.2", None)];
        let out = render("1.5.0", "abc", 2, &runs);
        assert_eq!(
            out.matches("# TYPE speedtest_download_bits_per_second gauge")
                .count(),
            1,
            "{out}"
        );
        assert_eq!(
            out.matches("speedtest_download_bits_per_second{").count(),
            2,
            "{out}"
        );
    }

    #[test]
    fn an_unparseable_timestamp_is_skipped_rather_than_drawn_at_the_epoch() {
        let mut broken = run("10.0.0.1", None);
        broken.recorded_at = "not a date".into();
        let out = render("1.5.0", "abc", 1, &[broken]);
        assert!(!out.contains("speedtest_run_timestamp_seconds{"), "{out}");
    }

    #[test]
    fn an_empty_history_still_produces_a_valid_body() {
        let out = render("1.5.0", "abc", 0, &[]);
        assert!(
            out.contains("speedtest_build_info{version=\"1.5.0\""),
            "{out}"
        );
        assert!(out.contains("speedtest_history_runs_total 0"), "{out}");
        assert!(out.ends_with('\n'), "exposition must end with a newline");
    }

    #[test]
    fn bufferbloat_factor_is_loaded_minus_idle_over_idle_for_each_direction() {
        let mut measured = run("10.0.0.1", None);
        measured.latency = Some(1.0);
        measured.down_loaded_latency = Some(2.0);
        measured.up_loaded_latency = Some(1.5);
        let out = render("1.5.0", "abc", 1, &[measured]);
        assert!(
            out.contains(
                "speedtest_bufferbloat_factor{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",direction=\"down\"} 1"
            ),
            "{out}"
        );
        assert!(
            out.contains(
                "speedtest_bufferbloat_factor{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",direction=\"up\"} 0.5"
            ),
            "{out}"
        );
    }

    #[test]
    fn bufferbloat_factor_is_absent_when_idle_or_loaded_latency_is_missing() {
        // No idle latency at all: neither direction has a baseline to compare
        // against.
        let mut no_idle = run("10.0.0.1", None);
        no_idle.latency = None;
        let out = render("1.5.0", "abc", 1, &[no_idle]);
        assert!(!out.contains("speedtest_bufferbloat_factor"), "{out}");

        // Idle latency present, but only one direction has a loaded reading:
        // that direction is exported, the other is not.
        let mut down_only = run("10.0.0.1", None);
        down_only.up_loaded_latency = None;
        let out = render("1.5.0", "abc", 1, &[down_only]);
        assert!(
            out.contains("speedtest_bufferbloat_factor{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",direction=\"down\""),
            "{out}"
        );
        assert!(!out.contains("direction=\"up\""), "{out}");
    }

    #[test]
    fn bufferbloat_factor_is_absent_when_idle_latency_is_zero() {
        // Zero idle latency is the sub-millisecond timing-resolution floor
        // documented in DECISIONS.md, not a genuinely instant link — dividing
        // by it would draw a nonsense spike instead of describing bufferbloat.
        let mut zero_idle = run("10.0.0.1", None);
        zero_idle.latency = Some(0.0);
        let out = render("1.5.0", "abc", 1, &[zero_idle]);
        assert!(!out.contains("speedtest_bufferbloat_factor"), "{out}");
    }

    #[test]
    fn quality_score_maps_each_classification_to_the_engines_own_ordinal() {
        let mut scored = run("10.0.0.1", None);
        scored.scores = [
            ("streaming".to_string(), "great".to_string()),
            ("gaming".to_string(), "bad".to_string()),
            ("rtc".to_string(), "poor".to_string()),
        ]
        .into_iter()
        .collect();
        let out = render("1.5.0", "abc", 1, &[scored]);
        assert!(
            out.contains("speedtest_quality_score{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",service=\"streaming\"} 4"),
            "{out}"
        );
        assert!(
            out.contains("speedtest_quality_score{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",service=\"gaming\"} 0"),
            "{out}"
        );
        assert!(
            out.contains("speedtest_quality_score{client=\"10.0.0.1\",name=\"\",profile=\"lan-1g\",service=\"rtc\"} 1"),
            "{out}"
        );
    }

    #[test]
    fn quality_score_is_absent_when_the_run_recorded_no_scores() {
        // The default fixture's `scores` is empty, matching a run whose
        // loaded latency never cleared `loadedLatencyIncrease`'s truthiness
        // guard — see DECISIONS.md.
        let out = render("1.5.0", "abc", 1, &[run("10.0.0.1", None)]);
        assert!(!out.contains("speedtest_quality_score"), "{out}");
    }

    #[test]
    fn an_unrecognised_classification_name_is_skipped_rather_than_guessed_at() {
        // The engine defines exactly five classification names. A string
        // outside that set is a value this build has never produced, and
        // inventing an ordinal for it would be a guess dressed up as data.
        let mut scored = run("10.0.0.1", None);
        scored.scores = [("streaming".to_string(), "excellent".to_string())]
            .into_iter()
            .collect();
        let out = render("1.5.0", "abc", 1, &[scored]);
        assert!(!out.contains("speedtest_quality_score"), "{out}");
    }
}
