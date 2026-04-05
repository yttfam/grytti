use crate::claude::{ClaudeScreen, ClaudeState, DetectedProcess, PermissionInfo};

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
    /// Permission prompt — user needs to approve/deny
    PermissionPrompt(PermissionInfo),
}

/// Spinner symbols that should never appear in responses
const SPINNER_CHARS: &[char] = &['✶', '✸', '✹', '✺', '✷', '✵', '✳', '✢', '·', '✻', '✽'];

/// Tracks screen state and emits events on meaningful transitions.
/// No timers, no settle delays. Pure state machine.
pub struct Bridge {
    pub last_state: ClaudeState,
    pub last_process: DetectedProcess,
    pub last_sent_response: String,
    saw_thinking: bool,
    /// Track last permission to avoid re-emitting the same prompt
    last_permission_key: Option<String>,
}

impl Bridge {
    pub fn new() -> Self {
        Self {
            last_state: ClaudeState::Unknown,
            last_process: DetectedProcess::Unknown,
            last_sent_response: String::new(),
            saw_thinking: false,
            last_permission_key: None,
        }
    }

    pub fn on_screen_update(&mut self, screen: &ClaudeScreen) -> Vec<BridgeEvent> {
        let mut events = Vec::new();

        // Track thinking
        if screen.state == ClaudeState::Thinking {
            self.saw_thinking = true;
            events.push(BridgeEvent::Thinking);
        }

        // Permission prompt — emit once per unique prompt
        if screen.state == ClaudeState::PermissionPrompt {
            if let Some(ref perm) = screen.permission {
                let key = format!("{}:{}", perm.tool, perm.options.len());
                if self.last_permission_key.as_ref() != Some(&key) {
                    events.push(BridgeEvent::PermissionPrompt(perm.clone()));
                    self.last_permission_key = Some(key);
                }
            }
        } else {
            // Left permission state — reset
            if self.last_permission_key.is_some() {
                self.last_permission_key = None;
            }
        }

        // Response: emit on Thinking → Idle transition with valid content.
        if screen.state == ClaudeState::Idle && self.saw_thinking {
            if let Some(ref response) = screen.response {
                if !response.is_empty()
                    && *response != self.last_sent_response
                    && !looks_like_spinner(response)
                {
                    let to_send = if !self.last_sent_response.is_empty()
                        && response.starts_with(&self.last_sent_response)
                    {
                        let new_part = response[self.last_sent_response.len()..].trim();
                        if new_part.is_empty() || looks_like_spinner(new_part) {
                            None
                        } else {
                            Some(new_part.to_string())
                        }
                    } else {
                        Some(response.clone())
                    };

                    if let Some(text) = to_send {
                        events.push(BridgeEvent::Response(text));
                    }
                    self.last_sent_response = response.clone();
                    self.saw_thinking = false;
                }
            }
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

/// Quick check if text looks like spinner content rather than a real response.
fn looks_like_spinner(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.lines().count() <= 2 {
        if let Some(first_char) = trimmed.chars().next() {
            if SPINNER_CHARS.contains(&first_char) {
                return true;
            }
        }
    }
    false
}
