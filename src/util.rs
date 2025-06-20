// server/src/util.rs
// Utility functions (color parsing, mention extraction, etc)

use regex::Regex;
use ratatui::style::Color;

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
