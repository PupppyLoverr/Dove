//! A Luau app for the Linux probes: `luau_probe <dir>` runs the directory's
//! app.luau as `zgx-run` does (`zgx_run::run_with`), built with the probes'
//! other examples. `scripts/luau_views_smoke.py` runs `scripts/luau_views`.
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: luau_probe <app.luau-or-dir>");
        return ExitCode::from(2);
    };
    match zgx_run::run_with(std::path::Path::new(&path), Vec::new()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("luau_probe: {error}");
            ExitCode::FAILURE
        }
    }
}
