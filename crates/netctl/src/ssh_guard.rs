//! SSH protection guard.
//!
//! Rejects a user rule that would cut the current admin session by matching
//! a DROP / REJECT on the SSH port (default 22) against INPUT or FORWARD.
//! The check is intentionally conservative: false positives are preferable
//! to locking the operator out, and the caller can always re-submit with
//! `force=true`.

use crate::model::RuleSpec;

/// Minimal DSL describing the ssh-guard policy. Defaults to TCP/22 on INPUT
/// or FORWARD.
#[derive(Debug, Clone)]
pub struct SshGuard {
    pub port: u16,
    pub chains: Vec<&'static str>,
}

impl Default for SshGuard {
    fn default() -> Self {
        Self {
            port: 22,
            chains: vec!["INPUT", "FORWARD"],
        }
    }
}

impl SshGuard {
    /// Returns `Some(reason)` when the rule would potentially block the
    /// current SSH session; the caller decides whether to require `force`.
    pub fn evaluate(&self, spec: &RuleSpec) -> Option<String> {
        // Scope to the filter table — mangle/nat rules aren't used to block
        // SSH in a way the guard can reason about.
        if !spec.table.eq_ignore_ascii_case("filter") {
            return None;
        }
        let chain_upper = spec.chain.to_ascii_uppercase();
        if !self
            .chains
            .iter()
            .any(|c| c.eq_ignore_ascii_case(&chain_upper))
        {
            return None;
        }
        let tokens: Vec<String> = spec
            .spec
            .split_whitespace()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        if !matches_ssh_port(&tokens, self.port) {
            return None;
        }
        if !mentions_drop_or_reject(&tokens) {
            return None;
        }
        Some(format!(
            "would block ssh ({}/tcp) on chain {}",
            self.port, spec.chain
        ))
    }
}

fn matches_ssh_port(tokens: &[String], port: u16) -> bool {
    // Any of: --dport 22, --destination-port 22, --dports ...,22,..., ssh by
    // name.
    let port_str = port.to_string();
    let mut i = 0;
    while i + 1 < tokens.len() {
        let head = tokens[i].as_str();
        let next = tokens[i + 1].as_str();
        if matches!(head, "--dport" | "--destination-port")
            && (next == port_str || next.eq_ignore_ascii_case("ssh"))
        {
            return true;
        }
        if (head == "--dports" || head == "--destination-ports")
            && next
                .split(',')
                .any(|p| p.trim() == port_str || p.trim().eq_ignore_ascii_case("ssh"))
        {
            return true;
        }
        i += 1;
    }
    false
}

fn mentions_drop_or_reject(tokens: &[String]) -> bool {
    let mut iter = tokens.iter().peekable();
    while let Some(tok) = iter.next() {
        if tok == "-j" || tok == "--jump" {
            if let Some(next) = iter.next() {
                if matches!(next.as_str(), "drop" | "reject") {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_drop_on_ssh() {
        let guard = SshGuard::default();
        let rule = RuleSpec::new("filter", "INPUT", "-p tcp --dport 22 -j DROP");
        assert!(guard.evaluate(&rule).is_some());
    }

    #[test]
    fn allows_accept_on_ssh() {
        let guard = SshGuard::default();
        let rule = RuleSpec::new("filter", "INPUT", "-p tcp --dport 22 -j ACCEPT");
        assert!(guard.evaluate(&rule).is_none());
    }

    #[test]
    fn ignores_unrelated_chains() {
        let guard = SshGuard::default();
        let rule = RuleSpec::new("filter", "OUTPUT", "-p tcp --dport 22 -j DROP");
        assert!(guard.evaluate(&rule).is_none());
    }

    #[test]
    fn catches_dports_range() {
        let guard = SshGuard::default();
        let rule = RuleSpec::new(
            "filter",
            "INPUT",
            "-p tcp -m multiport --dports 22,80 -j REJECT",
        );
        assert!(guard.evaluate(&rule).is_some());
    }
}
