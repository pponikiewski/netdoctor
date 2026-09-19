// No console window when launched from Explorer, but keep one for `cargo run`
// and for `--version`/`--report` on the command line.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod bandwidth;
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

    // Headless scan, for scripting or for pasting into a ticket.
    if args.iter().any(|a| a == "--scan") {
        let net = probe::netstate::read();
        let findings = diagnose::scan(&net, &store, &cfg, None);
        println!("{}", diagnose::summarise(&findings));
        println!("{}", "-".repeat(72));
        for f in &findings {
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
        .with_title(format!("NetDoctor {VERSION} — {}", i18n::app_tagline()))
        .with_visible(!minimised);

    let options = eframe::NativeOptions { viewport, ..Default::default() };

    eframe::run_native(
        "NetDoctor",
        options,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, store, cfg)))),
    )
}

fn print_help() {
    println!("netdoctor {VERSION} — {}\n\n{}", i18n::app_tagline(), i18n::cli_help());
}
