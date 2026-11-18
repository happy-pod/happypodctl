mod api;
mod cli;
mod commands;
mod config;
mod demo;
mod output;

use api::{ApiError, Client};
use clap::Parser;
use cli::{Cli, Command, DemoCmd};
use config::Resolved;
use serde_json::json;

fn make_client(cfg: &Resolved) -> Result<Client, ApiError> {
    if cfg.demo {
        return Ok(Client::new(
            &cfg.api_url,
            &cfg.api_key,
            Box::new(demo::Sim::new(config::demo_state_path())),
        ));
    }
    let http = api::Http::new().map_err(|e| ApiError::new(0, "CLIENT_ERROR", e))?;
    Ok(Client::new(&cfg.api_url, &cfg.api_key, Box::new(http)))
}

fn run(cli: Cli) -> Result<(), ApiError> {
    let demo_flag = cli.demo;
    let file = config::load().map_err(|e| ApiError::new(0, "CONFIG", e))?;
    let cfg = config::resolve(&file, demo_flag, cli.api_url.clone(), None);
    match cli.command {
        Command::Doctor(args) => {
            commands::doctor(&args, cfg.demo, cli.api_url, cli.output, make_client)
        }
        Command::Demo(DemoCmd::Reset) => {
            demo::Sim::new(config::demo_state_path())
                .reset()
                .map_err(|e| ApiError::new(0, "DEMO_STATE", e.to_string()))?;
            output::print(
                &json!({ "reset": true, "state": config::demo_state_path().display().to_string() }),
                cli.output,
                &[],
            );
            Ok(())
        }
        command => {
            let ctx = commands::Ctx {
                client: make_client(&cfg)?,
                cfg,
                format: cli.output,
            };
            commands::run(&ctx, command)
        }
    }
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!(
            "{}",
            serde_json::to_string(&e.to_json()).unwrap_or_else(|_| e.to_string())
        );
        std::process::exit(commands::exit_code(&e));
    }
}
