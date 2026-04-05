#[path = "../src/grid.rs"]
mod grid;
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/claude.rs"]
mod claude;
#[path = "../src/bridge.rs"]
mod bridge;

use grid::Grid;
use parser::GridPerformer;
use claude::{ClaudeState, DetectedProcess, parse_screen};
use bridge::{Bridge, BridgeEvent};

/// Parse an asciicast v2 file and replay the PTY output through our parser.
/// Returns a list of (timestamp, snapshot, screen) tuples at each frame.
fn replay_cast(path: &str, cols: usize, rows: usize) -> Vec<(f64, String, claude::ClaudeScreen)> {
    let content = std::fs::read_to_string(path).expect("failed to read cast file");
    let mut lines = content.lines();

    // Skip header
    let header = lines.next().expect("no header");
    let header: serde_json::Value = serde_json::from_str(header).expect("bad header");
    let cols = header["width"].as_u64().map(|v| v as usize).unwrap_or(cols);
    let rows = header["height"].as_u64().map(|v| v as usize).unwrap_or(rows);

    let grid = Grid::new(cols, rows);
    let mut vte_parser = vte::Parser::new();
    let mut performer = GridPerformer::new(grid);

    let mut results = Vec::new();

    for line in lines {
        let event: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = event[0].as_f64().unwrap_or(0.0);
        let event_type = event[1].as_str().unwrap_or("");
        let data = event[2].as_str().unwrap_or("");

        if event_type == "o" {
            // Output event — feed through parser
            GridPerformer::feed(&mut vte_parser, &mut performer, data.as_bytes());

            let snapshot = performer.grid.snapshot();
            let screen = parse_screen(&snapshot);
            results.push((ts, snapshot, screen));
        }
    }

    results
}

#[test]
fn replay_detects_idle_state() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);
    assert!(!results.is_empty(), "no frames parsed");

    // Should detect Idle at some point
    let has_idle = results.iter().any(|(_, _, screen)| screen.state == ClaudeState::Idle);
    assert!(has_idle, "never detected Idle state");
}

#[test]
fn replay_detects_thinking_or_spinner() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    // Thinking state or spinner text present in any frame
    let has_thinking = results.iter().any(|(_, _, screen)| screen.state == ClaudeState::Thinking);
    let has_spinner = results.iter().any(|(_, snap, _)| {
        snap.contains("esc to interrupt") || snap.contains("Incubating")
            || snap.contains("Simmering") || snap.contains("Contemplating")
    });
    assert!(has_thinking || has_spinner,
        "never detected Thinking state or spinner text in {} frames", results.len());
}

#[test]
fn replay_detects_claude_code_process() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    let has_claude = results.iter().any(|(_, _, screen)| screen.process == DetectedProcess::ClaudeCode);
    assert!(has_claude, "never detected Claude Code process");
}

#[test]
fn replay_extracts_responses() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    let has_response = results.iter().any(|(_, _, screen)| screen.response.is_some());
    assert!(has_response, "never extracted a response");
}

#[test]
fn replay_no_response_during_thinking() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    for (_, _, screen) in &results {
        if screen.state == ClaudeState::Thinking {
            assert!(screen.response.is_none(),
                "response extracted during Thinking: {:?}", screen.response);
        }
    }
}

#[test]
fn replay_bridge_emits_no_spinner_responses() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    let mut bridge = Bridge::new();
    let spinner_words = &[
        "Simmering", "Channelling", "Nucleating", "Percolating", "Distilling",
        "Crystallizing", "Manifesting", "Conjuring", "Synthesizing", "Composing",
        "Formulating", "Imagining", "Pondering", "Reflecting", "Contemplating",
        "Meditating", "Ruminating", "Deliberating", "Incubating", "Gestating",
        "Puttering", "Gitifying", "Whirlpooling", "Ideating", "Canoodling",
        "Moonwalking", "Cooked", "Cogitated",
    ];

    for (_, _, screen) in &results {
        let events = bridge.on_screen_update(screen);
        for event in &events {
            if let BridgeEvent::Response(text) = event {
                for word in spinner_words {
                    assert!(!text.contains(word),
                        "spinner word '{}' leaked into response: {}", word, text);
                }
            }
        }
    }
}

#[test]
fn replay_separator_lines_not_in_responses() {
    let results = replay_cast("tests/fixtures/claude-session.cast", 80, 24);

    for (_, _, screen) in &results {
        if let Some(ref response) = screen.response {
            for line in response.lines() {
                let trimmed = line.trim();
                // TUI separators: pure dashes or dashes with space-padded short label
                let char_count = trimmed.chars().count();
                if char_count > 20 && trimmed.starts_with('─') && trimmed.ends_with('─') {
                    let inner = trimmed.trim_start_matches('─').trim_end_matches('─');
                    let is_tui_sep = inner.is_empty()
                        || (inner.starts_with(' ') && inner.ends_with(' ') && inner.trim().len() <= 20);
                    assert!(!is_tui_sep,
                        "TUI separator line leaked into response: {}", trimmed);
                }
            }
        }
    }
}
