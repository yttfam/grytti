// Login flow state machine tests.
// We can't import login.rs directly because it depends on api::SessionState.
// Instead, test the bridge + claude state transitions that drive login.

#[path = "../src/claude.rs"]
mod claude;
#[path = "../src/bridge.rs"]
mod bridge;

use bridge::{Bridge, BridgeEvent};
use claude::{ClaudeState, DetectedProcess};

#[test]
fn bridge_starts_clean() {
    let br = Bridge::new();
    assert_eq!(br.last_state, ClaudeState::Unknown);
    assert_eq!(br.last_process, DetectedProcess::Unknown);
    assert!(br.last_sent_response.is_empty());
}

#[test]
fn bridge_emits_thinking() {
    let mut br = Bridge::new();
    let screen = claude::ClaudeScreen {
        state: ClaudeState::Thinking,
        process: DetectedProcess::ClaudeCode,
        response: None,
        spinner_text: None,
        tool_block: None,
        login_url: None,
        awaiting_code: false,
        login_success: false,
    };
    let events = br.on_screen_update(&screen);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Thinking)));
}

#[test]
fn bridge_emits_response_after_settle() {
    let mut br = Bridge::new();

    // First: thinking
    let thinking = claude::ClaudeScreen {
        state: ClaudeState::Thinking,
        process: DetectedProcess::ClaudeCode,
        response: None,
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    br.on_screen_update(&thinking);

    // Then: idle with response — settle timer starts
    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle,
        process: DetectedProcess::ClaudeCode,
        response: Some("Hello!".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&idle);
    // No response yet — settle timer just started
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));

    // Wait for settle, then call again with same Idle state
    std::thread::sleep(std::time::Duration::from_millis(600));
    // idle_since is set, last_state is now Idle, but idle_since hasn't been cleared
    // Need to re-enter the settle check — call with Idle again
    let events = br.on_screen_update(&idle);
    // Now response should be emitted
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Response(t) if t == "Hello!")));
}

#[test]
fn bridge_emits_process_change() {
    let mut br = Bridge::new();
    let screen = claude::ClaudeScreen {
        state: ClaudeState::Unknown,
        process: DetectedProcess::Shell,
        response: None,
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&screen);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::ProcessChanged(DetectedProcess::Shell))));

    // Same process again — no duplicate event
    let events = br.on_screen_update(&screen);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::ProcessChanged(_))));
}

#[test]
fn bridge_no_duplicate_response() {
    let mut br = Bridge::new();

    // Thinking → Idle with response
    let thinking = claude::ClaudeScreen {
        state: ClaudeState::Thinking, process: DetectedProcess::ClaudeCode,
        response: None, spinner_text: None, tool_block: None,
        login_url: None, awaiting_code: false, login_success: false,
    };
    br.on_screen_update(&thinking);

    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle, process: DetectedProcess::ClaudeCode,
        response: Some("answer".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };

    // First Idle call — starts settle timer
    let events = br.on_screen_update(&idle);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));

    // Wait for settle
    std::thread::sleep(std::time::Duration::from_millis(600));

    // Second Idle call — timer elapsed, response emitted
    let events = br.on_screen_update(&idle);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));

    // Third Idle call — no duplicate
    let events = br.on_screen_update(&idle);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));
}
