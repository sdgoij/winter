//! Packages the extensions beside a source registry for GitHub Pages.
//!
//! Usage: `registry-publish <extensions-dir> <out-dir>`. The Pages job runs it
//! with `extensions` and the site directory, so the published registry's
//! checksums are computed from the archives in the same run (ADR 088).

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(source), Some(out)) = (args.next(), args.next()) else {
        eprintln!("usage: registry-publish <extensions-dir> <out-dir>");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("registry-publish: too many arguments");
        return ExitCode::from(2);
    }

    match registry::package::build(Path::new(&source), Path::new(&out)) {
        Ok(packaged) => {
            println!(
                "wrote {} and {} archive(s)",
                packaged.registry.display(),
                packaged.archives.len()
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("registry-publish: {error}");
            ExitCode::FAILURE
        }
    }
}
