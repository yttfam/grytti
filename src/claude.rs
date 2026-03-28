/// Claude CLI state detection from grid snapshots.
/// Parses the TUI output to detect state transitions and extract responses.

#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeState {
    /// Waiting at the `❯` prompt for input
    Idle,
    /// Spinner visible — thinking/processing
    Thinking,
    /// Tool use in progress (Read, Write, Bash, etc.)
    ToolUse,
    /// Not logged in
    NotLoggedIn,
    /// Login flow in progress
    LoginPrompt,
    /// Unknown / startup / not yet determined
    Unknown,
}

/// Parsed snapshot of Claude CLI's current screen
#[derive(Debug, Clone)]
pub struct ClaudeScreen {
    pub state: ClaudeState,
    /// The latest response block (text after ⏺), if any
    pub response: Option<String>,
    /// The current spinner text, if thinking
    pub spinner_text: Option<String>,
    /// The current tool use block, if any
    pub tool_block: Option<String>,
    /// OAuth login URL if visible on screen
    pub login_url: Option<String>,
    /// Whether "Paste code here" prompt is visible
    pub awaiting_code: bool,
    /// Whether "Login successful" is visible
    pub login_success: bool,
}

// Spinner symbols Claude Code uses (rotating set)
const SPINNER_CHARS: &[char] = &['✶', '✸', '✹', '✺', '✹', '✷', '✵', '✳', '✢', '·', '✻', '✽'];

// Spinner labels Claude Code uses
const SPINNER_WORDS: &[&str] = &[
    "Simmering", "Channelling", "Nucleating", "Percolating", "Distilling",
    "Crystallizing", "Manifesting", "Conjuring", "Synthesizing", "Composing",
    "Formulating", "Imagining", "Pondering", "Reflecting", "Contemplating",
    "Meditating", "Ruminating", "Deliberating", "Incubating", "Gestating",
];

// Tool use markers in Claude Code output
const TOOL_MARKERS: &[&str] = &[
    "Read(", "Edit(", "Write(", "Bash(", "Glob(", "Grep(", "Agent(",
    "WebFetch(", "WebSearch(", "TodoWrite(", "TaskCreate(", "TaskUpdate(",
];

pub fn parse_screen(snapshot: &str) -> ClaudeScreen {
    let lines: Vec<&str> = snapshot.lines().collect();

    let mut state = ClaudeState::Unknown;
    let mut spinner_text = None;
    let mut tool_block = None;
    let mut login_url = None;
    let mut awaiting_code = false;

    let mut has_esc_to_interrupt = false;
    let mut has_idle_prompt = false;
    let mut has_not_logged_in = false;
    let mut has_login_menu = false;
    let mut has_login_success = false;

    // Collect URL fragments across wrapped lines
    let mut url_fragments: Vec<String> = Vec::new();
    let mut in_url = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if in_url {
                in_url = false;
            }
            continue;
        }

        // "esc to interrupt" = Claude is actively working
        if trimmed == "esc to interrupt" {
            has_esc_to_interrupt = true;
            continue;
        }

        // "? for shortcuts" = idle state indicator
        if trimmed == "? for shortcuts" {
            has_idle_prompt = true;
            continue;
        }

        // "Not logged in" detection
        if trimmed.contains("Not logged in") {
            has_not_logged_in = true;
        }

        // Login success detection
        if trimmed.contains("Login successful") || trimmed.contains("Authenticated") {
            has_login_success = true;
        }

        // Login menu detection
        if trimmed.contains("Select login method") {
            has_login_menu = true;
        }

        // "Paste code here" detection
        if trimmed.contains("Paste code here") {
            awaiting_code = true;
        }

        // OAuth URL extraction — URL wraps across multiple lines in the grid
        if trimmed.starts_with("https://claude.com/cai/oauth") {
            in_url = true;
            url_fragments.clear();
            url_fragments.push(trimmed.to_string());
        } else if in_url {
            // Continuation lines of wrapped URL (no spaces in URLs)
            if !trimmed.contains(' ') || trimmed.starts_with("9-") || trimmed.starts_with("laude") || trimmed.starts_with("e+") || trimmed.starts_with("ile_") || trimmed.starts_with("hallenge") {
                url_fragments.push(trimmed.to_string());
            } else {
                in_url = false;
            }
        }

        // Check for spinner — symbol + word combo
        let first_char = trimmed.chars().next().unwrap_or(' ');
        if SPINNER_CHARS.contains(&first_char) {
            for word in SPINNER_WORDS {
                if trimmed.contains(word) {
                    state = ClaudeState::Thinking;
                    spinner_text = Some(trimmed.to_string());
                    break;
                }
            }
        }

        // Tool use markers are unreliable for state — old tool calls remain
        // on screen after completion. We track tool_block for info but don't
        // set state from it. State comes from bottom-bar indicators instead.
    }

    // Reconstruct URL from fragments
    if !url_fragments.is_empty() {
        login_url = Some(url_fragments.join(""));
    }

    // Resolve state from bottom-bar indicators (most reliable)
    // These override any spinner detection since they're authoritative
    if has_login_menu || awaiting_code {
        state = ClaudeState::LoginPrompt;
    } else if has_not_logged_in && state == ClaudeState::Unknown {
        state = ClaudeState::NotLoggedIn;
    } else if has_esc_to_interrupt {
        // "esc to interrupt" = thinking or tool use in progress
        state = ClaudeState::Thinking;
    } else if has_idle_prompt {
        // "? for shortcuts" = idle, ready for input
        state = ClaudeState::Idle;
    }

    let response = extract_turn_response(&lines);

    ClaudeScreen {
        state,
        response,
        spinner_text,
        tool_block,
        login_url,
        awaiting_code,
        login_success: has_login_success,
    }
}

/// Extract the full response for the last turn — everything between the last
/// `❯` prompt and the next prompt/separator area. Includes tool calls + output.
fn extract_turn_response(lines: &[&str]) -> Option<String> {
    // Find the second-to-last `❯` line — that's where the user's last message was.
    // Everything between that and the next `❯` (or end) is the response.
    let mut prompt_positions: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("❯") {
            prompt_positions.push(i);
        }
    }

    // Need at least 2 prompts: the one with the user's message and the current empty one
    if prompt_positions.len() < 2 {
        // Fall back: try to find any ⏺ block
        return extract_last_response_block(lines);
    }

    // The second-to-last prompt is the user's last input
    let last_input_idx = prompt_positions[prompt_positions.len() - 2];
    // The last prompt is the current empty one
    let current_prompt_idx = prompt_positions[prompt_positions.len() - 1];

    let mut response_lines = Vec::new();
    for i in (last_input_idx + 1)..current_prompt_idx {
        let trimmed = lines[i].trim();
        // Skip heavy separators
        if trimmed.len() > 20 && trimmed.chars().all(|c| c == '─') {
            continue;
        }
        response_lines.push(lines[i].trim_end().to_string());
    }

    // Trim leading/trailing empty lines
    while response_lines.first().map_or(false, |l| l.trim().is_empty()) {
        response_lines.remove(0);
    }
    while response_lines.last().map_or(false, |l| l.trim().is_empty()) {
        response_lines.pop();
    }

    if response_lines.is_empty() {
        None
    } else {
        Some(response_lines.join("\n"))
    }
}

/// Fallback: extract text after the last `⏺` marker.
fn extract_last_response_block(lines: &[&str]) -> Option<String> {
    let mut last_start = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().starts_with("⏺") {
            last_start = Some(i);
        }
    }

    let start = last_start?;
    let mut response_lines = Vec::new();

    let first = lines[start].trim();
    let after_marker = first.strip_prefix("⏺").unwrap_or(first).trim();
    if !after_marker.is_empty() {
        response_lines.push(after_marker.to_string());
    }

    for line in &lines[start + 1..] {
        let trimmed = line.trim();
        if trimmed.starts_with("❯") {
            break;
        }
        if trimmed.len() > 20 && trimmed.chars().all(|c| c == '─') {
            break;
        }
        response_lines.push(line.trim_end().to_string());
    }

    while response_lines.last().map_or(false, |l| l.trim().is_empty()) {
        response_lines.pop();
    }

    if response_lines.is_empty() {
        None
    } else {
        Some(response_lines.join("\n"))
    }
}
