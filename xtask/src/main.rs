#![forbid(unsafe_code)]

mod file_sizes;
mod simdoc;

fn main() {
    if let Err(err) = run(std::env::args().collect()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let program = args.first().map(String::as_str).unwrap_or("xtask");
    match args.get(1).map(String::as_str) {
        Some("simdoc") => simdoc::run(args),
        Some("check-file-sizes") => file_sizes::run(),
        _ => Err(format!(
            "usage: {program} <simdoc [--check]|check-file-sizes>"
        )),
    }
}
