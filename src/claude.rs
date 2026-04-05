use serde::Serialize;

/// Returns true if the only difference between two snapshots is a spinner animation.
/// Spinner changes are single-character diffs (rotating symbol) — not real content.
pub fn is_spinner_only_change(old: &str, new: &str) -> bool {
    if old == new {
        return false; // no change at all
    }
    if old.len() != new.len() {
        return false; // length changed = real content
    }

    let diff_count = old.chars().zip(new.chars()).filter(|(a, b)| a != b).count();
    diff_count == 1
}

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
    /// Permission prompt — waiting for user to approve/deny
    PermissionPrompt,
    /// Unknown / startup / not yet determined
    Unknown,
}

/// What process appears to be running in the terminal
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum DetectedProcess {
    ClaudeCode,
    Shell,
    Unknown,
}

/// Permission prompt info extracted from the screen
#[derive(Debug, Clone)]
pub struct PermissionInfo {
    pub tool: String,
    pub command: String,
    pub options: Vec<String>,
}

/// Parsed snapshot of Claude CLI's current screen
#[derive(Debug, Clone)]
pub struct ClaudeScreen {
    pub state: ClaudeState,
    pub process: DetectedProcess,
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
    /// Permission prompt, if one is visible
    pub permission: Option<PermissionInfo>,
}

// Spinner symbols Claude Code uses (rotating set)
const SPINNER_CHARS: &[char] = &['✶', '✸', '✹', '✺', '✹', '✷', '✵', '✳', '✢', '·', '✻', '✽'];

// Spinner labels Claude Code uses
// Extracted from Claude Code binary — full spinner word list
pub const SPINNER_WORDS: &[&str] = &[
    "Accomplishing", "Actioning", "Actualizing", "Architecting",
    "Baking", "Beaming", "Befuddling", "Billowing", "Blanching",
    "Bloviating", "Boogieing", "Boondoggling", "Booping", "Bootstrapping",
    "Brewing", "Bunning", "Burrowing",
    "Calculating", "Canoodling", "Caramelizing", "Cascading", "Catapulting",
    "Cerebrating", "Channeling", "Channelling", "Choreographing", "Churning",
    "Clauding", "Coalescing", "Cogitating", "Combobulating", "Composing",
    "Computing", "Concocting", "Considering", "Contemplating", "Cooking",
    "Crafting", "Creating", "Crunching", "Crystallizing", "Cultivating",
    "Deciphering", "Deliberating", "Determining", "Discombobulating",
    "Doing", "Doodling", "Drizzling",
    "Ebbing", "Effecting", "Elucidating", "Embellishing", "Enchanting",
    "Envisioning", "Evaporating",
    "Fermenting", "Finagling", "Flibbertigibbeting", "Flowing", "Flummoxing",
    "Fluttering", "Forging", "Forming", "Frolicking", "Frosting",
    "Gallivanting", "Galloping", "Garnishing", "Generating", "Germinating",
    "Gesticulating", "Gitifying", "Grooving", "Gusting",
    "Harmonizing", "Hashing", "Hatching", "Herding", "Honking",
    "Hullaballooing", "Hyperspacing",
    "Ideating", "Imagining", "Improvising", "Incubating", "Inferring", "Infusing",
    "Ionizing",
    "Jitterbugging", "Julienning",
    "Kneading",
    "Leavening", "Levitating", "Lollygagging",
    "Manifesting", "Marinating", "Meandering", "Metamorphosing", "Misting",
    "Moonwalking", "Moseying", "Mulling", "Musing", "Mustering",
    "Nebulizing", "Nesting", "Newspapering", "Noodling", "Nucleating",
    "Orbiting", "Orchestrating", "Osmosing",
    "Perambulating", "Percolating", "Perusing", "Philosophising",
    "Photosynthesizing", "Pollinating", "Pondering", "Pontificating",
    "Pouncing", "Precipitating", "Prestidigitating", "Processing", "Proofing",
    "Propagating", "Puttering", "Puzzling",
    "Quantumizing",
    "Razzmatazzing", "Recombobulating", "Reticulating", "Roosting", "Ruminating",
    "Scampering", "Schlepping", "Scurrying", "Seasoning", "Shenaniganing",
    "Shimmying", "Simmering", "Skedaddling", "Sketching", "Slithering",
    "Smooshing", "Spelunking", "Spinning", "Sprouting", "Stewing",
    "Sublimating", "Swirling", "Swooping", "Symbioting", "Synthesizing",
    "Tempering", "Thinking", "Thundering", "Tinkering", "Tomfoolering",
    "Transfiguring", "Transmuting", "Twisting",
    "Undulating", "Unfurling", "Unravelling",
    "Vibing",
    "Waddling", "Wandering", "Warping", "Whatchamacalliting", "Whirlpooling",
    "Whirring", "Whisking", "Wibbling", "Working", "Wrangling",
    "Zesting", "Zigzagging",
    // Past tense (completion summaries: "✻ Cooked for 32s")
    "Baked", "Brewed", "Churned", "Cogitated", "Cooked", "Crunched", "Worked",
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
    let mut has_esc_to_interrupt_stale = false;
    let mut has_idle_prompt = false;
    let mut has_not_logged_in = false;
    let mut has_login_menu = false;
    let mut has_login_success = false;
    let mut has_permission_prompt = false;
    let mut permission_options: Vec<String> = Vec::new();

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
        if trimmed.contains("esc to interrupt") {
            has_esc_to_interrupt = true;
            // When embedded in "accept edits on ... · esc to interrupt", it can be stale
            if trimmed.contains("accept edits on") {
                has_esc_to_interrupt_stale = true;
            }
            continue;
        }

        // Idle: the `❯` prompt visible between separator lines means Claude is ready.
        // We detect idle by the ABSENCE of "esc to interrupt" rather than
        // matching specific hint text, since Claude Code has many bottom-bar variants
        // (? for shortcuts, accept edits on, etc.)
        if trimmed == "❯" {
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

        // Permission prompt detection
        if trimmed.contains("Do you want to proceed")
            || trimmed.contains("Do you want to create")
            || trimmed.contains("Do you want to overwrite")
            || trimmed.contains("Do you want to make this edit")
        {
            has_permission_prompt = true;
        }

        // Extract numbered permission options (e.g. "1. Yes", "2. No")
        // Options can start with ❯ (selected) or spaces
        if has_permission_prompt {
            let opt_line = trimmed.trim_start_matches('❯').trim();
            if let Some(rest) = opt_line.strip_prefix(|c: char| c.is_ascii_digit()) {
                if let Some(label) = rest.strip_prefix(". ") {
                    permission_options.push(label.to_string());
                }
            }
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
    if has_permission_prompt && !permission_options.is_empty() {
        state = ClaudeState::PermissionPrompt;
    } else if has_login_menu || awaiting_code {
        state = ClaudeState::LoginPrompt;
    } else if has_not_logged_in && state == ClaudeState::Unknown {
        state = ClaudeState::NotLoggedIn;
    } else if has_esc_to_interrupt {
        // "esc to interrupt" = thinking. But on the combined bottom bar
        // ("accept edits on · esc to interrupt"), it can be stale after response.
        // Use spinner detection as the tiebreaker.
        if state == ClaudeState::Thinking {
            // Spinner was detected — definitely thinking
        } else if !has_esc_to_interrupt_stale {
            // Standalone "esc to interrupt" — trust it
            state = ClaudeState::Thinking;
        } else {
            // Stale combined line, no spinner — fall through to idle check
            if has_idle_prompt {
                state = ClaudeState::Idle;
            }
        }
    } else if has_idle_prompt {
        state = ClaudeState::Idle;
    }

    // Build permission info if detected
    let permission = if has_permission_prompt && !permission_options.is_empty() {
        // Try to extract tool name from screen — look for "Bash ·" or tool keywords
        let tool = lines.iter()
            .find_map(|l| {
                let t = l.trim();
                // Permission dialog title: "Bash · command" or just "Bash"
                for name in &["Bash", "Write", "Edit", "Read", "Notebook"] {
                    if t.starts_with(name) && (t.len() == name.len() || t.as_bytes().get(name.len()) == Some(&b' ')) {
                        return Some(name.to_string());
                    }
                }
                // Also check for "Create file" / "Overwrite file" / "Edit file"
                if t.starts_with("Create file") { return Some("Write".to_string()); }
                if t.starts_with("Overwrite file") { return Some("Write".to_string()); }
                if t.starts_with("Edit file") { return Some("Edit".to_string()); }
                None
            })
            .unwrap_or_else(|| "Tool".to_string());

        Some(PermissionInfo {
            tool,
            command: String::new(), // TODO: extract command from dialog
            options: permission_options,
        })
    } else {
        None
    };

    // Detect process
    let process = if has_idle_prompt || has_esc_to_interrupt || has_login_menu || awaiting_code || has_permission_prompt {
        DetectedProcess::ClaudeCode
    } else {
        // Check for shell prompt patterns
        let has_shell_prompt = lines.iter().any(|l| {
            let t = l.trim();
            // Common shell prompt patterns
            (t.contains("@") && (t.ends_with('$') || t.ends_with('%') || t.ends_with('#')))
                || (t.contains("@") && t.contains(":") && t.contains("$"))
        });
        if has_shell_prompt {
            DetectedProcess::Shell
        } else if state != ClaudeState::Unknown {
            DetectedProcess::ClaudeCode
        } else {
            DetectedProcess::Unknown
        }
    };

    // Don't extract response while thinking — spinner content between prompts
    // would be falsely treated as a response
    let response = if state == ClaudeState::Thinking {
        None
    } else if process == DetectedProcess::Shell {
        extract_shell_output(&lines).or_else(|| extract_shell_snapshot(&lines))
    } else {
        extract_turn_response(&lines)
    };

    ClaudeScreen {
        state,
        process,
        response,
        spinner_text,
        tool_block,
        login_url,
        awaiting_code,
        login_success: has_login_success,
        permission,
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
        if is_separator_line(trimmed) { continue; }
        if trimmed == "⏺" { continue; }
        if is_spinner_line(trimmed) { continue; }
        if is_timing_line(trimmed) { continue; }
        let line = if trimmed.starts_with("⏺ ") {
            lines[i].trim_end().replacen("⏺ ", "", 1)
        } else {
            lines[i].trim_end().to_string()
        };
        response_lines.push(line);
    }

    // Trim leading/trailing empty lines
    while response_lines.first().map_or(false, |l| l.trim().is_empty()) {
        response_lines.remove(0);
    }
    while response_lines.last().map_or(false, |l| l.trim().is_empty()) {
        response_lines.pop();
    }

    clean_response(response_lines)
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
        if is_separator_line(trimmed) { break; }
        if trimmed == "⏺" { continue; }
        if is_spinner_line(trimmed) { continue; }
        if is_timing_line(trimmed) { continue; }
        let cleaned = if trimmed.starts_with("⏺ ") {
            line.trim_end().replacen("⏺ ", "", 1)
        } else {
            line.trim_end().to_string()
        };
        response_lines.push(cleaned);
    }

    while response_lines.last().map_or(false, |l| l.trim().is_empty()) {
        response_lines.pop();
    }

    clean_response(response_lines)
}

/// Extract shell output — everything between the second-to-last prompt and the last prompt.
fn extract_shell_output(lines: &[&str]) -> Option<String> {
    let is_shell_prompt = |line: &str| -> bool {
        let t = line.trim();
        (t.contains('@') && (t.ends_with('$') || t.ends_with('%') || t.ends_with('#')))
            || (t.contains('@') && t.contains(':') && t.contains('$'))
    };

    let mut prompt_positions: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if is_shell_prompt(line) {
            prompt_positions.push(i);
        }
    }

    if prompt_positions.len() < 2 {
        return None;
    }

    let cmd_prompt = prompt_positions[prompt_positions.len() - 2];
    let current_prompt = prompt_positions[prompt_positions.len() - 1];

    let mut output_lines = Vec::new();
    for i in (cmd_prompt + 1)..current_prompt {
        output_lines.push(lines[i].trim_end().to_string());
    }

    while output_lines.first().map_or(false, |l| l.trim().is_empty()) {
        output_lines.remove(0);
    }
    while output_lines.last().map_or(false, |l| l.trim().is_empty()) {
        output_lines.pop();
    }

    if output_lines.is_empty() {
        None
    } else {
        Some(output_lines.join("\n"))
    }
}

/// Fallback for shell: grab everything above the last shell prompt.
/// Used when the previous prompt scrolled off screen.
fn extract_shell_snapshot(lines: &[&str]) -> Option<String> {
    let is_shell_prompt = |line: &str| -> bool {
        let t = line.trim();
        (t.contains('@') && (t.ends_with('$') || t.ends_with('%') || t.ends_with('#')))
            || (t.contains('@') && t.contains(':') && t.contains('$'))
    };

    // Find the last prompt
    let mut last_prompt = None;
    for (i, line) in lines.iter().enumerate() {
        if is_shell_prompt(line) {
            last_prompt = Some(i);
        }
    }

    let prompt_idx = last_prompt?;
    if prompt_idx == 0 {
        return None;
    }

    // Grab everything above the last prompt, skip leading empty lines
    let mut output_lines = Vec::new();
    for i in 0..prompt_idx {
        output_lines.push(lines[i].trim_end().to_string());
    }

    while output_lines.first().map_or(false, |l| l.trim().is_empty()) {
        output_lines.remove(0);
    }
    while output_lines.last().map_or(false, |l| l.trim().is_empty()) {
        output_lines.pop();
    }

    if output_lines.is_empty() {
        None
    } else {
        Some(output_lines.join("\n"))
    }
}

/// Check if a line is spinner content (rotating symbol + thinking word).
pub fn is_spinner_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() { return false; }
    let first_char = trimmed.chars().next().unwrap_or(' ');
    if !SPINNER_CHARS.contains(&first_char) { return false; }
    SPINNER_WORDS.iter().any(|w| trimmed.contains(w))
}

/// "Cooked for Xs" / "Cogitated for Xs" summary lines — not spinner, but timing info
fn is_timing_line(line: &str) -> bool {
    let t = line.trim();
    SPINNER_CHARS.contains(&t.chars().next().unwrap_or(' '))
        && (t.contains("for ") && (t.contains("s") || t.contains("m ")))
}

/// Check if a line is a TUI separator (not content).
/// TUI separators: pure ─── or ─── with a short label like " infrakid "
/// Content like "──EOF──────" from heredocs should NOT match.
fn is_separator_line(line: &str) -> bool {
    let char_count = line.chars().count();
    if char_count < 20 {
        return false;
    }
    // Must start and end with ─
    if !line.starts_with('─') || !line.ends_with('─') {
        return false;
    }
    // Extract non-dash content
    let non_dash: String = line.chars().filter(|&c| c != '─' && c != ' ').collect();
    // Pure dashes (+ spaces) = separator
    if non_dash.is_empty() {
        return true;
    }
    // TUI labels are space-padded: "── infrakid ──" → " infrakid " after stripping dashes
    // Content like "──EOF──" has no leading/trailing space after stripping dashes
    let inner = line.trim_start_matches('─').trim_end_matches('─');
    // Empty inner = pure dashes (already handled above)
    // Space-padded label = TUI separator
    inner.starts_with(' ') && inner.ends_with(' ') && inner.trim().len() <= 20
}

/// Final cleanup: remove junk from partial redraws, spinner text, garbled separators.
fn clean_response(lines: Vec<String>) -> Option<String> {
    let cleaned: Vec<String> = lines.into_iter()
        .filter(|line| {
            let t = line.trim();
            // Remove lines containing spinner words
            if SPINNER_WORDS.iter().any(|w| t.contains(w)) {
                return false;
            }
            // Remove lines that are just a spinner char
            if t.chars().count() <= 2 && t.chars().next().map_or(false, |c| SPINNER_CHARS.contains(&c)) {
                return false;
            }
            // Remove garbled lines from partial redraws.
            // Normal response text never contains ─ (box-drawing dash).
            // Any line with multiple ─ chars is either a separator or redraw garbage.
            let dash_count = t.chars().filter(|&c| c == '─').count();
            if dash_count > 5 {
                return false;
            }
            true
        })
        .collect();

    // Trim leading/trailing empty lines after filtering
    let start = cleaned.iter().position(|l| !l.trim().is_empty()).unwrap_or(0);
    let end = cleaned.iter().rposition(|l| !l.trim().is_empty()).map(|i| i + 1).unwrap_or(0);

    if start >= end {
        None
    } else {
        // Strip common leading indent (Claude Code indents responses with 2 spaces)
        let slice = &cleaned[start..end];
        let min_indent = slice.iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.len() - l.trim_start().len())
            .min()
            .unwrap_or(0);

        let dedented: Vec<&str> = slice.iter()
            .map(|l| if l.len() >= min_indent { &l[min_indent..] } else { l.as_str() })
            .collect();

        // Unwrap terminal-wrapped lines and collapse blank lines.
        // Terminal wraps at ~80 cols mid-sentence. Blank lines = real paragraph breaks.
        // Lines starting with special chars (⎿, -, *, •, digits) = keep as separate lines.
        let mut result = String::new();
        let mut prev_blank = false;
        for line in &dedented {
            let is_blank = line.trim().is_empty();
            if is_blank {
                if !prev_blank && !result.is_empty() {
                    result.push_str("\n\n");
                }
                prev_blank = true;
                continue;
            }
            prev_blank = false;

            let trimmed = line.trim();
            // Lines that should start a new line (not unwrapped)
            let is_new_block = trimmed.starts_with('⎿')
                || trimmed.starts_with('-')
                || trimmed.starts_with('*')
                || trimmed.starts_with('•')
                || trimmed.starts_with("Bash(")
                || trimmed.starts_with("Read(")
                || trimmed.starts_with("Edit(")
                || trimmed.starts_with("Write(")
                || trimmed.starts_with("Glob(")
                || trimmed.starts_with("Grep(")
                || trimmed.chars().next().map_or(false, |c| c.is_ascii_digit() && trimmed.chars().nth(1) == Some('.'));

            if result.is_empty() || is_new_block {
                if !result.is_empty() && !result.ends_with('\n') {
                    result.push('\n');
                }
                result.push_str(trimmed);
            } else {
                // Continuation of a wrapped line — join with space
                result.push(' ');
                result.push_str(trimmed);
            }
        }

        // Trim trailing whitespace
        let result = result.trim_end().to_string();
        if result.is_empty() { None } else { Some(result) }
    }
}
