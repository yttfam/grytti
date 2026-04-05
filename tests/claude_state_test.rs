#[path = "../src/claude.rs"]
mod claude;

use claude::{parse_screen, ClaudeState, DetectedProcess};

// === State detection from bottom-bar indicators ===
// Bug fix: tool use markers (Bash(, Read() on screen were falsely overriding idle state

#[test]
fn idle_with_shortcuts_hint() {
    let screen = r#"
⏺ Hello! How can I help you?

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::Idle);
    assert_eq!(parsed.process, DetectedProcess::ClaudeCode);
}

#[test]
fn thinking_with_spinner() {
    let screen = r#"
❯ What is 2+2?

✶ Contemplating…

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  esc to interrupt
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::Thinking);
}

#[test]
fn thinking_with_esc_to_interrupt() {
    // Even without a visible spinner word, "esc to interrupt" means thinking
    let screen = r#"
❯ do something

⏺ Bash(ls -la)
  ⎿  total 42

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  esc to interrupt
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::Thinking);
}

// Bug fix: old tool markers on screen (Bash(, Read() were setting ToolUse
// even after the tool finished and response was visible
#[test]
fn idle_after_tool_use_completes() {
    let screen = r#"
❯ turn on the lights

⏺ Bash(curl -s -X POST http://hass:8123/api/services/switch/turn_on ...)
  ⎿  [{"entity_id": "switch.living_room"}]

⏺ Done! Living room lights are on.

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    // Must be Idle, NOT ToolUse — the "? for shortcuts" is authoritative
    assert_eq!(parsed.state, ClaudeState::Idle);
}

#[test]
fn idle_with_multiple_tool_calls_on_screen() {
    let screen = r#"
⏺ Read(src/main.rs)
  ⎿  fn main() { ... }

⏺ Edit(src/main.rs)
  ⎿  Applied edit

⏺ Bash(cargo test)
  ⎿  test result: ok

⏺ All done — tests pass and code is updated.

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::Idle);
}

// === Response extraction ===

#[test]
fn extract_simple_response() {
    let screen = r#"
❯ What is 2+2?

⏺ 4

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.response.as_deref(), Some("4"));
}

// Bug fix: response extractor was only getting last ⏺ block,
// missing tool call output. Now extracts full turn.
#[test]
fn extract_response_with_tool_calls() {
    let screen = r#"
❯ check uptime

⏺ Bash(uptime)
  ⎿  12:30 up 5 days

⏺ Server has been up for 5 days.

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    let resp = parsed.response.unwrap();
    // Full turn includes both tool call and text response
    assert!(resp.contains("Bash(uptime)"));
    assert!(resp.contains("12:30 up 5 days"));
    assert!(resp.contains("Server has been up for 5 days"));
}

#[test]
fn extract_only_latest_turn() {
    let screen = r#"
❯ first question

⏺ first answer

❯ second question

⏺ second answer

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    let resp = parsed.response.unwrap();
    // Should NOT contain the first turn
    assert!(!resp.contains("first answer"));
    assert!(resp.contains("second answer"));
}

// === Not logged in detection ===

#[test]
fn not_logged_in_state() {
    let screen = r#"
❯ hello

  ⎿  Not logged in · Please run /login

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts                                    Not logged in · Run /login
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::NotLoggedIn);
}

// === Login prompt detection ===

#[test]
fn login_menu_detected() {
    let screen = r#"
❯ /login

────────────────────────────────────────────────────────────────────────────────
  Login

   Select login method:

   ❯ 1. Claude account with subscription

     2. Anthropic Console account

  Esc to cancel
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.state, ClaudeState::LoginPrompt);
}

// === OAuth URL extraction ===
// Bug fix: URL wraps across multiple grid lines

#[test]
fn extract_oauth_url() {
    let screen = r#"
  Login

   Browser didn't open? Use the url below to sign in (c to copy)

  https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d
  9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.c
  laude.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key&code_challenge
  =abc123&code_challenge_method=S256&state=xyz789

   Paste code here if prompted >

  Esc to cancel
"#;
    let parsed = parse_screen(screen);
    assert!(parsed.login_url.is_some());
    let url = parsed.login_url.unwrap();
    assert!(url.starts_with("https://claude.com/cai/oauth/authorize"));
    assert!(url.contains("state=xyz789"));
    assert!(parsed.awaiting_code);
}

// === Login success detection ===

#[test]
fn login_success_detected() {
    let screen = r#"
  Login

   Login successful

  Esc to cancel
"#;
    let parsed = parse_screen(screen);
    assert!(parsed.login_success);
}

// === Process detection ===

#[test]
fn detect_claude_code_process() {
    let screen = r#"
⏺ Hello!

────────────────────────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────────────────────────
  ? for shortcuts
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.process, DetectedProcess::ClaudeCode);
}

#[test]
fn detect_shell_process() {
    let screen = r#"
cali@mista:~$ ls
Desktop  Documents  Downloads
cali@mista:~$
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.process, DetectedProcess::Shell);
}

#[test]
fn detect_zsh_shell() {
    let screen = r#"
cali@calimini perso % cd grytti
cali@calimini grytti %
"#;
    let parsed = parse_screen(screen);
    assert_eq!(parsed.process, DetectedProcess::Shell);
}
