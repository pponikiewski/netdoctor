// No console window when launched from Explorer, but keep one for `cargo run`
// and for `--version`/`--report` on the command line.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// An `unsafe fn` is a promise the caller has to keep, not a licence for its
// body to do anything it likes. Without this, every line inside one of them is
// implicitly unsafe, so the 43 explicit `unsafe` blocks in this crate stop
// marking where the risk actually is. This crate is mostly Win32 FFI; the
// blocks are the map, and the map has to stay accurate.
#![deny(unsafe_op_in_unsafe_fn)]

mod autostart;
mod bandwidth;
mod cause;
mod diagnose;
mod i18n;
mod monitor;
mod optimize;
mod probe;
mod settings;
mod single;
mod store;
mod ui;
mod update;
mod winreg;

use std::sync::Arc;

use eframe::egui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Everything the command line accepts. An argument outside this list is a
/// typo, and a typo used to be ignored — `--qiuck` quietly ran the full scan,
/// which saturates the line for a quarter of a minute.
const FLAGS: [&str; 7] = ["--scan", "--quick", "--minimised", "--version", "-V", "--help", "-h"];

fn main() -> eframe::Result<()> {
    install_panic_hook();
    // Clears what the previous update left behind. It can only happen here:
    // the file stays locked for as long as the process that was replaced is
    // running, so the process that replaced it does the sweeping.
    update::clean_old();

    let args: Vec<String> = std::env::args().skip(1).collect();

    let cfg = settings::Settings::load();
    // Before anything that produces text, including --help and --scan.
    i18n::set(cfg.effective_lang());

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("netdoctor {VERSION}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if let Some(bad) = args.iter().find(|a| !FLAGS.contains(&a.as_str())) {
        eprintln!("{}", i18n::cli_unknown_flag(bad));
        print_help();
        std::process::exit(2);
    }

    let store = match store::Store::open_default() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            // A scripted `--scan` has to be able to tell this apart from a
            // clean run, so it cannot exit 0.
            eprintln!("{} {e}", i18n::err_open_db());
            std::process::exit(1);
        }
    };

    // Headless scan, for scripting or for pasting into a ticket. `--quick`
    // drops the load test, which is the only part that takes real time and the
    // only part that saturates the line.
    if args.iter().any(|a| a == "--scan") {
        let deep = !args.iter().any(|a| a == "--quick");
        let net = probe::netstate::read();
        let scan = diagnose::scan(&net, &store, &cfg, deep, None);

        let v = &scan.verdict;
        println!("{}", i18n::verdict_heading());
        println!("  {} — {}", v.segment.label(), v.confidence.label());
        if let Some(split) = &v.split {
            println!("  {split}");
        }
        println!("  {}", v.cost);
        for (n, a) in v.actions.iter().enumerate() {
            println!("  {}. {}", n + 1, a.text);
        }

        println!("{}", "-".repeat(72));
        println!("{}", diagnose::summarise(&scan.findings));
        for f in &scan.findings {
            println!("[{:>8}] {} :: {}", f.severity.label(), f.title, f.detail);
            if !f.advice.is_empty() {
                println!("           -> {}", f.advice);
            }
        }
        return Ok(());
    }

    // Only the windowed run takes the guard. `--scan` writes nothing to the
    // history and is the one thing a script might sensibly run while the app
    // is up.
    //
    // Held in a binding rather than dropped straight away: the name is only
    // taken for as long as this handle lives.
    let _instance = match single::acquire() {
        Some(guard) => guard,
        None => {
            // Not an error to report. Somebody launched the app that is
            // already running — most often because autostart started it
            // hidden — so the useful answer is the window they were looking
            // for, not a message box.
            single::raise_existing_window();
            return Ok(());
        }
    };

    let minimised = args.iter().any(|a| a == "--minimised") || cfg.start_minimised;

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([1120.0, 760.0])
        .with_min_inner_size([900.0, 620.0])
        .with_title(format!("NetDoctor {VERSION} · {}", i18n::app_tagline()))
        .with_visible(!minimised);

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "NetDoctor",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, store, cfg)))),
    )
}

fn print_help() {
    println!("netdoctor {VERSION} · {}\n\n{}", i18n::app_tagline(), i18n::cli_help());
}

/// The release build has no console (`windows_subsystem = "windows"`) and
/// aborts on panic, so without this a crash is an application that simply
/// vanishes. A diagnostics tool that cannot say why it died has it the wrong
/// way round: write the reason next to the database and point at the file.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let dir = settings::data_dir();
        let path = dir.join("crash.log");
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let where_ = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".into());
        let line = format!("netdoctor {VERSION} · unix {when} · {where_}\n{info}\n\n");

        let _ = std::fs::create_dir_all(&dir);
        let wrote = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()))
            .is_ok();

        previous(info);
        if wrote {
            show_crash_notice(&path.display().to_string());
        }
    }));
}

#[cfg(windows)]
fn show_crash_notice(path: &str) {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let wide = |s: &str| -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    };
    let text = wide(&i18n::crash_notice(path));
    let title = wide("NetDoctor");
    unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn show_crash_notice(path: &str) {
    eprintln!("{}", i18n::crash_notice(path));
}

#[cfg(test)]
mod tests {
    /// The README quotes a test count, and a quoted number rots the moment
    /// someone adds a test. It sat at "43" while the suite grew to 213, which
    /// is worse than no number at all: a reader who checks it once and finds
    /// it wrong stops trusting the rest of the page.
    ///
    /// Counting attributes in the tree rather than asking the test harness is
    /// deliberate — a harness can only report the tests it was asked to run,
    /// so `--ignored` ones would go missing and the number would drift again.
    fn test_attributes_in_source() -> usize {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else { return };
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }

        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);

        // Split so this file does not contain the literal it is looking for,
        // which would make the scanner count itself.
        let needle = concat!("#[", "test]");
        files
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.matches(needle).count())
            .sum()
    }

    /// The number the README prints, from `"<n> tests,"`.
    fn count_claimed_by_readme() -> Option<usize> {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"),
        )
        .ok()?;
        let at = text.find(" tests, covering")?;
        let digits: String = text[..at].chars().rev().take_while(|c| c.is_ascii_digit()).collect();
        digits.chars().rev().collect::<String>().parse().ok()
    }

    #[test]
    fn the_readme_still_quotes_the_right_number_of_tests() {
        let actual = test_attributes_in_source();

        // A scanner that matched nothing would agree with any README, so it
        // has to prove it read the tree before its answer means anything.
        assert!(actual > 100, "the scanner only found {actual} tests, so it did not read the tree");

        let claimed = count_claimed_by_readme().expect("README does not state a test count");
        assert_eq!(
            claimed, actual,
            "README says {claimed} tests, the tree has {actual}. Update the README."
        );
    }

    /// Three documents shipped with an unfilled template prompt in them, one of
    /// them the first paragraph of the file that is supposed to explain what
    /// this project even is. Nobody notices, because a template comment reads
    /// as furniture rather than as a gap.
    ///
    /// ponytail: matches the template's own imperative, not emptiness. A
    /// section left blank without one of these comments still slips through,
    /// and that is the right trade while every placeholder here comes from the
    /// same template — an emptiness check needs a list of the sections that
    /// are allowed to be empty, and that list rots the same way.
    #[test]
    fn no_document_still_carries_an_unfilled_placeholder() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut docs = vec![root.join("CLAUDE.md"), root.join("README.md")];
        let entries = std::fs::read_dir(root.join("docs")).expect("docs/ is missing");
        docs.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md")),
        );

        // Fewer files than that means the walk found nothing and every
        // assertion below would pass on an empty list.
        assert!(docs.len() >= 5, "only looked at {} documents", docs.len());

        let mut unfilled = Vec::new();
        for doc in &docs {
            let Ok(text) = std::fs::read_to_string(doc) else { continue };
            for (n, line) in text.lines().enumerate() {
                let lower = line.to_lowercase();
                // Both spellings: the mutation check showed that an editor
                // typing without Polish diacritics walks straight past a
                // match on the accented form alone.
                let marks = lower.contains("uzupeł") || lower.contains("uzupel");
                if lower.contains("<!--") && marks {
                    unfilled.push(format!("{}:{}", doc.display(), n + 1));
                }
            }
        }

        assert!(unfilled.is_empty(), "template placeholders still unfilled: {unfilled:?}");
    }
}
