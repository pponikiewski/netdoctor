// No console window when launched from Explorer, but keep one for `cargo run`
// and for `--version`/`--report` on the command line.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
