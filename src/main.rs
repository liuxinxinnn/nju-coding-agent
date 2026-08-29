use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nju_coding_agent::tools::default_registry;
use nju_coding_agent::tui;
use nju_coding_agent::{
    Agent, Config, HttpLanguageModel, MarkdownMemoryStore, MemoryProvider, Result,
};

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

    /// Use the simple line-oriented REPL instead of the full-screen TUI.
    #[arg(long)]
    plain: bool,

    /// Start with Plan mode disabled; verification after edits remains mandatory.
    #[arg(long)]
    no_plan: bool,

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
    let mut config = Config::from_env(cli.workspace, cli.max_steps, cli.yes)?;
    config.planning_enabled = !cli.no_plan;

    if cli.task.is_empty() && !cli.plain {
        return tui::run(config).await;
    }

    let mut agent = build_agent(&config)?;

    if !cli.task.is_empty() {
        let answer = agent.run_turn(&cli.task.join(" ")).await?;
        println!("{answer}");
        return Ok(());
    }

    run_plain_repl(&config, &mut agent).await
}

fn build_agent(config: &Config) -> Result<Agent> {
    let model = Arc::new(HttpLanguageModel::new(
        &config.base_url,
        config.api_key.clone(),
        config.model.clone(),
    )?);
    let tools = default_registry(config.workspace.clone(), config.auto_approve)?;
    let mut agent = Agent::new(
        model,
        tools,
        &config.workspace,
        config.max_steps,
        config.context_window_tokens,
    );
    let memory = MarkdownMemoryStore::open_default(&config.workspace)?;
    agent.set_memory_snapshot(&memory.snapshot()?);
    agent.set_planning_enabled(config.planning_enabled);
    Ok(agent)
}

async fn run_plain_repl(config: &Config, agent: &mut Agent) -> Result<()> {
    let memory = MarkdownMemoryStore::open_default(&config.workspace)?;
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
        agent.set_memory_snapshot(&memory.snapshot()?);
        match agent.run_turn(input).await {
            Ok(answer) => println!("{answer}"),
            Err(error) => eprintln!("turn failed: {error}"),
        }
    }
    Ok(())
}
