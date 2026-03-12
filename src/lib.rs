use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use regex::Regex;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Normalize text for matching - lowercase, remove special chars
#[allow(dead_code)]
fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Common speech-to-text mappings for tech terms
fn get_common_mappings() -> Vec<( &'static str, &'static str)> {
    vec![
        // Git
        ("dot get", ".git"),
        ("git dir", ".git"),
        ("get repo", "git repo"),
        ("git repo", "git repo"),
        ("git hub", "GitHub"),
        
        // Source
        ("source", "src"),
        ("s r c", "src"),
        ("source code", "src"),
        
        // NPM/Node
        ("n p m", "npm"),
        ("node package manager", "npm"),
        ("node modules", "node_modules"),
        ("node modules", "node_modules"),
        
        // Rust
        ("cargo", "Cargo"),
        ("cargo", "Cargo.toml"),
        ("cargo", "Cargo.lock"),
        ("rust", "Rust"),
        
        // File extensions
        ("dot js", ".js"),
        ("dot ts", ".ts"),
        ("dot rs", ".rs"),
        ("dot python", ".py"),
        ("dot json", ".json"),
        ("dot yamel", ".yaml"),
        ("dot yamel", ".yml"),
        ("dot md", ".md"),
        ("dot to ml", "Cargo.toml"),
        
        // Common commands
        ("run", "cargo run"),
        ("build", "cargo build"),
        ("test", "cargo test"),
        ("check", "cargo check"),
        ("clippy", "cargo clippy"),
        
        // Directories
        ("bin", "bin"),
        ("lib", "lib"),
        ("target", "target"),
        ("docs", "docs"),
        ("tests", "tests"),
        
        // Version control
        ("push", "git push"),
        ("pull", "git pull"),
        ("commit", "git commit"),
        ("branch", "git branch"),
        ("checkout", "git checkout"),
        
        // Misc
        ("api", "API"),
        ("cli", "CLI"),
        ("sdk", "SDK"),
        ("ide", "IDE"),
        ("todo", "TODO"),
        ("readme", "README"),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub text: String,
    pub speaker: String,
    pub timestamp: i64,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermMapping {
    pub id: i64,
    pub spoken: String,
    pub canonical: String,
    pub context: Option<String>,
    pub frequency: i32,
}

pub struct Nudge {
    conn: Connection,
    fuzzy_matcher: SkimMatcherV2,
    /// Multiple mappings per spoken term, keyed by context
    mappings: HashMap<String, Vec<(String, String)>>, // spoken -> [(context, canonical), ...]
}

impl Nudge {
    pub fn new(db_path: &PathBuf) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;
        
        // Create tables
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT NOT NULL,
                speaker TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                context TEXT
            )",
            [],
        )?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS term_mappings (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                spoken TEXT NOT NULL,
                canonical TEXT NOT NULL,
                context TEXT,
                frequency INTEGER DEFAULT 1,
                UNIQUE(spoken, canonical)
            )",
            [],
        )?;
        
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_mappings_spoken ON term_mappings(spoken)",
            [],
        )?;
        
        let mut nudge = Self {
            conn,
            fuzzy_matcher: SkimMatcherV2::default(),
            mappings: HashMap::new(),
        };
        
        // Load common mappings
        nudge.load_common_mappings()?;
        
        Ok(nudge)
    }
    
    /// Load common speech-to-text mappings
    pub fn load_common_mappings(&mut self) -> Result<(), rusqlite::Error> {
        for (spoken, canonical) in get_common_mappings() {
            let spoken_lower = spoken.to_lowercase();

            // Determine context from the canonical form
            let context = Self::infer_context(canonical);

            // Store in database
            self.conn.execute(
                "INSERT OR IGNORE INTO term_mappings (spoken, canonical, context, frequency)
                 VALUES (?1, ?2, ?3, 1)",
                params![spoken_lower, canonical, context],
            )?;

            // Keep in memory with context - insert into vec
            self.mappings
                .entry(spoken_lower)
                .or_insert_with(Vec::new)
                .push((context.unwrap_or_else(|| "general".to_string()), canonical.to_string()));
        }

        Ok(())
    }

    /// Infer context from a canonical term
    fn infer_context(canonical: &str) -> Option<String> {
        let lower = canonical.to_lowercase();
        if lower.contains("git") {
            Some("git".to_string())
        } else if lower.contains("npm") || lower.contains("node") {
            Some("npm".to_string())
        } else if lower.contains("cargo") || lower.contains("rust") {
            Some("cargo".to_string())
        } else {
            Some("general".to_string())
        }
    }

    /// Detect context from recent messages
    pub fn detect_context(&self, limit: usize) -> String {
        // Get keywords to look for
        let keywords = [
            ("git", "git"),
            ("npm", "npm"),
            ("node", "npm"),
            ("cargo", "cargo"),
            ("rust", "cargo"),
            ("docker", "docker"),
            ("k8s", "kubernetes"),
            ("kubernetes", "kubernetes"),
        ];

        if let Ok(messages) = self.get_recent_messages(limit) {
            let mut context_counts: HashMap<String, i32> = HashMap::new();

            // Weight by recency: more recent messages count more
            for (idx, msg) in messages.iter().enumerate() {
                // Weight: newest message gets weight = limit, decreases linearly
                let weight = (limit - idx) as i32;
                let text_lower = msg.text.to_lowercase();
                for (keyword, context) in &keywords {
                    if text_lower.contains(keyword) {
                        *context_counts.entry(context.to_string()).or_insert(0) += weight;
                    }
                }
            }

            // Return the most common context (by weighted count)
            if let Some((context, _)) = context_counts.into_iter().max_by_key(|(_, c)| *c) {
                return context;
            }
        }

        "general".to_string()
    }
    
    /// Add a message to history
    pub fn add_message(&self, text: &str, speaker: &str, context: Option<&str>) -> Result<i64, rusqlite::Error> {
        let timestamp = chrono::Utc::now().timestamp();
        
        self.conn.execute(
            "INSERT INTO messages (text, speaker, timestamp, context) VALUES (?1, ?2, ?3, ?4)",
            params![text, speaker, timestamp, context],
        )?;
        
        Ok(self.conn.last_insert_rowid())
    }
    
    /// Load message history from a corpus
    pub fn load_corpus(&mut self, messages: Vec<(String, String)>) -> Result<(), rusqlite::Error> {
        // messages: (text, speaker)
        for (text, speaker) in messages {
            self.add_message(&text, &speaker, None)?;
        }

        // Extract terms from messages and create mappings
        self.extract_terms_from_history()?;

        Ok(())
    }
    
    fn extract_terms_from_history(&mut self) -> Result<(), rusqlite::Error> {
        // Simple extraction: find capitalized words, file paths, command patterns, etc.
        let path_regex = Regex::new(r"(/|\\)(\w+)").unwrap();
        let caps_regex = Regex::new(r"\b[A-Z]{2,}\b").unwrap();
        // Match command patterns like "npm run", "git push", "cargo build"
        let command_regex = Regex::new(r"\b(npm|cargo|git|pnpm|yarn|docker)\s+(\w+)\b").unwrap();

        let mut stmt = self.conn.prepare("SELECT id, text FROM messages")?;
        let messages: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();

        for (_id, text) in messages {
            // Extract command patterns: "npm run" -> "run" -> "npm run"
            for cap in command_regex.captures_iter(&text) {
                if let Some(tool) = cap.get(1) {
                    if let Some(cmd) = cap.get(2) {
                        let tool_str = tool.as_str();
                        let cmd_str = cmd.as_str();
                        let canonical = format!("{} {}", tool_str, cmd_str);
                        let spoken = cmd_str.to_lowercase();

                        // Add mapping with context
                        self.conn.execute(
                            "INSERT OR REPLACE INTO term_mappings (spoken, canonical, context, frequency)
                             VALUES (?1, ?2, ?3, 1)",
                            params![spoken, canonical, tool_str],
                        )?;

                        // Also update in-memory mappings
                        self.mappings
                            .entry(spoken)
                            .or_insert_with(Vec::new)
                            .push((tool_str.to_string(), canonical));
                    }
                }
            }

            // Extract paths
            for cap in path_regex.captures_iter(&text) {
                if let Some(path) = cap.get(1) {
                    let path_str = path.as_str();
                    // Add common variations
                    let lower = path_str.to_lowercase();
                    self.conn.execute(
                        "INSERT OR IGNORE INTO term_mappings (spoken, canonical, context) VALUES (?1, ?2, ?3)",
                        params![lower, path_str, "general"],
                    )?;
                }
            }

            // Extract acronyms
            for cap in caps_regex.captures_iter(&text) {
                if let Some(acronym) = cap.get(0) {
                    let acr_str = acronym.as_str();
                    self.conn.execute(
                        "INSERT OR IGNORE INTO term_mappings (spoken, canonical, context) VALUES (?1, ?2, ?3)",
                        params![acr_str.to_lowercase(), acr_str, "general"],
                    )?;
                }
            }
        }

        Ok(())
    }

    /// Process transcribed text and return corrected version
    pub fn process(&self, input: &str) -> String {
        self.process_with_context(input, None)
    }

    /// Process with explicit context override
    pub fn process_with_context(&self, input: &str, explicit_context: Option<&str>) -> String {
        // Determine context: use explicit or detect from history
        let context = explicit_context
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.detect_context(5));

        let input_lower = input.to_lowercase();
        let mut result = input.to_string();

        // Step 1: Check direct context-aware mappings from database (deterministic order)
        if let Some(canonical) = self.find_exact_match_with_context(&input_lower, &context) {
            return canonical;
        }

        // Step 2: Try fuzzy match from database with context
        if let Some((_, canonical)) = self.find_best_match(&input_lower, Some(&context)) {
            result = canonical.clone();
            return result;
        }

        // Step 3: Apply pattern rules only if no mapping found
        result = self.apply_pattern_rules(&result);

        // Step 4: Try mappings again on the transformed input
        let transformed_lower = result.to_lowercase();
        if let Some(mappings) = self.mappings.get(&transformed_lower) {
            for (mapping_context, canonical) in mappings {
                if mapping_context == &context || mapping_context == "general" {
                    result = canonical.clone();
                    return result;
                }
            }
        }

        result
    }

    /// Find best matching term using fuzzy matching with context
    fn find_best_match(&self, input: &str, context: Option<&str>) -> Option<(i64, String)> {
        let mut best_score: i64 = 0;
        let mut best_canonical: Option<String> = None;

        // Query with context preference
        let query = if context.is_some() {
            "SELECT id, spoken, canonical, context FROM term_mappings
             WHERE context = ?1 OR context = 'general'
             ORDER BY CASE WHEN context = ?1 THEN 0 ELSE 1 END"
        } else {
            "SELECT id, spoken, canonical, context FROM term_mappings"
        };

        let mut stmt = self.conn.prepare(query).ok()?;

        let mappings: Vec<(i64, String, String, String)> = if let Some(ctx) = context {
            stmt.query_map(params![ctx], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            stmt.query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .ok()?
            .filter_map(|r| r.ok())
            .collect()
        };

        for (_id, spoken, canonical, _mapping_context) in mappings {
            // Use fuzzy matcher
            if let Some(score) = self.fuzzy_matcher.fuzzy_match(&input, &spoken) {
                if score > best_score && score > 50 {
                    best_score = score;
                    best_canonical = Some(canonical);
                }
            }
        }

        best_canonical.map(|c| (best_score, c))
    }

    /// Find exact match with context from database (deterministic ordering)
    fn find_exact_match_with_context(&self, input: &str, context: &str) -> Option<String> {
        // Query with deterministic ordering: exact context first, then general, then any
        let query = "SELECT canonical FROM term_mappings
                     WHERE spoken = ?1
                     ORDER BY CASE WHEN context = ?2 THEN 0
                                   WHEN context = 'general' THEN 1
                                   ELSE 2 END
                     LIMIT 1";

        let mut stmt = self.conn.prepare(query).ok()?;

        let result = stmt.query_row(params![input, context], |row| row.get(0));
        result.ok()
    }
    
    /// Apply pattern-based rules
    fn apply_pattern_rules(&self, text: &str) -> String {
        let mut result = text.to_string();
        
        // "dot X" -> ".X"
        let dot_regex = Regex::new(r"\bdot\s+(\w+)\b").unwrap();
        result = dot_regex.replace_all(&result, |caps: &regex::Captures| {
            let word = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            format!(".{}", word)
        }).to_string();
        
        // "X dot Y" -> "X.Y"
        let dot_between = Regex::new(r"\b(\w+)\s+dot\s+(\w+)\b").unwrap();
        result = dot_between.replace_all(&result, "$1.$2").to_string();
        
        // "s r c" -> "src" (spaced letters)
        let spaced_letters = Regex::new(r"\b(\w)\s+(\w)\s+(\w)\b").unwrap();
        result = spaced_letters.replace_all(&result, |caps: &regex::Captures| {
            format!("{}{}{}", 
                caps.get(1).map(|m| m.as_str()).unwrap_or(""),
                caps.get(2).map(|m| m.as_str()).unwrap_or(""),
                caps.get(3).map(|m| m.as_str()).unwrap_or("")
            )
        }).to_string();
        
        result
    }
    
    /// Get message history for context
    pub fn get_recent_messages(&self, limit: usize) -> Result<Vec<Message>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT id, text, speaker, timestamp, context
             FROM messages
             ORDER BY timestamp DESC, id DESC
             LIMIT ?1"
        )?;
        
        let messages = stmt
            .query_map(params![limit as i64], |row| {
                Ok(Message {
                    id: row.get(0)?,
                    text: row.get(1)?,
                    speaker: row.get(2)?,
                    timestamp: row.get(3)?,
                    context: row.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        
        Ok(messages)
    }
    
    /// Add a custom mapping (learned)
    pub fn learn_mapping(&mut self, spoken: &str, canonical: &str, context: Option<&str>) -> Result<(), rusqlite::Error> {
        let spoken_lower = spoken.to_lowercase();
        let ctx = context.unwrap_or("general");

        self.conn.execute(
            "INSERT INTO term_mappings (spoken, canonical, context, frequency)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(spoken, canonical) DO UPDATE SET frequency = frequency + 1",
            params![spoken_lower, canonical, ctx],
        )?;

        // Update in-memory mappings
        self.mappings
            .entry(spoken_lower)
            .or_insert_with(Vec::new)
            .push((ctx.to_string(), canonical.to_string()));

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    
    #[test]
    fn test_basic_matching() {
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let nudge = Nudge::new(&db_path).unwrap();

        // Test direct mappings
        assert_eq!(nudge.process("dot get"), ".git");
        assert_eq!(nudge.process("git repo"), "git repo");
        assert_eq!(nudge.process("node modules"), "node_modules");
    }
    
    #[test]
    fn test_pattern_rules() {
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();
        
        let nudge = Nudge::new(&db_path).unwrap();
        
        // Test "dot X" -> ".X"
        assert_eq!(nudge.process("dot js"), ".js");
        assert_eq!(nudge.process("dot ts"), ".ts");
        assert_eq!(nudge.process("dot rs"), ".rs");
        
        // Test spaced letters
        assert_eq!(nudge.process("s r c"), "src");
    }
    
    #[test]
    fn test_load_corpus() {
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // Load some corpus
        let messages = vec![
            ("Let's check the src directory".to_string(), "user".to_string()),
            ("Run cargo build".to_string(), "user".to_string()),
            ("Check the README".to_string(), "user".to_string()),
        ];

        nudge.load_corpus(messages).unwrap();
        
        // Should have extracted terms
        let messages = nudge.get_recent_messages(10).unwrap();
        assert!(!messages.is_empty());
    }
    
    #[test]
    fn test_fuzzy_matching() {
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // Add custom mapping
        nudge.learn_mapping("dot j s", ".js", Some("programming")).unwrap();

        // Should match
        let result = nudge.process("dot j s");
        assert_eq!(result, ".js");
    }

    #[test]
    fn test_context_aware_git() {
        // When conversation is about git, "push" should map to "git push"
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // Load git-related corpus to set context
        let messages = vec![
            ("Let's git push the changes".to_string(), "user".to_string()),
            ("Did you git pull latest?".to_string(), "user".to_string()),
            ("Check git status".to_string(), "user".to_string()),
        ];
        nudge.load_corpus(messages).unwrap();

        // Process "push" - should use git context to produce "git push"
        let result = nudge.process("push");
        assert_eq!(result, "git push");
    }

    #[test]
    fn test_context_aware_npm() {
        // When conversation is about npm, "run" should map to "npm run"
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // Load npm-related corpus to set context
        let messages = vec![
            ("Run npm install first".to_string(), "user".to_string()),
            ("npm run dev".to_string(), "user".to_string()),
            ("npm test".to_string(), "user".to_string()),
        ];
        nudge.load_corpus(messages).unwrap();

        // Process "run" - should use npm context
        let result = nudge.process("run");
        assert_eq!(result, "npm run");
    }

    #[test]
    fn test_no_context_defaults_to_general() {
        // When there's no specific context, should use the most common mapping
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // Load generic corpus with no strong context
        let messages = vec![
            ("Hello there".to_string(), "user".to_string()),
            ("How are you".to_string(), "user".to_string()),
        ];
        nudge.load_corpus(messages).unwrap();

        // "run" without context defaults to "cargo run" (most common)
        let result = nudge.process("run");
        assert_eq!(result, "cargo run");
    }

    #[test]
    fn test_context_detects_multiple_topics() {
        // Context should detect multiple topics and pick the most recent one
        let temp_db = NamedTempFile::new().unwrap();
        let db_path = temp_db.path().to_path_buf();

        let mut nudge = Nudge::new(&db_path).unwrap();

        // First add some git context
        let git_messages = vec![
            ("git push".to_string(), "user".to_string()),
            ("git pull".to_string(), "user".to_string()),
        ];
        nudge.load_corpus(git_messages).unwrap();

        // Then add npm context (more recent)
        let npm_messages = vec![
            ("npm run build".to_string(), "user".to_string()),
            ("npm install".to_string(), "user".to_string()),
        ];
        nudge.load_corpus(npm_messages).unwrap();

        // "run" should now prefer npm context (most recent)
        let result = nudge.process("run");
        assert_eq!(result, "npm run");
    }
}
