//! Host utility: read built unit DLLs' PE export directories and require
//! that each exports exactly `init` and `shutdown` (the Crux extension
//! unit ABI; no proxy or `DllMain`-started entry points).
//!
//! `cargo run -p workshop-ext-sdk --example export_check -- a.dll [b.dll ...]`
use std::process::ExitCode;

fn main() -> ExitCode {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: export_check UNIT.dll [UNIT.dll ...]");
        return ExitCode::FAILURE;
    }
    let mut ok = true;
    for path in paths {
        let shown = path.to_string_lossy();
        match std::fs::read(&path)
            .ok()
            .and_then(|b| workshop_ext_sdk::pe::exports(&b))
        {
            Some(names) if workshop_ext_sdk::pe::is_unit(&names) => {
                println!("export_check: {shown} exports exactly init, shutdown");
            }
            other => {
                eprintln!("export_check: {shown}: unexpected exports {other:?}");
                ok = false;
            }
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
