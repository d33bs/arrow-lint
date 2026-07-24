use std::path::PathBuf;

use anyhow::Result;
use arrowlint_core::{format_packs, lint_paths, list_builtin_rules, LintConfig, OutputFormat};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "arrow-lint-rs")]
#[command(about = "Lint Apache Arrow datasets and related formats.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Lint {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = Output::Text)]
        output: Output,
    },
    Rules {
        #[arg(long, value_enum, default_value_t = Output::Text)]
        output: Output,
    },
    Formats {
        #[arg(long, value_enum, default_value_t = Output::Text)]
        output: Output,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Output {
    Text,
    Json,
    Sarif,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Lint {
            paths,
            config,
            output,
        } => {
            let config = match config {
                Some(path) => LintConfig::from_path(path)?,
                None => LintConfig::default(),
            };
            let fail_on = config.rules.fail_on;
            let report = lint_paths(&paths, config)?;
            print!("{}", report.render(output.into())?);
            if report.has_failure_at(fail_on) {
                std::process::exit(1);
            }
        }
        Command::Rules { output } => match output {
            Output::Json | Output::Sarif => {
                println!("{}", serde_json::to_string_pretty(&list_builtin_rules())?);
            }
            Output::Text => {
                for rule in list_builtin_rules() {
                    println!(
                        "{} {:<30} {:<16} {}",
                        rule.id, rule.name, rule.category, rule.summary
                    );
                }
            }
        },
        Command::Formats { output } => match output {
            Output::Json | Output::Sarif => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&format_packs::known_format_packs())?
                );
            }
            Output::Text => {
                for pack in format_packs::known_format_packs() {
                    println!(
                        "{:<10} {:<16} {:<20} {}",
                        pack.name,
                        pack.status,
                        pack.rule_pack,
                        pack.best_practice_focus.join(", ")
                    );
                }
            }
        },
    }
    Ok(())
}

impl From<Output> for OutputFormat {
    fn from(value: Output) -> Self {
        match value {
            Output::Text => Self::Text,
            Output::Json => Self::Json,
            Output::Sarif => Self::Sarif,
        }
    }
}
