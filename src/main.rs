use clap::Parser;
use directories::ProjectDirs;
use log::{error, info, warn};
use nudge::Nudge;
use serde::Deserialize;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Command;

/// nudge configuration
#[derive(Debug, Deserialize, Default)]
struct Config {
    database: Option<PathBuf>,
    whisper_path: Option<PathBuf>,
    model: Option<String>,
    language: Option<String>,
}

/// Find config directory and load config
fn load_config() -> Config {
    // Find config directory
    let config_dir = if let Some(proj_dirs) = ProjectDirs::from("com", "nudge", "nudge") {
        proj_dirs.config_dir().to_path_buf()
    } else {
        // Fallback to current directory
        PathBuf::from(".")
    };

    let config_path = config_dir.join("config.toml");

    if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(contents) => {
                match toml::from_str(&contents) {
                    Ok(config) => {
                        info!("Loaded config from {:?}", config_path);
                        return config;
                    }
                    Err(e) => {
                        warn!("Failed to parse config: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("Failed to read config file: {}", e);
            }
        }
    }

    Config::default()
}

/// Get default database path
fn default_db_path() -> PathBuf {
    if let Some(proj_dirs) = ProjectDirs::from("com", "nudge", "nudge") {
        let data_dir = proj_dirs.data_dir();
        // Create directory if it doesn't exist
        let _ = fs::create_dir_all(data_dir);
        data_dir.join("nudge.db")
    } else {
        PathBuf::from("nudge.db")
    }
}

/// Translate audio to text using Whisper with context-aware corrections
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Audio file to process (optional if using stdin)
    #[arg(default_value = "-")]
    input: String,

    /// Agent response to store as context for future translations
    #[arg(short, long)]
    response: Option<String>,

    /// Whisper model to use (tiny, base, small, medium, large)
    #[arg(short, long)]
    model: Option<String>,

    /// Language code (en, es, fr, de, etc.)
    #[arg(short, long)]
    language: Option<String>,

    /// Path to SQLite database
    #[arg(short, long)]
    database: Option<PathBuf>,

    /// Path to whisper binary
    #[arg(long)]
    whisper_path: Option<PathBuf>,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            writeln!(
                buf,
                "[{} {} {}] {}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.target(),
                record.args()
            )
        })
        .init();

    let args = Args::parse();

    // Set log level
    if args.verbose {
        std::env::set_var("RUST_LOG", "debug");
    }

    // Load config file
    let config = load_config();

    // Determine values: CLI args > config > defaults
    let db_path = args.database
        .or(config.database)
        .unwrap_or_else(default_db_path);

    let model = args.model
        .or(config.model)
        .unwrap_or_else(|| "base".to_string());

    let language = args.language
        .or(config.language)
        .unwrap_or_else(|| "en".to_string());

    let whisper_path = args.whisper_path.or(config.whisper_path);

    // Create nudge instance
    let nudge = match Nudge::new(&db_path) {
        Ok(n) => n,
        Err(e) => {
            error!("Failed to initialize nudge database: {}", e);
            std::process::exit(1);
        }
    };

    // Handle --response mode (store context, no output)
    if let Some(response) = args.response {
        info!("Storing agent response as context");
        match nudge.add_message(&response, "agent", None) {
            Ok(_) => {
                info!("Context stored successfully");
                std::process::exit(0);
            }
            Err(e) => {
                error!("Failed to store context: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Handle audio processing mode
    let audio_data = if args.input == "-" {
        // Read from stdin
        info!("Reading audio from stdin");
        let mut buffer = Vec::new();
        match io::stdin().read_to_end(&mut buffer) {
            Ok(_) => buffer,
            Err(e) => {
                error!("Failed to read stdin: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        // Read from file
        info!("Reading audio from file: {}", args.input);
        match fs::read(&args.input) {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to read audio file {}: {}", args.input, e);
                std::process::exit(1);
            }
        }
    };

    if audio_data.is_empty() {
        error!("No audio data provided");
        std::process::exit(1);
    }

    // Save audio to temp file for whisper (whisper needs a file)
    let temp_dir = std::env::temp_dir();
    let temp_audio = temp_dir.join(format!("nudge_audio_{}.ogg", std::process::id()));

    if let Err(e) = fs::write(&temp_audio, &audio_data) {
        error!("Failed to write temp audio file: {}", e);
        std::process::exit(1);
    }

    // Find whisper binary
    let whisper_bin = whisper_path
        .or_else(|| which::which("whisper").ok())
        .unwrap_or_else(|| PathBuf::from("whisper"));

    info!("Running whisper on audio file");
    let whisper_output = Command::new(&whisper_bin)
        .args([
            "-f", temp_audio.to_str().unwrap(),
            "--language", &language,
            "--model", &model,
            "--no-timestamps",
        ])
        .output();

    // Clean up temp file
    let _ = fs::remove_file(&temp_audio);

    let whisper_output = match whisper_output {
        Ok(output) => output,
        Err(e) => {
            error!("Failed to run whisper: {}", e);
            error!("Make sure whisper is installed: https://github.com/ggerganov/whisper.cpp");
            std::process::exit(1);
        }
    };

    if !whisper_output.status.success() {
        let stderr = String::from_utf8_lossy(&whisper_output.stderr);
        error!("Whisper failed: {}", stderr);
        std::process::exit(1);
    }

    // Get transcribed text
    let transcribed = String::from_utf8_lossy(&whisper_output.stdout).trim().to_string();

    if transcribed.is_empty() {
        warn!("Whisper returned empty transcription");
        // Still output empty string so the bot can handle it
        println!();
        std::process::exit(0);
    }

    info!("Transcribed: {}", transcribed);

    // Process with nudge for context-aware corrections
    let corrected = nudge.process(&transcribed);

    if args.verbose {
        if corrected != transcribed {
            info!("Corrected: {} -> {}", transcribed, corrected);
        } else {
            info!("No corrections needed");
        }
    }

    // Output corrected text to stdout
    println!("{}", corrected);
}
