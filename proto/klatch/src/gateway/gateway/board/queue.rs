//! The merge queue section of the board (#864).

use super::*;

impl<S: Store> Gateway<S> {
    /// The merge queue of a room (#864) and who holds `merge:main`. The queue words are read by prefix at any depth;
    /// the state words count only from an integrator or the human, so the role of each distinct sender of one is
    /// looked up now (a role granted or removed later applies to the words already posted).
    pub(super) fn merge_queue_of(
        &self,
        room: &str,
        now: i64,
    ) -> Result<(Vec<crate::domain::QueueEntry>, Option<String>)> {
        let words = self.store.words(room, &crate::domain::QUEUE_WORDS)?;
        let claims = self.store.claims(room, now)?;
        let merge_holder = claims
            .iter()
            .find(|c| crate::domain::is_merge_claim(&c.path))
            .map(|c| c.owner.clone());
        if words.is_empty() {
            return Ok((Vec::new(), merge_holder));
        }
        let mut vouched: Vec<String> = Vec::new();
        let mut asked: Vec<&str> = Vec::new();
        for m in words
            .iter()
            .filter(|m| !m.body.starts_with(crate::domain::READY_PREFIX))
        {
            let who = m.sender.as_str();
            if asked.contains(&who) {
                continue;
            }
            asked.push(who);
            if who == "admin" || self.role_of(who)? == Some(Role::Integrator) {
                vouched.push(who.to_string());
            }
        }
        let mut queue = crate::domain::merge_queue(&words, &|who| vouched.iter().any(|v| v == who));
        for e in &mut queue {
            let path = e.path();
            e.owner = claims
                .iter()
                .find(|c| c.path == path)
                .map(|c| c.owner.clone());
        }
        Ok((queue, merge_holder))
    }
}
