//! An outer action shares one history key across its document edits and nested calls.
use super::{EngineError, Result, Session};

#[derive(Clone, Debug)]
pub(crate) struct ActionHistory {
    pub key: String,
    pub label: String,
}

pub(super) fn begin(s: &mut Session, name: &str) -> Result<()> {
    if !s.prefs().actions.create_single_history_state {
        return Ok(());
    }
    let next = s.next_action_history.checked_add(1).ok_or_else(|| EngineError::Other("action history identifier exhausted".into()))?;
    s.next_action_history = next;
    // A caller-supplied coalesce key must never join the first edit to a previous gesture.
    for st in &mut s.docs {
        st.coalesce = None;
    }
    s.action_history = Some(ActionHistory { key: format!("action-history:{next}"), label: format!("Action: {name}") });
    Ok(())
}

pub(super) fn finish(s: &mut Session) {
    let Some(group) = s.action_history.take() else { return };
    for st in &mut s.docs {
        if st.coalesce.as_deref() == Some(group.key.as_str()) {
            st.coalesce = None;
        }
    }
}
