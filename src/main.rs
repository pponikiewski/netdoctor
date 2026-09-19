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
mod store;
mod ui;
mod winreg;

use std::sync::Arc;

use eframe::egui;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> eframe::Result<()> {
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

    let store = match store::Store::open_default() {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("{} {e}", i18n::err_open_db());
            return Ok(());
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
