// Utility functions (color parsing, mention extraction, etc)

use regex::Regex;
use ratatui::style::Color;
use common::{UserRole, UserColor};

// Parses a color from a string using the ratatui library.
pub fn parse_color(color_str: &str) -> Color {
    match color_str {
        "Reset" => Color::Reset,
        "Black" => Color::Black,
        "Red" => Color::Red,
        "Green" => Color::Green,
        "Yellow" => Color::Yellow,
        "Blue" => Color::Blue,
        "Magenta" => Color::Magenta,
        "Cyan" => Color::Cyan,
        "Gray" => Color::Gray,
        "DarkGray" => Color::DarkGray,
        "LightRed" => Color::LightRed,
        "LightGreen" => Color::LightGreen,
        "LightYellow" => Color::LightYellow,
        "LightBlue" => Color::LightBlue,
        "LightMagenta" => Color::LightMagenta,
        "LightCyan" => Color::LightCyan,
        "White" => Color::White,
        // Handle hex colors
        hex if hex.starts_with('#') && hex.len() == 7 => {
            if let Ok(r) = u8::from_str_radix(&hex[1..3], 16) {
                if let Ok(g) = u8::from_str_radix(&hex[3..5], 16) {
                    if let Ok(b) = u8::from_str_radix(&hex[5..7], 16) {
                        return Color::Rgb(r, g, b);
                    }
                }
            }
            Color::Reset
        }
        _ => Color::Reset,
    }
}

// Helper function to parse color string directly to UserColor
pub fn parse_user_color(color_str: &str) -> UserColor {
    UserColor::from(color_str)
}

// Parses a user role from a string
pub fn parse_role(role_str: &str) -> UserRole {
    match role_str {
        "Admin" => UserRole::Admin,
        "Moderator" => UserRole::Moderator,
        _ => UserRole::User,
    }
}

// Extracts mentions from the content, returning a vector of mention strings.
pub fn extract_mentions(content: &str) -> Vec<String> {
    let re = Regex::new(r"@([a-zA-Z0-9_]+)").unwrap();
    re.captures_iter(content)
        .map(|cap| cap[1].to_string())
        .collect()
}
