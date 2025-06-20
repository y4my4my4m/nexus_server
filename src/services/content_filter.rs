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
    pub fn filter_message(&self, content: &str, author_id: uuid::Uuid) -> FilterResult {
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
                info!("Blocked message from {} containing word: {}", author_id, word);
                return FilterResult::Blocked {
                    reason: "Message contains blocked content".to_string(),
                };
            }
        }
        
        // Check regex patterns
        for pattern in &self.blocked_patterns {
            if pattern.is_match(content) {
                info!("Blocked message from {} matching pattern: {}", author_id, pattern.as_str());
                return FilterResult::Blocked {
                    reason: "Message contains blocked content pattern".to_string(),
                };
            }
        }
        
        // Check for potential spam (basic heuristics)
        if self.is_potential_spam(content) {
            return FilterResult::Flagged {
                reason: "Message flagged as potential spam".to_string(),
            };
        }
        
        FilterResult::Allowed
    }
    
    /// Basic spam detection
    fn is_potential_spam(&self, content: &str) -> bool {
        // Check for excessive repetition
        let words: Vec<&str> = content.split_whitespace().collect();
        if words.len() > 10 {
            let unique_words: HashSet<&str> = words.iter().cloned().collect();
            let repetition_ratio = words.len() as f32 / unique_words.len() as f32;
            if repetition_ratio > 3.0 {
                return true;
            }
        }
        
        // Check for excessive capitalization
        let caps_count = content.chars().filter(|c| c.is_uppercase()).count();
        let total_letters = content.chars().filter(|c| c.is_alphabetic()).count();
        if total_letters > 20 && caps_count as f32 / total_letters as f32 > 0.7 {
            return true;
        }
        
        // Check for excessive special characters
        let special_count = content.chars().filter(|c| !c.is_alphanumeric() && !c.is_whitespace()).count();
        if content.len() > 20 && special_count as f32 / content.len() as f32 > 0.5 {
            return true;
        }
        
        false
    }
    
    /// Filter username for registration
    pub fn filter_username(&self, username: &str) -> FilterResult {
        if !self.config.auto_moderation_enabled {
            return FilterResult::Allowed;
        }
        
        // Check for blocked words in username
        let username_lower = username.to_lowercase();
        for word in &self.blocked_words {
            if username_lower.contains(word) {
                return FilterResult::Blocked {
                    reason: "Username contains blocked content".to_string(),
                };
            }
        }
        
        // Check regex patterns
        for pattern in &self.blocked_patterns {
            if pattern.is_match(username) {
                return FilterResult::Blocked {
                    reason: "Username contains blocked content pattern".to_string(),
                };
            }
        }
        
        FilterResult::Allowed
    }
    
    /// Update blocked words list
    pub fn update_blocked_words(&mut self, words: Vec<String>) {
        self.blocked_words = words.into_iter().map(|word| word.to_lowercase()).collect();
        info!("Updated blocked words list with {} entries", self.blocked_words.len());
    }
    
    /// Update blocked patterns
    pub fn update_blocked_patterns(&mut self, patterns: Vec<String>) -> Result<(), String> {
        let mut new_patterns = Vec::new();
        for pattern in &patterns {
            match Regex::new(pattern) {
                Ok(regex) => new_patterns.push(regex),
                Err(e) => return Err(format!("Invalid regex pattern '{}': {}", pattern, e)),
            }
        }
        
        self.blocked_patterns = new_patterns;
        info!("Updated blocked patterns list with {} entries", self.blocked_patterns.len());
        Ok(())
    }
}