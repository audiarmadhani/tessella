//! Tells the person when an agent needs them and they are looking elsewhere: a Mac
//! notification, and the number on the Dock icon.

use super::*;
use tessella::notify;

/// A notice replaces an older one for its pane, and this keeps one change, which can
/// reach here as both an Inbox item and an agent state, from sounding twice.
const QUIET: Duration = Duration::from_secs(3);

#[derive(Default)]
pub(super) struct Notices {
    /// Nothing is news until the first look at what is already there.
    ready: bool,
    states: HashMap<String, String>,
    items: HashSet<String>,
    sounded: HashMap<String, Instant>,
    badge: usize,
}

/// What a notice's id holds, which a click comes back with.
fn notice_id(session: &str, window: &str) -> String {
    format!("{session}\t{window}")
}

/// A change of state worth telling someone about, in words.
fn state_news(state: &str, done_too: bool) -> Option<&'static str> {
    match state {
        "errored" => Some("Hit an error"),
        "done" if done_too => Some("Finished"),
        _ => None,
    }
}

impl Tessella {
    pub(super) fn send_notices(&mut self, window: &Window) {
        for id in notify::take_clicks() {
            if let Some((session, pane)) = id.split_once('\t') {
                let (session, pane) = (session.to_string(), pane.to_string());
                self.focus_anywhere(&session, &pane);
            }
        }
        let waiting = self.fleet.inbox.iter().filter(|i| i.waits()).count();
        if waiting != self.notices.badge {
            self.notices.badge = waiting;
            notify::set_badge(waiting);
        }
        let settings = self.settings.notifications.clone();
        if !settings.enabled {
            return;
        }

        let mut news: Vec<(String, String, String)> = Vec::new();
        let items: HashSet<String> = self.fleet.inbox.iter().filter(|i| i.waits()).map(|i| i.id.clone()).collect();
        for item in self.fleet.inbox.iter().filter(|i| i.waits() && !self.notices.items.contains(&i.id)) {
            let body = if item.summary.is_empty() { "Needs you".to_string() } else { item.summary.clone() };
            news.push((notice_id(&item.session, &item.window), item.name.clone(), body));
        }
        self.notices.items = items;

        let mut states = HashMap::new();
        for agent in &self.fleet.agents {
            let id = notice_id(&agent.session, &agent.window);
            let changed = self.notices.states.get(&id) != Some(&agent.state);
            if let Some(words) = state_news(&agent.state, settings.done).filter(|_| changed) {
                let body = if agent.message.is_empty() { words.to_string() } else { format!("{words}: {}", agent.message) };
                news.push((id.clone(), agent.name.clone(), body));
            }
            states.insert(id, agent.state.clone());
        }
        self.notices.states = states;

        // The first look is what was already true, with nothing new to tell.
        if !self.notices.ready {
            self.notices.ready = !self.fleet.agents.is_empty() || !self.fleet.inbox.is_empty();
            return;
        }
        let in_view = notice_id(&self.state.session, &self.state.focused);
        for (id, title, body) in news {
            if window.is_window_active() && id == in_view {
                continue;
            }
            let quiet = self.notices.sounded.get(&id).is_some_and(|t| t.elapsed() < QUIET);
            if !quiet {
                self.notices.sounded.insert(id.clone(), Instant::now());
                notify::post(&id, &title, &body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finish_is_news_only_when_asked_for() {
        assert_eq!(state_news("done", true), Some("Finished"));
        assert_eq!(state_news("done", false), None);
        assert_eq!(state_news("errored", false), Some("Hit an error"));
        assert_eq!(state_news("working", true), None);
    }
}
