//! The machines a new pane can run on: this one, and tuios's [hosts] table.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Machine {
    /// None is this machine. Otherwise the name in the [hosts] table.
    pub host: Option<String>,
    pub label: String,
    pub detail: String,
    /// good, slow, stalled, reconnecting, signin or down. Empty for this machine.
    pub health: String,
    pub ready: bool,
}

/// This machine first, then each host from a bridge "hosts" event's `hosts`.
pub fn from_event(hosts: &Value) -> Vec<Machine> {
    let mut list = vec![Machine {
        host: None,
        label: "This Mac".to_string(),
        detail: "Runs here".to_string(),
        health: String::new(),
        ready: true,
    }];
    for h in hosts["hosts"].as_array().into_iter().flatten() {
        let Some(name) = h["name"].as_str() else { continue };
        let ready = h["status"] == "up";
        let detail = if ready {
            match h["rtt_ms"].as_i64() {
                Some(ms) if ms > 0 => format!("{ms} ms"),
                _ => "Connected".to_string(),
            }
        } else {
            h["reason"]
                .as_str()
                .filter(|r| !r.is_empty())
                .or(h["status"].as_str())
                .unwrap_or("Not connected")
                .to_string()
        };
        list.push(Machine {
            host: Some(name.to_string()),
            label: name.to_string(),
            detail,
            health: h["health"].as_str().unwrap_or_default().to_string(),
            ready,
        });
    }
    list
}

/// The picker's selection, which only ever rests on a machine that can take a pane.
#[derive(Debug, Clone, Default)]
pub struct MachinePicker {
    pub selected: usize,
}

impl MachinePicker {
    /// Moves to the next ready machine, wrapping at the ends.
    pub fn step(&mut self, machines: &[Machine], by: i32) {
        let n = machines.len() as i32;
        if n == 0 {
            return;
        }
        let mut at = self.selected as i32;
        for _ in 0..n {
            at = (at + by).rem_euclid(n);
            if machines[at as usize].ready {
                self.selected = at as usize;
                return;
            }
        }
    }

    pub fn current<'a>(&self, machines: &'a [Machine]) -> Option<&'a Machine> {
        machines.get(self.selected).filter(|m| m.ready)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn machines() -> Vec<Machine> {
        from_event(&json!({"live": true, "hosts": [
            {"name": "build", "addr": "build", "status": "up", "health": "good", "rtt_ms": 45},
            {"name": "gpu", "status": "unreachable", "health": "down", "reason": "No route to host"},
            {"name": "build", "status": "up", "health": "slow"}
        ]}))
    }

    #[test]
    fn this_machine_comes_first_and_needs_no_host() {
        let m = machines();
        assert_eq!(m[0].host, None);
        assert_eq!(m[1].host.as_deref(), Some("build"));
        assert_eq!(m[1].detail, "45 ms");
    }

    #[test]
    fn with_no_hosts_only_this_machine_is_listed() {
        assert_eq!(from_event(&json!({"hosts": []})).len(), 1);
        assert_eq!(from_event(&json!({"live": false})).len(), 1);
    }

    #[test]
    fn a_host_that_is_down_says_why_and_is_not_ready() {
        let m = machines();
        assert!(!m[2].ready);
        assert_eq!(m[2].detail, "No route to host");
    }

    #[test]
    fn the_selection_skips_a_machine_that_is_down() {
        let m = machines();
        let mut p = MachinePicker { selected: 1 };
        p.step(&m, 1);
        assert_eq!(p.current(&m).unwrap().label, "build");
        p.step(&m, 1);
        assert_eq!(p.current(&m).unwrap().label, "This Mac");
        p.step(&m, -1);
        assert_eq!(p.current(&m).unwrap().label, "build");
    }

    #[test]
    fn a_selection_left_on_a_machine_that_went_down_picks_nothing() {
        let m = machines();
        assert_eq!(MachinePicker { selected: 2 }.current(&m), None);
    }
}
