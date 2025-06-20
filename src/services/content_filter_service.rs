use std::collections::HashSet;
use regex::Regex;
use common::config::ModerationConfig;
use tracing::{info, warn};

/// Content filtering service for automatic moderation
pub struct ContentFilterService {
    blocked_words: HashSet<String>,
    blocked_patterns: Vec<Regex>,
    config: ModerationConfig,
}

#[derive(Debug)]
pub enum FilterResult {
    Allowed,
    Blocked { reason: String },
    Flagged { reason: String }, // For manual review
}

impl ContentFilterService {
    pub fn new(config: ModerationConfig) -> Result<Self, String> {
        let blocked_words: HashSet<String> = config.blocked_words
            .iter()
            .map(|word| word.to_lowercase())
            .collect();
        
        let mut blocked_patterns = Vec::new();
        for pattern in &config.blocked_patterns {
            match Regex::new(pattern) {
                Ok(regex) => blocked_patterns.push(regex),
                Err(e) => {
                    warn!("Invalid regex pattern '{}': {}", pattern, e);
                    return Err(format!("Invalid regex pattern '{}': {}", pattern, e));
                }
            }
        }
        
        Ok(Self {
            blocked_words,
            blocked_patterns,
            config,
        })
    }
    
    /// Filter message content
    pub fn filter_message(&self, content: &str, _author_id: uuid::Uuid) -> FilterResult {
        if !self.config.auto_moderation_enabled {
            return FilterResult::Allowed;
        }
        
        // Check message length
        if content.len() > self.config.message_length_limit {
            return FilterResult::Blocked {
                reason: format!(
                    "Message too long. Maximum {} characters allowed.",
                    self.config.message_length_limit
                ),
            };
        }
        
        // Check for blocked words
        let content_lower = content.to_lowercase();
        for word in &self.blocked_words {
            if content_lower.contains(word) {
                return FilterResult::Blocked {
                    reason: "Message contains blocked content".to_string(),
                };
            }
        }
        
        // Check regex patterns
        for pattern in &self.blocked_patterns {
            if pattern.is_match(content) {
                return FilterResult::Blocked {
                    reason: "Message contains blocked content pattern".to_string(),
                };
            }
        }
        
        FilterResult::Allowed
    }
}