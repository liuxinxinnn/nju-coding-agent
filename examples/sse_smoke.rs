use std::io::{self, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use nju_coding_agent::llm::{DeltaHandler, LanguageModel, Message};
use nju_coding_agent::{Config, Error, HttpLanguageModel, Result};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("SSE smoke failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let config = Config::from_env(PathBuf::from("."), 2, true)?;
    let model = HttpLanguageModel::new(&config.base_url, config.api_key, config.model.clone())?;
    let deltas = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = Arc::clone(&deltas);
    let handler: DeltaHandler = Arc::new(move |delta| {
        print!("{delta}");
        let _ = io::stdout().flush();
        if let Ok(mut values) = captured.lock() {
            values.push(delta.to_owned());
        }
    });
    let prompt = concat!(
        "请用一段不少于80个汉字的中文说明为什么编程智能体在修改代码后必须运行真实测试。",
        "只输出说明正文，不要使用工具，不要标题。"
    );

    let message = model
        .complete_stream(&[Message::user(prompt)], &[], handler)
        .await?;
    println!();

    let values = deltas
        .lock()
        .map_err(|_| Error::Agent("SSE delta buffer lock was poisoned".to_owned()))?;
    let streamed = values.concat();
    let final_content = message.content.as_deref().unwrap_or_default();
    let aggregate_matches = streamed == final_content;
    let truly_incremental = values.len() >= 2;

    println!("SSE model: {}", config.model);
    println!("SSE content delta events: {}", values.len());
    println!("SSE streamed characters: {}", streamed.chars().count());
    println!("SSE aggregate matches final message: {aggregate_matches}");
    println!(
        "SSE reasoning_content received: {}",
        message
            .reasoning_content
            .as_deref()
            .is_some_and(|value| !value.is_empty())
    );

    if !aggregate_matches {
        return Err(Error::Agent(
            "streamed deltas did not match the assembled message".to_owned(),
        ));
    }
    if !truly_incremental {
        return Err(Error::Agent(
            "only one content delta was observed; the request may have used non-stream fallback"
                .to_owned(),
        ));
    }
    if streamed.chars().count() < 80 {
        return Err(Error::Agent(
            "streamed response was shorter than the requested smoke-test minimum".to_owned(),
        ));
    }
    Ok(())
}
