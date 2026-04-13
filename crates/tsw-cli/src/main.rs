//! tsw-downloader — Linux CLI for downloading and verifying The Secret World.

mod args;
mod config_file;
mod init;

use anyhow::Result;
use clap::Parser;

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            let mut src = e.source();
            while let Some(s) = src {
                eprintln!("  caused by: {s}");
                src = s.source();
            }
            1
        }
    };
    std::process::exit(code);
}

fn run() -> Result<i32> {
    let cli = args::Cli::parse();

    init_logging(cli.verbose, cli.quiet);

    match cli.command {
        args::Command::Init(init_args) => init::run(init_args, cli.config),
        args::Command::Install(_) => {
            println!("install: not yet implemented");
            Ok(0)
        }
        args::Command::Verify(_) => {
            println!("verify: not yet implemented");
            Ok(0)
        }
        args::Command::Uninstall(_) => {
            println!("uninstall: not yet implemented");
            Ok(0)
        }
    }
}

fn init_logging(verbose: u8, quiet: bool) {
    use env_logger::Builder;
    use log::LevelFilter;

    let level = if quiet {
        LevelFilter::Error
    } else {
        match verbose {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            _ => LevelFilter::Debug,
        }
    };

    Builder::new().filter_level(level).init();
}
