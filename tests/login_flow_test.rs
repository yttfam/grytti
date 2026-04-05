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
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&screen);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Thinking)));
}

#[test]
fn bridge_emits_response_on_thinking_to_idle() {
    let mut br = Bridge::new();

    // Thinking
    let thinking = claude::ClaudeScreen {
        state: ClaudeState::Thinking,
        process: DetectedProcess::ClaudeCode,
        response: None,
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    br.on_screen_update(&thinking);

    // Idle with response — should emit immediately (no timer)
    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle,
        process: DetectedProcess::ClaudeCode,
        response: Some("Hello!".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&idle);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Response(t) if t == "Hello!")));
}

#[test]
fn bridge_no_response_without_thinking() {
    let mut br = Bridge::new();

    // Idle with response but no Thinking first — should NOT emit
    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle,
        process: DetectedProcess::ClaudeCode,
        response: Some("surprise!".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&idle);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));
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

    // Same process again — no duplicate
    let events = br.on_screen_update(&screen);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::ProcessChanged(_))));
}

#[test]
fn bridge_no_duplicate_response() {
    let mut br = Bridge::new();

    // Thinking → Idle → Response
    br.on_screen_update(&claude::ClaudeScreen {
        state: ClaudeState::Thinking, process: DetectedProcess::ClaudeCode,
        response: None, spinner_text: None, tool_block: None,
        login_url: None, awaiting_code: false, login_success: false,
    });

    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle, process: DetectedProcess::ClaudeCode,
        response: Some("answer".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&idle);
    assert!(events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));

    // Same idle again — no duplicate (saw_thinking was reset)
    let events = br.on_screen_update(&idle);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))));
}

#[test]
fn bridge_rejects_spinner_response() {
    let mut br = Bridge::new();

    br.on_screen_update(&claude::ClaudeScreen {
        state: ClaudeState::Thinking, process: DetectedProcess::ClaudeCode,
        response: None, spinner_text: None, tool_block: None,
        login_url: None, awaiting_code: false, login_success: false,
    });

    // Idle but response looks like spinner
    let idle = claude::ClaudeScreen {
        state: ClaudeState::Idle, process: DetectedProcess::ClaudeCode,
        response: Some("· Incubating…".to_string()),
        spinner_text: None, tool_block: None, login_url: None,
        awaiting_code: false, login_success: false,
    };
    let events = br.on_screen_update(&idle);
    assert!(!events.iter().any(|e| matches!(e, BridgeEvent::Response(_))),
        "spinner text should not be emitted as Response");
}
