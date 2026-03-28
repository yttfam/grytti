#[path = "../src/login.rs"]
mod login;
#[path = "../src/claude.rs"]
mod claude;
#[path = "../src/grid.rs"]
mod grid;
#[path = "../src/parser.rs"]
mod parser;
#[path = "../src/api.rs"]
mod api;
#[path = "../src/telegram.rs"]
mod telegram;

use login::{LoginFlow, LoginState};

// === Login flow state machine ===

#[test]
fn login_flow_starts_idle() {
    let flow = LoginFlow::new();
    assert_eq!(flow.state, LoginState::Idle);
    assert!(!flow.is_waiting_for_code());
}

#[test]
fn login_flow_waiting_for_code() {
    let mut flow = LoginFlow::new();
    flow.state = LoginState::WaitingForCode;
    assert!(flow.is_waiting_for_code());
}

#[test]
fn login_flow_reset() {
    let mut flow = LoginFlow::new();
    flow.state = LoginState::WaitingForCode;
    flow.reset();
    assert_eq!(flow.state, LoginState::Idle);
    assert!(!flow.is_waiting_for_code());
}
