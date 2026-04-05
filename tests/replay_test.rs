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
use claude::{ClaudeState, DetectedProcess, parse_screen, is_spinner_only_change};
use bridge::{Bridge, BridgeEvent};

/// Replay a .cast file, feeding every output event through the VTE parser.
/// Returns snapshots at each frame.
fn replay_frames(path: &str) -> Vec<(f64, String, claude::ClaudeScreen)> {
    let content = std::fs::read_to_string(path).expect("failed to read cast file");
    let mut lines = content.lines();

    let header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
    let cols = header["width"].as_u64().unwrap_or(80) as usize;
    let rows = header["height"].as_u64().unwrap_or(24) as usize;

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
        if event[1].as_str().unwrap_or("") != "o" { continue; }
        let data = event[2].as_str().unwrap_or("");

        GridPerformer::feed(&mut vte_parser, &mut performer, data.as_bytes());

        let snapshot = performer.grid.snapshot();
        let screen = parse_screen(&snapshot);
        results.push((ts, snapshot, screen));
    }
    results
}

/// Replay through the bridge, collecting emitted events.
/// Simulates the main loop: feeds frames, skips spinner-only changes,
/// runs bridge on real changes.
fn replay_bridge(path: &str) -> (Vec<BridgeEvent>, Vec<String>) {
    let content = std::fs::read_to_string(path).expect("failed to read cast file");
    let mut lines_iter = content.lines();

    let header: serde_json::Value = serde_json::from_str(lines_iter.next().unwrap()).unwrap();
    let cols = header["width"].as_u64().unwrap_or(80) as usize;
    let rows = header["height"].as_u64().unwrap_or(24) as usize;

    // Collect events with timestamps
    let mut raw_events: Vec<(f64, String)> = Vec::new();
    for line in lines_iter {
        let event: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if event[1].as_str().unwrap_or("") != "o" { continue; }
        if let Some(data) = event[2].as_str() {
            raw_events.push((event[0].as_f64().unwrap_or(0.0), data.to_string()));
        }
    }

    let grid = Grid::new(cols, rows);
    let mut vte_parser = vte::Parser::new();
    let mut performer = GridPerformer::new(grid);
    let mut bridge = Bridge::new();
    let mut last_published = String::new();
    let mut last_ts = 0.0f64;
    let mut all_events = Vec::new();
    let mut all_states = Vec::new();

    for (_ts, data) in &raw_events {
        GridPerformer::feed(&mut vte_parser, &mut performer, data.as_bytes());

        let snapshot = performer.grid.snapshot();
        if snapshot.is_empty() || snapshot == last_published {
            continue;
        }
        if is_spinner_only_change(&last_published, &snapshot) {
            last_published = snapshot;
            continue;
        }

        let screen = parse_screen(&snapshot);
        all_states.push(format!("{:?}", screen.state));

        let events = bridge.on_screen_update(&screen);
        all_events.extend(events);

        last_published = snapshot;
    }

    (all_events, all_states)
}

const CAST_FILE: &str = "tests/fixtures/claude-session.cast";

// Spinner words that must NEVER appear in a Response event
const SPINNER_WORDS: &[&str] = &[
    "Simmering", "Channelling", "Nucleating", "Percolating", "Distilling",
    "Crystallizing", "Manifesting", "Conjuring", "Synthesizing", "Composing",
    "Formulating", "Imagining", "Pondering", "Reflecting", "Contemplating",
    "Meditating", "Ruminating", "Deliberating", "Incubating", "Gestating",
    "Puttering", "Gitifying", "Whirlpooling", "Ideating", "Canoodling",
    "Moonwalking",
];

// === Frame-level tests ===

#[test]
fn replay_has_frames() {
    let results = replay_frames(CAST_FILE);
    assert!(results.len() > 100, "only {} frames", results.len());
}

#[test]
fn replay_detects_idle() {
    let results = replay_frames(CAST_FILE);
    let idle_count = results.iter().filter(|(_, _, s)| s.state == ClaudeState::Idle).count();
    assert!(idle_count > 0, "never detected Idle in {} frames", results.len());
}

#[test]
fn replay_detects_claude_code() {
    let results = replay_frames(CAST_FILE);
    let cc_count = results.iter().filter(|(_, _, s)| s.process == DetectedProcess::ClaudeCode).count();
    assert!(cc_count > 0, "never detected Claude Code process");
}

#[test]
fn replay_has_responses() {
    let results = replay_frames(CAST_FILE);
    let resp_count = results.iter().filter(|(_, _, s)| s.response.is_some()).count();
    assert!(resp_count > 0, "no responses extracted from {} frames", results.len());
}

#[test]
fn replay_no_response_while_thinking() {
    let results = replay_frames(CAST_FILE);
    for (ts, _, screen) in &results {
        if screen.state == ClaudeState::Thinking {
            assert!(screen.response.is_none(),
                "response during Thinking at {:.1}s: {:?}", ts,
                screen.response.as_deref().unwrap_or("").chars().take(50).collect::<String>());
        }
    }
}

// === Separator filtering ===

#[test]
fn replay_no_tui_separators_in_responses() {
    let results = replay_frames(CAST_FILE);
    for (ts, _, screen) in &results {
        if let Some(ref response) = screen.response {
            for line in response.lines() {
                let trimmed = line.trim();
                let char_count = trimmed.chars().count();
                if char_count > 20 && trimmed.starts_with('─') && trimmed.ends_with('─') {
                    let inner = trimmed.trim_start_matches('─').trim_end_matches('─');
                    let is_tui_sep = inner.is_empty()
                        || (inner.starts_with(' ') && inner.ends_with(' ') && inner.trim().len() <= 20);
                    assert!(!is_tui_sep,
                        "TUI separator at {:.1}s: {}", ts, trimmed);
                }
            }
        }
    }
}

#[test]
fn replay_no_bare_markers_in_responses() {
    let results = replay_frames(CAST_FILE);
    for (_, _, screen) in &results {
        if let Some(ref response) = screen.response {
            for line in response.lines() {
                assert!(line.trim() != "⏺", "bare ⏺ marker in response");
            }
        }
    }
}

// === Bridge event tests ===

#[test]
fn bridge_emits_responses() {
    let (events, _) = replay_bridge(CAST_FILE);
    let resp_count = events.iter().filter(|e| matches!(e, BridgeEvent::Response(_))).count();
    assert!(resp_count > 0, "bridge never emitted a Response");
}

#[test]
fn bridge_no_spinner_in_responses() {
    let (events, _) = replay_bridge(CAST_FILE);
    for event in &events {
        if let BridgeEvent::Response(text) = event {
            for word in SPINNER_WORDS {
                assert!(!text.contains(word),
                    "spinner '{}' in bridge Response: {}...{}",
                    word,
                    text.chars().take(30).collect::<String>(),
                    text.chars().rev().take(30).collect::<String>());
            }
        }
    }
}

#[test]
fn bridge_no_empty_responses() {
    let (events, _) = replay_bridge(CAST_FILE);
    for event in &events {
        if let BridgeEvent::Response(text) = event {
            assert!(!text.trim().is_empty(), "bridge emitted empty Response");
        }
    }
}

#[test]
fn bridge_no_duplicate_consecutive_responses() {
    let (events, _) = replay_bridge(CAST_FILE);
    let responses: Vec<&str> = events.iter()
        .filter_map(|e| if let BridgeEvent::Response(t) = e { Some(t.as_str()) } else { None })
        .collect();

    for pair in responses.windows(2) {
        assert_ne!(pair[0], pair[1], "duplicate consecutive Response: {}...",
            pair[0].chars().take(50).collect::<String>());
    }
}

#[test]
fn bridge_no_rapid_subset_responses() {
    // Consecutive responses shouldn't be exact duplicates.
    // A response containing the previous is OK if there was a real
    // Thinking→Idle cycle between them (Claude paused mid-response).
    let (events, _) = replay_bridge(CAST_FILE);
    let responses: Vec<&str> = events.iter()
        .filter_map(|e| if let BridgeEvent::Response(t) = e { Some(t.as_str()) } else { None })
        .collect();

    for pair in responses.windows(2) {
        assert_ne!(pair[0], pair[1], "exact duplicate response: {}...",
            pair[0].chars().take(50).collect::<String>());
    }
}

#[test]
fn bridge_no_separator_in_responses() {
    let (events, _) = replay_bridge(CAST_FILE);
    for event in &events {
        if let BridgeEvent::Response(text) = event {
            for line in text.lines() {
                let trimmed = line.trim();
                let char_count = trimmed.chars().count();
                if char_count > 20 && trimmed.starts_with('─') && trimmed.ends_with('─') {
                    let inner = trimmed.trim_start_matches('─').trim_end_matches('─');
                    let is_tui_sep = inner.is_empty()
                        || (inner.starts_with(' ') && inner.ends_with(' ') && inner.trim().len() <= 20);
                    assert!(!is_tui_sep,
                        "TUI separator in bridge Response: {}", trimmed);
                }
            }
        }
    }
}

// === State transition sanity ===

#[test]
fn replay_has_state_transitions() {
    let (_, states) = replay_bridge(CAST_FILE);
    let unique: std::collections::HashSet<&str> = states.iter().map(|s| s.as_str()).collect();
    assert!(unique.len() >= 2,
        "only {} unique states: {:?}", unique.len(), unique);
}
