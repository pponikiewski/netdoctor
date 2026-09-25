//! An explanation of a finished scan, from a language model the user chose
//! and pays for.
//!
//! The scan's own verdict stays the answer. It is computed from the
//! measurements by rules that are tested; the model is not, and it is asked
//! to explain the evidence, not to overrule it. Everything here is off until
//! the user pastes their own OpenRouter key, and nothing leaves the machine
//! until they press the button.
//!
//! What is sent is the scan as the user sees it on screen, minus what would
//! identify them or their network: the Wi-Fi name and the access point's MAC
//! are replaced, and every public address other than the two anchors the scan
//! pings is masked. Private addresses (the router, a Pi-hole) stay, because
//! "your router is 192.168.1.1" is part of the explanation and names no one.

use std::net::Ipv4Addr;
use std::time::Duration;

use crate::diagnose::{self, LinkState, Scan};
use crate::i18n::{self, Lang};
use crate::probe::netstate::NetState;
use crate::settings::Settings;

const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
/// Cheap, fast and good enough to read a page of numbers. The user can name
/// any other OpenRouter model in the settings.
pub const DEFAULT_MODEL: &str = "deepseek/deepseek-v4-flash";
/// Read when the settings hold no key, so a developer can try the feature
/// without writing a key into a file.
const KEY_ENV: &str = "OPENROUTER_API_KEY";

/// The key to use, if there is one.
pub fn key(cfg: &Settings) -> Option<String> {
    let saved = cfg.ai_key.trim();
    if !saved.is_empty() {
        return Some(saved.to_string());
    }
    std::env::var(KEY_ENV).ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty())
}

pub fn model(cfg: &Settings) -> &str {
    let m = cfg.ai_model.trim();
    if m.is_empty() {
        DEFAULT_MODEL
    } else {
        m
    }
}

/// The model's answer, in the four parts the tab lays out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Explanation {
    /// Which link is at fault and how sure that is, in a sentence or two.
    pub problem: String,
    /// The readings it rests on.
    pub why: Vec<String>,
    /// What to do, most likely to help first.
    pub steps: Vec<String>,
    /// What the scan could not tell.
    pub unknown: Vec<String>,
}

/// Reads the model's reply into its parts. The prompt asks for JSON; a model
/// that answers in prose anyway gets its prose shown, cleaned of Markdown,
/// rather than an error for having ignored the format.
pub fn parse_explanation(text: &str) -> Explanation {
    let trimmed = text.trim();
    // Some models wrap JSON in a fenced block despite being asked not to.
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|t| t.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let json = body
        .find('{')
        .zip(body.rfind('}'))
        .and_then(|(a, b)| serde_json::from_str::<serde_json::Value>(&body[a..=b]).ok());
    if let Some(v) = json.filter(|v| v["problem"].is_string()) {
        let list = |key: &str| -> Vec<String> {
            v[key]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        return Explanation {
            problem: undash(v["problem"].as_str().unwrap_or_default().trim()),
            why: list("why").iter().map(|x| undash(x)).collect(),
            steps: list("steps").iter().map(|x| undash(x)).collect(),
            unknown: list("unknown").iter().map(|x| undash(x)).collect(),
        };
    }
    let plain: String = trimmed
        .lines()
        .map(|l| l.trim_start_matches('#').trim().replace("**", ""))
        .collect::<Vec<_>>()
        .join("\n");
    Explanation { problem: undash(&plain), ..Default::default() }
}

/// Long dashes the model writes despite being asked not to, as the colon
/// the rest of the app uses.
fn undash(text: &str) -> String {
    text.replace(" \u{2014} ", ": ")
        .replace(" \u{2013} ", ": ")
        .replace(['\u{2014}', '\u{2013}'], "-")
}

/// Ask the model about `scan`. Blocking; run it off the UI thread.
///
/// `Err` is a sentence for the user: what failed and, where the service said,
/// why. A failure is never dressed up as an answer.
pub fn explain(scan: &Scan, net: &NetState, cfg: &Settings) -> Result<Explanation, String> {
    let key = key(cfg).ok_or_else(|| i18n::ai_err_no_key().to_string())?;
    let body = serde_json::json!({
        "model": model(cfg),
        "temperature": 0.2,
        "max_tokens": 1400,
        "messages": [
            { "role": "system", "content": system_prompt(i18n::current()) },
            { "role": "user", "content": report(scan, net, cfg) },
        ],
    });

    // The read timeout is the one that matters: a model can take a while to
    // start answering, and a stalled connection should not hang the button.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(90))
        .build();
    let response = agent
        .post(ENDPOINT)
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .set("X-Title", "NetDoctor")
        .send_string(&body.to_string());

    let text = match response {
        Ok(r) => r.into_string().map_err(|e| i18n::ai_err_network(&e.to_string()))?,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().ok().and_then(|b| service_message(&b)).unwrap_or_default();
            return Err(i18n::ai_err_status(code, &detail));
        }
        Err(e) => return Err(i18n::ai_err_network(&e.to_string())),
    };
    answer_of(&text).map(|t| parse_explanation(&t))
}

/// The reply's text, or why there is none.
fn answer_of(body: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| i18n::ai_err_reply(&e.to_string()))?;
    // OpenRouter reports some failures inside a 200.
    if let Some(msg) = service_message(body) {
        return Err(i18n::ai_err_reply(&msg));
    }
    v["choices"][0]["message"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| i18n::ai_err_reply(i18n::ai_err_empty()))
}

/// The service's own explanation of an error, when the body carries one.
fn service_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    v["error"]["message"].as_str().map(str::to_string)
}

fn system_prompt(lang: Lang) -> String {
    let language = match lang {
        Lang::Pl => "Polish",
        Lang::En => "English",
    };
    format!(
        "You are a network engineer reading the result of a home connection diagnostic run \
         by a Windows app. The user is not technical. Write every string in {language}.\n\n\
         Rules:\n\
         - The app's verdict was computed from the measurements by tested rules. Do not \
           contradict it unless a measurement below plainly shows otherwise, and if you do, \
           name that measurement.\n\
         - Use only the data given. Quote the numbers you rely on. Never invent a reading, a \
           device model or a cause the data does not support.\n\
         - Anything marked not measured, not established or ignored ping is unknown, not \
           broken. Say what is unknown instead of guessing.\n\
         - Link times are cumulative round trips to the far end of each link, not the link \
           alone; \"this link added\" is what that link alone contributed.\n\
         - Every step must be something the user can do themselves at home (move a device, \
           plug in a cable, restart the router, change a setting in the app's Optimise tab, \
           scan again at another time), or a specific thing to tell the provider. Never tell \
           them to log into a device the provider manages (an antenna, a modem the provider \
           locked), to change firmware, or to use a tool the app does not have.\n\
         - The line kind, when given, is the user's own description of their connection. Read \
           the numbers against it: 35 ms is healthy on a radio or LTE line and slow on fibre.\n\n\
         Answer with one JSON object and nothing else: no Markdown, no code fence.\n\
         {{\"problem\": \"one or two sentences: which link is at fault and how sure that is\", \
         \"why\": [\"the readings this rests on, one short sentence each, with the numbers\"], \
         \"steps\": [\"what to do, most likely to help first, one short sentence each\"], \
         \"unknown\": [\"what the scan could not tell, and what would settle it\"]}}\n\
         At most four items in why and in steps, at most three in unknown, each under 25 words. \
         Do not use em dashes or en dashes; use a colon or a comma."
    )
}

/// The scan, as the text the model reads. Labels are English so the prompt
/// reads one way; values and the findings' own words are in the user's
/// language, exactly as the app shows them.
pub fn report(scan: &Scan, net: &NetState, cfg: &Settings) -> String {
    let m = &scan.measurements;
    let v = &scan.verdict;
    let mut out = String::new();
    let mut line = |s: String| {
        out.push_str(&s);
        out.push('\n');
    };

    line("APP VERDICT".into());
    line(format!("- Segment at fault: {}", v.segment.label()));
    line(format!("- Confidence: {}", v.confidence.label()));
    if let Some(split) = &v.split {
        line(format!("- Where the time goes: {split}"));
    }
    line(format!("- Cost to the user: {}", v.cost));
    for (i, a) in v.actions.iter().enumerate() {
        line(format!("- Suggested step {}: {}", i + 1, a.text));
    }

    line(String::new());
    line("CONNECTION".into());
    let kind = match cfg.line_kind {
        crate::settings::LineKind::Unknown => "not given".to_string(),
        k => k.label().to_string(),
    };
    line(format!("- Line kind (as the user described it): {kind}"));
    if let Some(down) = cfg.plan(false) {
        let up = cfg.plan(true).map_or_else(|| "?".to_string(), |u| format!("{u:.0}"));
        line(format!("- Plan speed: {down:.0} / {up} Mbps"));
    }
    line(format!("- Medium: {}", m.medium.label()));
    if let Some(pct) = m.signal_pct {
        line(format!("- Wi-Fi signal: {pct}%"));
    }
    if let Some(ch) = net.channel {
        line(format!("- Wi-Fi channel: {ch} ({})", net.band().unwrap_or("?")));
    }
    if net.link_speed_mbps > 0 {
        line(format!("- Adapter link rate: {} Mbps", net.link_speed_mbps));
    }

    line(String::new());
    line("LINKS (round trip to the far end of each)".into());
    if let Some(why) = &m.blind {
        line(format!("- No ping could be sent: {why}"));
    }
    let names = ["This PC -> router", "Router -> provider", "Provider -> internet"];
    for (name, link) in names.iter().zip(diagnose::chain(&scan.findings, m)) {
        let owner = if link.local { " (the user's own second router)" } else { "" };
        let state = match &link.state {
            LinkState::Measured(s) => {
                let ms = |x: Option<f64>| x.map(|x| format!("{x:.1}")).unwrap_or("?".into());
                let share = link.added_ms.map(|a| format!(", this link added {a:.0} ms"));
                format!(
                    "sent {}, lost {:.0}%, min {} / avg {} / max {} ms, jitter {} ms{}",
                    s.count,
                    s.loss_pct,
                    ms(s.min),
                    ms(s.avg),
                    ms(s.max),
                    ms(s.jitter),
                    share.unwrap_or_default()
                )
            }
            LinkState::Silent => "no answer, and nothing else got through".into(),
            LinkState::Filtered => "ignores ping, but traffic passes".into(),
            LinkState::Unknown => "not established".into(),
            LinkState::NotMeasured => "not measured".into(),
        };
        line(format!("- {name}{owner}: {state}"));
    }

    line(String::new());
    line("OTHER READINGS".into());
    let or_none = |x: Option<f64>| x.map(|x| format!("{x:.0} ms")).unwrap_or("not measured".into());
    line(format!("- DNS answer: {}", or_none(m.dns_ms)));
    line(format!(
        "- TCP connect to port 443: {}",
        if m.tcp_blocked { "refused or timed out".into() } else { or_none(m.tcp_ms) }
    ));
    if let Some((usual, n)) = m.baseline {
        line(format!("- Usual ping on this line (7-day median of {n} readings): {usual:.0} ms"));
    }
    if let (Some(own), Some(public)) = (m.dns_own_ms, m.dns_public_ms) {
        line(format!(
            "- DNS, best of 3: the connection's own resolvers {own:.0} ms, public 1.1.1.1 {public:.0} ms"
        ));
    }
    line(format!(
        "- IPv6: {}",
        match &m.ipv6 {
            Some(diagnose::Ipv6State::Works(ms)) => format!("works, connected in {ms:.0} ms"),
            Some(diagnose::Ipv6State::Absent) =>
                "no IPv6 route (normal, everything uses IPv4)".into(),
            Some(diagnose::Ipv6State::Broken) =>
                "an IPv6 route exists but connections time out".into(),
            Some(diagnose::Ipv6State::Failed(why)) => format!("could not be checked: {why}"),
            None => "not checked".into(),
        }
    ));
    if m.syslog.is_empty() {
        line("- Windows event log during the scan: no connection faults".into());
    }
    for e in &m.syslog {
        line(format!(
            "- Windows logged at {}: {}{}",
            diagnose::format_clock_s(e.ts),
            i18n::log_kind(e.kind),
            e.reason.map(|r| format!(" (reason {r})")).unwrap_or_default()
        ));
    }
    match &m.load {
        Some(l) => line(format!(
            "- Load test: idle {} ms, loaded {} ms, {} Mbps, grade {}{}",
            l.idle_avg.map(|x| format!("{x:.0}")).unwrap_or("?".into()),
            l.loaded_avg.map(|x| format!("{x:.0}")).unwrap_or("?".into()),
            l.mbps.map(|x| format!("{x:.0}")).unwrap_or("?".into()),
            l.grade_or_unknown().letter(),
            if l.error.is_empty() { String::new() } else { format!(" ({})", l.error) }
        )),
        None => line("- Load test: not run".into()),
    }

    if let Some(run) = &m.long {
        line(String::new());
        line(format!(
            "LONG MEASUREMENT ({} of {} s, every link probed once a second on one clock{})",
            run.ticks.len(),
            run.planned_s,
            if run.cancelled { ", stopped early by the user" } else { "" }
        ));
        for (name, s) in [("router", &run.gw), ("provider", &run.edge), ("internet", &run.net)] {
            match s {
                Some(s) => line(format!(
                    "- To the {name}: lost {:.1}%, avg {:.1} ms, max {:.1} ms, jitter {:.1} ms",
                    s.loss_pct,
                    s.avg.unwrap_or(0.0),
                    s.max.unwrap_or(0.0),
                    s.jitter.unwrap_or(0.0)
                )),
                None => line(format!("- To the {name}: never answered or not probed")),
            }
        }
        if run.episodes.is_empty() {
            line("- No lost seconds and no latency spikes.".into());
        }
        for e in &run.episodes {
            line(format!(
                "- At {} for {} s: {} starting {}{}",
                diagnose::format_clock_s(run.started_at + e.start_s as f64),
                e.len_s,
                match e.kind {
                    crate::longrun::Trouble::Loss => "packets lost",
                    crate::longrun::Trouble::Spike => "latency spike",
                },
                e.segment.label(),
                e.worst_ms.map(|w| format!(", {w:.0} ms above usual")).unwrap_or_default()
            ));
        }
    }

    line(String::new());
    line("INDIVIDUAL CHECKS (severity | title | detail | advice)".into());
    let hours: Vec<String> = m
        .outage_hours
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(h, n)| format!("{h:02}:00-{:02}:00 x{n}", (h + 1) % 24))
        .collect();
    if !hours.is_empty() {
        line(String::new());
        line(format!("OUTAGES BY HOUR STARTED (last 24 h, local time): {}", hours.join(", ")));
    }

    for f in &scan.findings {
        line(format!("- {} | {} | {} | {}", f.severity.label(), f.title, f.detail, f.advice));
    }

    redact(&out, net)
}

/// Remove what identifies the user: the network's name, the access point's
/// hardware address, and public addresses other than the scan's anchors.
fn redact(text: &str, net: &NetState) -> String {
    let mut out = text.to_string();
    // Longest first, and only names long enough not to eat ordinary words.
    for (secret, mask) in [(&net.bssid, "[BSSID]"), (&net.ssid, "[SSID]")] {
        if secret.trim().len() >= 3 {
            out = out.replace(secret.as_str(), mask);
        }
    }
    mask_public_ips(&out)
}

fn mask_public_ips(text: &str) -> String {
    let keep = [diagnose::ANCHOR, diagnose::ANCHOR_ALT];
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        match token.parse::<Ipv4Addr>() {
            Ok(a) if diagnose::is_public(a) && !keep.contains(&a) => out.push_str("[public IP]"),
            _ => out.push_str(token),
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || c == '.' {
            token.push(c);
        } else {
            // A sentence can end on an address: "reached 1.2.3.4."
            let trailing = token.ends_with('.');
            if trailing {
                token.pop();
            }
            flush(&mut token, &mut out);
            if trailing {
                out.push('.');
            }
            out.push(c);
        }
    }
    let trailing = token.ends_with('.');
    if trailing {
        token.pop();
    }
    flush(&mut token, &mut out);
    if trailing {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnose::{Finding, Measurements, Severity};

    #[test]
    fn public_addresses_are_masked_but_the_anchors_and_the_lan_are_not() {
        let t = "hop 83.12.4.1 then 1.1.1.1, router 192.168.1.1, cgnat 100.64.0.1.";
        assert_eq!(
            mask_public_ips(t),
            "hop [public IP] then 1.1.1.1, router 192.168.1.1, cgnat 100.64.0.1."
        );
        // Numbers that are not addresses are left alone.
        assert_eq!(mask_public_ips("12.5 ms, 0.0%, v1.3.0"), "12.5 ms, 0.0%, v1.3.0");
    }

    #[test]
    fn the_report_carries_no_network_name_or_access_point_address() {
        let net = NetState {
            ssid: "Kowalski_5G".into(),
            bssid: "aa:bb:cc:dd:ee:ff".into(),
            ..Default::default()
        };
        let mut f = Finding::new_for_test("wifi", Severity::Good);
        f.detail = "Intel: Kowalski_5G via aa:bb:cc:dd:ee:ff, edge 83.12.4.1".into();
        let scan = Scan { findings: vec![f], ..Scan::default() };
        let r = report(&scan, &net, &Settings::default());
        assert!(!r.contains("Kowalski"), "{r}");
        assert!(!r.contains("aa:bb"), "{r}");
        assert!(!r.contains("83.12.4.1"), "{r}");
        assert!(r.contains("[SSID]") && r.contains("[BSSID]"));
    }

    #[test]
    fn an_unmeasured_scan_is_reported_as_unmeasured() {
        let scan = Scan {
            measurements: Measurements { blind: Some("no ICMP".into()), ..Default::default() },
            ..Scan::default()
        };
        let r = report(&scan, &NetState::default(), &Settings::default());
        assert!(r.contains("No ping could be sent: no ICMP"));
        // The three links, DNS and TCP.
        assert_eq!(r.matches(": not measured").count(), 5, "{r}");
    }

    #[test]
    fn a_reply_is_its_text_and_a_failure_is_never_an_answer() {
        let ok = r#"{"choices":[{"message":{"content":"  The router.  "}}]}"#;
        assert_eq!(answer_of(ok).unwrap(), "The router.");
        assert!(answer_of(r#"{"error":{"message":"Insufficient credits"}}"#)
            .unwrap_err()
            .contains("Insufficient credits"));
        assert!(answer_of(r#"{"choices":[{"message":{"content":""}}]}"#).is_err());
        assert!(answer_of("not json").is_err());
    }

    /// Goes to the network: `cargo test -- --ignored refused_key`.
    #[test]
    #[ignore]
    fn a_refused_key_comes_back_as_a_refusal_not_an_answer() {
        let cfg = Settings { ai_key: "sk-or-v1-not-a-real-key".into(), ..Settings::default() };
        let err = explain(&Scan::default(), &NetState::default(), &cfg).unwrap_err();
        assert!(err.contains("401"), "{err}");
    }

    /// Runs a real quick scan of this machine and asks the model about it.
    /// Needs `OPENROUTER_API_KEY`; costs a fraction of a cent.
    /// `cargo test -- --ignored explains_a_real_scan --nocapture`
    #[test]
    #[ignore]
    fn explains_a_real_scan() {
        let net = crate::probe::netstate::read();
        let store = crate::store::Store::open_in_memory().unwrap();
        let cfg = Settings::default();
        let scan = crate::diagnose::scan(&net, &store, &cfg, false, None, None);
        println!("--- sent ---\n{}", report(&scan, &net, &cfg));
        let answer = explain(&scan, &net, &cfg).unwrap();
        println!("--- answer ---\n{answer:#?}");
        assert!(!answer.problem.is_empty());
    }

    #[test]
    fn an_answer_is_read_into_its_parts_and_prose_is_kept_readable() {
        let json =
            r#"{"problem":"The router.","why":["Loss 5%"],"steps":["Restart it"],"unknown":[]}"#;
        let e = parse_explanation(json);
        assert_eq!(e.problem, "The router.");
        assert_eq!(e.why, vec!["Loss 5%".to_string()]);
        assert_eq!(e.steps, vec!["Restart it".to_string()]);
        // Fenced, and with a sentence in front: still the object.
        let fenced = format!("```json\n{json}\n```");
        assert_eq!(parse_explanation(&fenced), e);
        assert_eq!(parse_explanation(&format!("Here you go: {json}")), e);
        // Prose instead of JSON is shown, without the Markdown marks.
        let prose = parse_explanation("## The problem\n**The router** is at fault.");
        assert_eq!(prose.problem, "The problem\nThe router is at fault.");
        assert!(prose.steps.is_empty());
        // Dashes the model was told not to write come back as the app's colon.
        let dashed = r#"{"problem":"Wi-Fi \u2013 not the line","why":[],"steps":[],"unknown":[]}"#;
        assert_eq!(parse_explanation(dashed).problem, "Wi-Fi: not the line");
    }

    #[test]
    fn no_key_means_no_request() {
        let cfg = Settings { ai_key: "  ".into(), ..Settings::default() };
        if std::env::var(KEY_ENV).is_err() {
            assert!(explain(&Scan::default(), &NetState::default(), &cfg).is_err());
        }
    }
}
