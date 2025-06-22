// Utility functions (color parsing, mention extraction, etc)

use regex::Regex;
use common::{UserRole, UserColor};

// Parses a color from a string - returns UserColor
pub fn parse_color(color_str: &str) -> UserColor {
    UserColor::new(color_str)
}

// Parses a user role from a string
pub fn parse_role(role_str: &str) -> UserRole {
    match role_str {
        "Admin" => UserRole::Admin,
        "Moderator" => UserRole::Moderator,
        "User" | _ => UserRole::User,
    }
}

// Extracts mentions from the content, returning a vector of mention strings.
pub fn extract_mentions(content: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let re = Regex::new(r"@([a-zA-Z0-9_]+)").unwrap();
    for cap in re.captures_iter(content) {
        if let Some(username) = cap.get(1) {
            mentions.push(username.as_str().to_string());
        }
    }
    mentions
}
