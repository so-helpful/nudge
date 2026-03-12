# nudge

A text processing binary that converts speech-to-text input into accurate technical text for developers.

## The Problem

Speaking about code is hard to transcribe. When you send voice messages about programming:

- "dot get" → Whisper writes "dog et"
- "s r c" → Whisper writes "source"
- "npm" → Whisper writes "n p m"
- "cargo" → Whisper writes "cargo.toml" (sometimes)

**nudge** solves this by using your conversation history to intelligently correct transcribed text.

## How It Works

```
Voice Message → Telegram → nudge → Whisper → nudge → Clean Text → Telegram
```

1. You send a voice message
2. nudge receives the audio (via middleware)
3. Whisper transcribes the audio
4. nudge processes the text using:
   - Built-in mappings for common tech terms
   - Your message history as context
   - Fuzzy matching for corrections
   - Learned mappings from previous corrections

## Features

- **Smart Term Matching**: Converts "dot get" → ".git", "s r c" → "src", "n p m" → "npm"
- **History-Aware**: Learns from your conversation context
- **Fuzzy Matching**: Handles misheard variations
- **Learns Over Time**: Stores mappings in SQLite, frequency-weighted
- **Middleware Ready**: Designed to run between Telegram and your chat

## Usage

### Running Tests

```bash
cargo test
```

### Using as a Library

```rust
use nudge::Nudge;
use std::path::PathBuf;

// Create nudge instance
let nudge = Nudge::new(&PathBuf::from("nudge.db")).unwrap();

// Load your message history
nudge.load_corpus(vec![
    ("Check the src directory".to_string(), "user".to_string()),
    ("Run cargo build".to_string(), "user".to_string()),
]).unwrap();

// Process transcribed text
let corrected = nudge.process("dot get repo");
assert_eq!(corrected, ".git repo");
```

### CLI Binary

```bash
# Process text directly
echo "dot get" | cargo run --quiet

# Or use the library programmatically for full middleware support
```

## Architecture

- **SQLite**: Stores message history and term mappings
- **Fuzzy Matching**: Uses `fuzzy-matcher` crate for approximate matching
- **Pattern Rules**: Regex-based transformations for common patterns

## Requirements

- Rust 2021 edition
- SQLite (bundled via rusqlite)
- [Whisper CLI](https://github.com/ggerganov/whisper.cpp) for speech-to-text

## Roadmap

- [ ] Telegram middleware integration
- [ ] Context-aware disambiguation
- [ ] Learning from corrections
- [ ] CLI with config file

## Example Mappings

| You Say | Whisper Writes | nudge Outputs |
|---------|---------------|---------------|
| dot get | "dot get" | `.git` |
| s r c | "source" | `src` |
| n p m | "n p m" | `npm` |
| node modules | "node modules" | `node_modules` |
| cargo | "cargo" | `Cargo.toml` |

## License

MIT
