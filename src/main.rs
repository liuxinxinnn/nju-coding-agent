use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nju_coding_agent::tools::default_registry;
use nju_coding_agent::{Agent, Config, HttpLanguageModel, Result};

#[derive(Debug, Parser)]
#[command(name = "nju-coding-agent")]
#[command(about = "A small framework-free coding agent")]
struct Cli {
    /// Workspace boundary for all file and command tools.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,

    /// Maximum model/tool loop steps for each user turn.
    #[arg(long, default_value_t = 20)]
    max_steps: usize,

    /// Automatically approve non-read-only commands (blocked commands remain blocked).
    #[arg(long)]
    yes: bool,

    /// Task to execute. If omitted, starts a multi-turn interactive session.
    task: Vec<String>,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::from_env(cli.workspace, cli.max_steps, cli.yes)?;
    let model = Arc::new(HttpLanguageModel::new(
        &config.base_url,
        config.api_key.clone(),
        config.model.clone(),
    )?);
    let tools = default_registry(config.workspace.clone(), config.auto_approve)?;
    let mut agent = Agent::new(model, tools, &config.workspace, config.max_steps);

    if !cli.task.is_empty() {
        let answer = agent.run_turn(&cli.task.join(" ")).await?;
        println!("{answer}");
        return Ok(());
    }

    println!("Workspace: {}", config.workspace.display());
    println!("Enter a task, or /quit to exit.");
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            break;
        }
        let input = input.trim();
        if input.eq_ignore_ascii_case("/quit") || input.eq_ignore_ascii_case("/exit") {
            break;
        }
        if input.is_empty() {
            continue;
        }
        match agent.run_turn(input).await {
            Ok(answer) => println!("{answer}"),
            Err(error) => eprintln!("turn failed: {error}"),
        }
    }
    Ok(())
}
