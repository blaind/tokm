//! Native command-line interface for `tokm`.

mod app;
mod args;
mod output;

use std::{io, process::ExitCode};

use clap::Parser;

use crate::{app::execute, args::Args};

fn main() -> ExitCode {
    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(2);
            let _ = error.print();
            return ExitCode::from(exit_code);
        }
    };
    let result = match execute(&args) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("tokm: {error}");
            return ExitCode::from(error.exit_code());
        }
    };

    let stdout = io::stdout();
    if let Err(error) = output::write_report(stdout.lock(), &result, &args) {
        if error.kind() == io::ErrorKind::BrokenPipe {
            return ExitCode::SUCCESS;
        }
        eprintln!("tokm: cannot write output: {error}");
        return ExitCode::FAILURE;
    }

    if args.verbose {
        let stderr = io::stderr();
        if let Err(error) = output::write_verbose_skips(stderr.lock(), &result) {
            eprintln!("tokm: cannot write diagnostics: {error}");
            return ExitCode::FAILURE;
        }
    }

    if let Some(budget) = args.max_tokens
        && result.measured_tokens() > budget
    {
        let measured_tokens = result.measured_tokens();
        let stderr = io::stderr();
        if let Err(error) = output::write_budget_failure(stderr.lock(), measured_tokens, budget) {
            eprintln!("tokm: cannot write budget failure: {error}");
            return ExitCode::FAILURE;
        }
        return ExitCode::from(3);
    }

    ExitCode::SUCCESS
}
