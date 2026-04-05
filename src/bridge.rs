use crate::claude::{ClaudeScreen, ClaudeState, DetectedProcess};

/// Events the bridge emits for transports to consume.
#[derive(Debug, Clone)]
pub enum BridgeEvent {
    /// Claude finished responding — here's the full response
    Response(String),
    /// Claude is thinking — keep showing activity
    Thinking,
    /// Process changed (Claude Code / Shell)
    ProcessChanged(DetectedProcess),
    /// Shell command output (new content only)
    ShellOutput(String),
}

/// Wait this long after transitioning to Idle before emitting Response.
/// Gives streaming output time to settle.
const IDLE_SETTLE_MS: u64 = 500;

/// Tracks screen state and emits events on meaningful transitions.
/// Transport-agnostic — knows nothing about Telegram, WebSocket, etc.
pub struct Bridge {
    pub last_state: ClaudeState,
    pub last_process: DetectedProcess,
    pub last_sent_response: String,
    idle_since: Option<std::time::Instant>,
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            last_state: ClaudeState::Unknown,
            last_process: DetectedProcess::Unknown,
            last_sent_response: String::new(),
            idle_since: None,
        }
    }

    /// Process a screen update and return events for transports to handle.
    pub fn on_screen_update(&mut self, screen: &ClaudeScreen) -> Vec<BridgeEvent> {
        let mut events = Vec::new();

        // Thinking
        if screen.state == ClaudeState::Thinking {
            events.push(BridgeEvent::Thinking);
        }

        // Idle settle + response
        // Start settle timer on:
        // 1. Transition to Idle (was Thinking/Unknown)
        // 2. Response content changed while already Idle (fast answer)
        if screen.state == ClaudeState::Idle {
            let response_changed = screen.response.as_ref()
                .map_or(false, |r| !r.is_empty() && *r != self.last_sent_response);

            if self.last_state != ClaudeState::Idle || (response_changed && self.idle_since.is_none()) {
                self.idle_since = Some(std::time::Instant::now());
            }
            if let Some(since) = self.idle_since {
                if since.elapsed().as_millis() >= IDLE_SETTLE_MS as u128 {
                    if let Some(ref response) = screen.response {
                        if *response != self.last_sent_response && !response.is_empty() {
                            events.push(BridgeEvent::Response(response.clone()));
                            self.last_sent_response = response.clone();
                        }
                    }
                    self.idle_since = None;
                }
            }
        } else {
            self.idle_since = None;
        }

        // Shell mode output
        if screen.process == DetectedProcess::Shell && screen.state == ClaudeState::Unknown {
            if let Some(ref response) = screen.response {
                if *response != self.last_sent_response && !response.is_empty() {
                    let new_content = self.diff_shell_output(response);
                    if !new_content.is_empty() {
                        events.push(BridgeEvent::ShellOutput(new_content));
                    }
                    self.last_sent_response = response.clone();
                }
            }
        }

        // Process change
        if screen.process != self.last_process && screen.process != DetectedProcess::Unknown {
            events.push(BridgeEvent::ProcessChanged(screen.process.clone()));
            self.last_process = screen.process.clone();
        }

        self.last_state = screen.state.clone();
        events
    }

    /// Extract only the new part of shell output.
    fn diff_shell_output(&self, response: &str) -> String {
        if self.last_sent_response.is_empty() {
            return response.to_string();
        }
        if response.starts_with(&self.last_sent_response) {
            response[self.last_sent_response.len()..]
                .trim_start_matches('\n')
                .to_string()
        } else if let Some(idx) = response.find(&self.last_sent_response) {
            response[idx + self.last_sent_response.len()..]
                .trim_start_matches('\n')
                .to_string()
        } else {
            response.to_string()
        }
    }
}
