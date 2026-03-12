use clap::Parser;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::Command;

/// Simple Whisper transcription test tool - outputs raw Whisper text
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Audio file to process (optional if using stdin)
    #[arg(default_value = "-")]
    input: String,

    /// Whisper model to use (tiny, base, small, medium, large)
    #[arg(short, long, default_value = "base")]
    model: String,

    /// Language code (en, es, fr, de, etc.)
    #[arg(short, long, default_value = "en")]
    language: String,

    /// Path to whisper binary
    #[arg(long)]
    whisper_path: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    eprintln!("nudge-test: Running Whisper transcription only");

    // Handle audio input
    let audio_data = if args.input == "-" {
        // Read from stdin
        eprintln!("Reading audio from stdin...");
        let mut buffer = Vec::new();
        match io::stdin().read_to_end(&mut buffer) {
            Ok(_) => buffer,
            Err(e) => {
                eprintln!("Failed to read stdin: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Read from file
        eprintln!("Reading audio from file: {}", args.input);
        match fs::read(&args.input) {
            Ok(data) => data,
            Err(e) => {
                eprintln!("Failed to read audio file {}: {}", args.input, e);
                std::process::exit(1);
            }
        }
    };

    if audio_data.is_empty() {
        eprintln!("No audio data provided");
        std::process::exit(1);
    }

    // Save audio to temp file for whisper
    let temp_dir = std::env::temp_dir();
    let temp_audio = temp_dir.join(format!("nudge_test_audio_{}.ogg", std::process::id()));

    if let Err(e) = fs::write(&temp_audio, &audio_data) {
        eprintln!("Failed to write temp audio file: {}", e);
        std::process::exit(1);
    }

    // Find whisper binary
    let whisper_bin = args.whisper_path
        .or_else(|| which::which("whisper").ok())
        .unwrap_or_else(|| PathBuf::from("whisper"));

    eprintln!("Running Whisper (model: {}, language: {})", args.model, args.language);

    let whisper_output = Command::new(&whisper_bin)
        .args([
            "-f", temp_audio.to_str().unwrap(),
            "--language", &args.language,
            "--model", &args.model,
            "--no-timestamps",
        ])
        .output();

    // Clean up temp file
    let _ = fs::remove_file(&temp_audio);

    let whisper_output = match whisper_output {
        Ok(output) => output,
        Err(e) => {
            eprintln!("Failed to run whisper: {}", e);
            eprintln!("Make sure whisper is installed: https://github.com/ggerganov/whisper.cpp");
            std::process::exit(1);
        }
    };

    if !whisper_output.status.success() {
        let stderr = String::from_utf8_lossy(&whisper_output.stderr);
        eprintln!("Whisper failed: {}", stderr);
        std::process::exit(1);
    }

    // Output transcribed text only (no processing)
    let transcribed = String::from_utf8_lossy(&whisper_output.stdout).trim().to_string();

    // Output to stdout only (no logging)
    println!("{}", transcribed);
}
