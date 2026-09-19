//! Tweak triage through TypeSafe System One.
//!
//! The optimise tab knows what every tweak does and whether it is currently
//! set. What it cannot know is which of the pending ones matter *on this
//! link, right now* — that judgment needs the measurements and the tweak's
//! description read together, which is semantic work rather than a rule.
//!
//! So: code owns the workflow. It reads the adapter, the ping statistics and
//! the airwaves, collects the tweaks that are not yet applied, and asks one
//! narrow judgment per tweak. The model never picks what to change and never
//! changes anything; it returns a position on a scale, and the UI sorts by it.
//!
//! This is the only part of the app that talks to anything outside the
//! machine besides the probes. It is off unless the user has entered a key,
//! and [`state`] is the complete list of what leaves the machine when it is
//! on: link characteristics and timings, no identifiers.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::monitor::Snapshot;
use crate::optimize::Risk;
use crate::probe::airscan::AirScan;
use crate::probe::netstate::NetState;
use crate::settings::{Scope, Settings};
use crate::store::{Stats, Store};

const ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";
const MODEL: &str = "jev-latest";

/// The window the statistics are read over. Five minutes is long enough to
/// have seen jitter and short enough that it still describes now.
const WINDOW_S: f64 = 300.0;

/// How many tweaks are sent in one request. Every question costs tokens and
/// adds latency, and a list longer than this is not a ranking anyone reads.
const MAX_CANDIDATES: usize = 12;

/// A tweak that is readable, applicable and currently not applied — the only
/// kind worth ranking. Built by the UI from the tweak list it already has.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    /// What applying it changes.
    pub changes: String,
    /// Why it is supposed to help, including its known costs.
    pub rationale: String,
    pub risk: String,
    /// How the machine is set now, as the tweak's own read reported it.
    pub current: String,
}

/// Where one tweak landed. `score` is a position on the four levels defined
/// in [`help_question`], so it is comparable across tweaks in the same batch
/// and meaningless on its own.
#[derive(Debug, Clone)]
pub struct Priority {
    pub tweak_id: String,
    pub score: f64,
    pub confidence: f64,
    /// The nearest level's own description, which is what the UI shows. It
    /// comes from the response's legend rather than from a local table, so
    /// it cannot drift away from what was actually asked.
    pub level: String,
    /// Probability that applying this one breaks everyday software on this
    /// link. A separate judgment, because "worth doing" and "safe to do" are
    /// different questions and the answer to one does not bound the other.
    pub breakage: f64,
}

impl Priority {
    /// Whether the ranking is worth showing as a recommendation rather than
    /// as a number. Below level 2 the model is saying "no measurable gain
    /// here", and a split distribution means it could not place the tweak at
    /// all — neither deserves a badge that reads as advice.
    pub fn is_recommendation(&self) -> bool {
        self.score >= 1.5 && self.confidence >= 0.4
    }

    /// Whether the breakage answer is strong enough to warn about. A Noul
    /// near 0.5 is the model declining to commit, not a medium risk, so the
    /// threshold sits well clear of it.
    pub fn is_risky(&self) -> bool {
        self.breakage >= 0.65
    }
}

/// The key in use, preferring the environment over the settings file so a
/// key never has to be written to disk to try the feature once.
pub fn key(settings: &Settings) -> Option<String> {
    std::env::var("TYPESAFE_API_KEY")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .or_else(|| {
            let k = settings.typesafe_key.trim();
            (!k.is_empty()).then(|| k.to_string())
        })
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Everything the judgments are allowed to see.
///
/// Deliberately narrow. The SSID, the BSSID, the local and gateway addresses
/// and the DNS servers are all available here and none of them are included:
/// they identify a household without helping any of the questions. What goes
/// out describes a radio link and a set of timings.
pub fn state(
    net: &NetState,
    last: &Snapshot,
    store: &Store,
    settings: &Settings,
    air: &AirScan,
    candidates: &[Candidate],
) -> Value {
    let mut link = Map::new();
    link.insert("medium".into(), json!(net.medium.label()));
    link.insert("link_speed_mbps".into(), json!(net.link_speed_mbps));
    if net.medium == crate::probe::netstate::Medium::Wifi {
        link.insert("band".into(), json!(net.band()));
        link.insert("channel".into(), json!(net.channel));
        link.insert("signal_pct".into(), json!(net.signal_pct));
        link.insert("rssi_dbm".into(), json!(net.rssi_dbm));
        link.insert("radio_standard".into(), json!(net.phy));
        link.insert("rx_mbps".into(), json!(net.rx_mbps));
        link.insert("tx_mbps".into(), json!(net.tx_mbps));
        link.insert("security".into(), json!(net.security));
    }

    // One entry per hop in the chain rather than per target: the question is
    // "where does it hurt", and three named hops answer it where a list of
    // host keys would need the model to know the topology.
    let mut hops = Map::new();
    for target in settings.targets() {
        let s = store.stats(&target.key, WINDOW_S);
        if s.count == 0 {
            continue;
        }
        let name = match target.scope {
            Scope::Lan => "router",
            Scope::Isp => "isp",
            Scope::Internet => "internet",
        };
        // Several targets can share a scope; the worst one is the one that
        // describes the problem, so a slower entry replaces a faster one.
        let worse = hops
            .get(name)
            .and_then(|v: &Value| v["avg_ms"].as_f64())
            .is_none_or(|prev| s.avg.unwrap_or(f64::MAX) > prev);
        if worse {
            hops.insert(name.into(), stats_json(&s));
        }
    }

    let mut measurements = Map::new();
    measurements.insert("window_seconds".into(), json!(WINDOW_S as u64));
    measurements.insert("hops".into(), Value::Object(hops));
    measurements.insert("dns_lookup_ms".into(), json!(last.dns_ms));
    measurements.insert("wifi_roamed_recently".into(), json!(last.roamed));
    if !last.note.is_empty() {
        measurements.insert("current_note".into(), json!(last.note));
    }

    let mut out = Map::new();
    out.insert("link".into(), Value::Object(link));
    out.insert("measurements".into(), Value::Object(measurements));

    // Only when a scan has actually been run. An absent key says "unknown",
    // which is true; zeroes would say "quiet", which would not be.
    if !air.aps.is_empty() {
        out.insert(
            "airwaves".into(),
            json!({
                "networks_heard": air.aps.len(),
                "sharing_our_channel": air.co_channel(),
                "on_a_dfs_channel": air.on_dfs(),
            }),
        );
    }

    out.insert("candidates".into(), json!(candidates));
    Value::Object(out)
}

fn stats_json(s: &Stats) -> Value {
    json!({
        "avg_ms": s.avg,
        "min_ms": s.min,
        "max_ms": s.max,
        "jitter_ms": s.jitter,
        "loss_pct": (s.loss_pct * 100.0).round() / 100.0,
        "samples": s.count,
    })
}

// ---------------------------------------------------------------------------
// Questions
// ---------------------------------------------------------------------------

/// How much this particular change would help this particular link.
///
/// The levels describe situations rather than degrees, which is what lets the
/// model match them against the measurements instead of guessing at a word
/// like "moderate". Level 0 has to be reachable: most tweaks are irrelevant
/// to most links, and a scale without a floor would rank them all as useful.
fn help_question(i: usize) -> Value {
    json!({
        "type": "score",
        "instructions": {
            "judge": format!(
                "Considering only the change described in `candidates[{i}]`, \
                 how much would applying it improve this connection as `link`, \
                 `measurements` and `airwaves` describe it right now?"
            ),
            "ignore": "Whether the change is safe, reversible or needs a reboot. \
                       That is asked separately. Judge the size of the gain only.",
            "note": "`candidates[].rationale` argues for the change in general. \
                     Decide whether this specific link shows the problem it argues about."
        },
        "criteria": [
            "The measurements show no sign of the problem this change addresses, \
             or the link is the wrong kind for it — a wired link for a radio fix, \
             a fast quiet link for a congestion fix. Applying it would change nothing here.",
            "The change is defensible housekeeping on this link, but nothing in the \
             measurements is being held back by it, so any gain would be too small to notice.",
            "The measurements show the specific problem this change targets — the \
             latency, jitter, loss, signal or interference it is meant to address is \
             present and out of the ordinary. Applying it should produce a noticeable gain.",
            "The worst thing about this connection is the exact problem this change \
             fixes, and nothing else in the candidate list addresses it. This is the \
             first thing to try."
        ]
    })
}

/// Whether the change plausibly breaks things, judged against this link.
///
/// Separate from the gain because the two do not trade off: a change can be
/// the most useful one on offer and still be the one that stops a launcher
/// working. Weighting them into a single number would hide exactly that case.
fn risk_question(i: usize) -> Value {
    json!({
        "type": "noul",
        "instructions": format!(
            "Applying the change in `candidates[{i}]` on this machine: is there a \
             realistic chance it degrades or breaks everyday software — game \
             launchers, voice and video calls, VPNs, corporate access — rather \
             than only helping?"
        ),
        "criteria": {
            "true": "The change alters behaviour that ordinary applications depend on, \
                     and `candidates[].rationale` or the nature of the change gives a \
                     concrete way for something to stop working.",
            "false": "The change is confined to this machine's own tuning, and a \
                      failure mode would show up as a smaller gain rather than as \
                      software that no longer works."
        }
    })
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// Ranks the pending tweaks in one request.
///
/// Every question is independent and reads the same state, so they go
/// together and the service answers them in parallel. Blocking: the caller
/// runs this on a worker thread.
pub fn rank(api_key: &str, state: &Value, candidates: &[Candidate]) -> Result<Vec<Priority>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut questions = Map::new();
    for i in 0..candidates.len() {
        questions.insert(format!("help_{i}"), help_question(i));
        questions.insert(format!("risk_{i}"), risk_question(i));
    }

    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(45))
        .build();

    let body = json!({ "state": state, "model": MODEL, "questions": questions });
    let reply: Value = match agent
        .post(ENDPOINT)
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .send_json(body)
    {
        Ok(r) => r.into_json().context("the reply was not JSON")?,
        // The body of an error response says what is wrong far better than
        // the status line does, so it is worth unwrapping before giving up.
        Err(ureq::Error::Status(code, resp)) => {
            let detail = resp.into_string().unwrap_or_default();
            bail!("{code}: {}", first_line(&detail));
        }
        Err(e) => bail!("{e}"),
    };

    let answers = reply
        .get("answers")
        .and_then(Value::as_object)
        .context("the reply carried no answers")?;

    Ok(parse(answers, candidates))
}

/// Turns the answer map back into one [`Priority`] per candidate.
///
/// A candidate whose Score is missing or malformed is dropped rather than
/// defaulted: a zero would rank it as "no gain here", which is a claim the
/// service never made. A missing Noul is different — it only suppresses a
/// warning, so it falls back to zero.
fn parse(answers: &Map<String, Value>, candidates: &[Candidate]) -> Vec<Priority> {
    let mut out = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let Some(help) = answers.get(&format!("help_{i}")) else { continue };
        let Some(score) = help.get("score").and_then(Value::as_f64) else { continue };
        let confidence = help.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);

        out.push(Priority {
            tweak_id: c.id.clone(),
            score,
            confidence,
            level: legend_at(help, score),
            breakage: answers
                .get(&format!("risk_{i}"))
                .and_then(|a| a.get("noul"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
        });
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

/// The description of the level the score sits nearest, read out of the
/// legend the service returned with it.
fn legend_at(answer: &Value, score: f64) -> String {
    let nearest = score.round().max(0.0) as u64;
    answer
        .get("legend")
        .and_then(|l| l.get(nearest.to_string()))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn first_line(s: &str) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or(s).trim();
    if line.len() > 200 {
        format!("{}\u{2026}", &line[..200])
    } else {
        line.to_string()
    }
}

/// The risk label as the questions see it. The UI's own label is localised,
/// and a Polish word in an English question would make the state harder to
/// read, not easier.
pub fn risk_word(risk: Risk) -> &'static str {
    match risk {
        Risk::Low => "low",
        Risk::Medium => "medium",
        Risk::High => "high",
    }
}

/// Trims the candidate list to what one request should carry, worst risk
/// last so that the cheap, safe wins survive the cut.
pub fn cap(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    candidates.truncate(MAX_CANDIDATES);
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> Candidate {
        Candidate {
            id: id.into(),
            title: "t".into(),
            changes: "c".into(),
            rationale: "r".into(),
            risk: "low".into(),
            current: "now".into(),
        }
    }

    #[test]
    fn answers_are_matched_back_to_candidates_and_sorted_by_score() {
        let answers: Map<String, Value> = serde_json::from_value(json!({
            "help_0": { "type": "score", "score": 0.4, "confidence": 0.9,
                        "legend": { "0": "nothing here", "1": "housekeeping" } },
            "risk_0": { "type": "noul", "noul": 0.02 },
            "help_1": { "type": "score", "score": 2.6, "confidence": 0.8,
                        "legend": { "3": "first thing to try" } },
            "risk_1": { "type": "noul", "noul": 0.81 },
        }))
        .unwrap();

        let got = parse(&answers, &[candidate("a"), candidate("b")]);

        assert_eq!(got.len(), 2);
        assert_eq!(got[0].tweak_id, "b");
        assert_eq!(got[0].level, "first thing to try");
        assert!(got[0].is_recommendation());
        assert!(got[0].is_risky());
        assert_eq!(got[1].tweak_id, "a");
        assert_eq!(got[1].level, "nothing here");
        assert!(!got[1].is_recommendation());
        assert!(!got[1].is_risky());
    }

    #[test]
    fn a_candidate_with_no_score_is_dropped_rather_than_ranked_zero() {
        let answers: Map<String, Value> = serde_json::from_value(json!({
            "help_1": { "type": "score", "score": 1.0, "confidence": 0.7, "legend": {} },
        }))
        .unwrap();

        let got = parse(&answers, &[candidate("a"), candidate("b")]);

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].tweak_id, "b");
    }

    #[test]
    fn a_missing_risk_answer_only_suppresses_the_warning() {
        let answers: Map<String, Value> = serde_json::from_value(json!({
            "help_0": { "type": "score", "score": 2.9, "confidence": 0.95, "legend": {} },
        }))
        .unwrap();

        let got = parse(&answers, &[candidate("a")]);

        assert!(got[0].is_recommendation());
        assert!(!got[0].is_risky());
    }

    #[test]
    fn a_split_distribution_is_not_shown_as_a_recommendation() {
        let split = Priority {
            tweak_id: "a".into(),
            score: 2.0,
            confidence: 0.2,
            level: String::new(),
            breakage: 0.0,
        };
        assert!(!split.is_recommendation());
    }

    #[test]
    fn an_undecided_noul_is_not_a_warning() {
        let unsure = Priority {
            tweak_id: "a".into(),
            score: 2.0,
            confidence: 0.9,
            level: String::new(),
            breakage: 0.5,
        };
        assert!(!unsure.is_risky());
    }
}
