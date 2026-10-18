use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    match youtube_rust::run(&youtube_rust::Options::parse()) {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result).expect("valid JSON")
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
