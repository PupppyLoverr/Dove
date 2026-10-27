//! The engine's attention model (`dove_core::attention`) as the app reads
//! it: one place that answers "what is this thread doing, what does it
//! need", and acts on it, for the sidebar's Needs you, Voice Mode and the
//! notch. It reads the engine first, so it works for threads with no view
//! open, and falls back to the view for a thread the engine hasn't seen
//! live since launch (one loaded from its transcript).

use dove_core::proto::{PermissionDecision, PermissionRequest, TaskId};
use gpui::{App, Context, Window};

use crate::shell::Shell;
use crate::transcript::VoiceState;

impl Shell {
    /// Repaints the shell when what the sidebar shows changes (a status, an
    /// approval arriving or going), including for threads with no open view.
    /// Activity lines and streaming text don't repaint it.
    pub fn watch_attention(&mut self, cx: &mut Context<Self>) {
        let rx = self.engine.attention().subscribe();
        let attention = self.engine.attention().clone();
        let signature = move || {
            let mut sig: Vec<(TaskId, dove_core::proto::TaskStatus, Vec<String>)> = attention
                .all()
                .into_iter()
                .map(|(id, s)| (id, s.status, s.pending.iter().map(|p| p.request_id.clone()).collect()))
                .collect();
            sig.sort_by_key(|(id, _, _)| id.to_string());
            sig
        };
        let mut last = signature();
        cx.spawn(async move |this, cx| {
            while rx.recv_async().await.is_ok() {
                let now = signature();
                if now != last {
                    last = now;
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// What `id` is doing and needs.
    pub fn task_state(&self, id: TaskId, cx: &App) -> VoiceState {
        if let Some(s) = self.engine.attention().get(id) {
            return VoiceState {
                live: s.live,
                capabilities: s.capabilities,
                pending: s.pending,
                todos: s.todos,
                status_text: s.activity,
                last_error: s.last_error,
            };
        }
        self.threads.get(&id).map(|v| v.read(cx).chat.read(cx).voice_state()).unwrap_or_default()
    }

    /// The request a Needs-you row shows: the newest undecided one.
    pub fn task_pending(&self, id: TaskId, cx: &App) -> Option<PermissionRequest> {
        match self.engine.attention().get(id) {
            Some(s) if s.live => s.pending.last().cloned(),
            _ => self.threads.get(&id).and_then(|v| v.read(cx).chat.read(cx).pending()),
        }
    }

    /// The agent's latest reply as plain text, its last `max` characters.
    pub fn task_reply(&self, id: TaskId, max: usize, cx: &App) -> String {
        match self.engine.attention().get(id) {
            Some(s) if !s.reply.trim().is_empty() => s.reply_tail(max),
            _ => self.threads.get(&id).map(|v| v.read(cx).chat.read(cx).last_reply(max)).unwrap_or_default(),
        }
    }

    /// Answers a permission request: through the engine when the session is
    /// live (whoever holds it), else through the thread's view.
    pub fn task_decide(
        &mut self,
        id: TaskId,
        request_id: String,
        decision: PermissionDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.engine.decide(id, request_id.clone(), decision) {
            return;
        }
        if let Some(chat) = self.chat_for(id, window, cx) {
            chat.update(cx, |c, cx| c.decide_from_outside(request_id, decision, cx));
        }
    }

    /// Stops the current turn.
    pub fn task_interrupt(&mut self, id: TaskId, window: &mut Window, cx: &mut Context<Self>) {
        if self.engine.interrupt(id) {
            return;
        }
        if let Some(chat) = self.chat_for(id, window, cx) {
            chat.update(cx, |c, cx| c.interrupt(cx));
        }
    }

    /// Sends like the composer's Send: to the live session through the
    /// engine, or resumes the thread through its view (which then holds the
    /// new session).
    pub fn task_send(&mut self, id: TaskId, text: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.engine.send(id, text.clone()) {
            return true;
        }
        let Some(chat) = self.chat_for(id, window, cx) else { return false };
        chat.update(cx, |c, cx| c.submit(text, window, cx));
        true
    }

    /// The thread's chat view, made (not shown) if it isn't open.
    pub fn chat_for(
        &mut self,
        id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::Entity<crate::transcript::TaskView>> {
        if !self.threads.contains_key(&id) {
            let task = self.task(id)?.clone();
            let project = self.project(task.project_id)?.clone();
            self.make_thread(task, project, None, window, cx);
        }
        self.threads.get(&id).map(|v| v.read(cx).chat.clone())
    }
}
