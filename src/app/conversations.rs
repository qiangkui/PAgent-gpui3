//! Saving conversations, filing them under a category, and switching between them.

use super::*;

impl DshFlowApp {
    /// Write every conversation into the data directory, one folder each.
    ///
    /// Called after anything that changes a transcript, so a crash or a closed
    /// window loses at most the answer in flight. A failed write is reported in
    /// the banner but never interrupts the conversation.
    pub(crate) fn save_chats(&mut self) {
        let stored: Vec<crate::chats::StoredChat> = self
            .chats
            .iter()
            .map(|chat| crate::chats::StoredChat {
                title: chat.title.clone(),
                dir: chat.dir.clone(),
                category: chat.category.clone(),
                updated_ms: chat.updated_ms,
                details: {
                    let total = chat.messages.len();
                    let mut open: Vec<(usize, usize)> = chat
                        .details
                        .iter()
                        .copied()
                        // A message index past the end belongs to a turn a retry
                        // removed; keeping it would grow the file forever.
                        .filter(|(message, _)| *message != DRAFT_MESSAGE && *message < total)
                        .collect();
                    open.sort_unstable();
                    open
                },
                checks: {
                    let total = chat.messages.len();
                    let mut ticked: Vec<(usize, usize)> = chat
                        .checks
                        .iter()
                        .copied()
                        .filter(|(message, _)| *message != DRAFT_MESSAGE && *message < total)
                        .collect();
                    ticked.sort_unstable();
                    ticked
                },
                draft: self.chat_drafts.get(&chat.dir).cloned().unwrap_or_default(),
                draft_files: self
                    .attachment_drafts
                    .get(&chat.dir)
                    .map(|files| files.iter().map(|file| file.name.clone()).collect())
                    .unwrap_or_default(),
                pinned: chat.pinned,
                tools: chat.tools,
                read_only: chat.read_only,
                full_access: chat.full_access,
                auto_actions: chat.auto_actions,
                max_rounds: chat.max_rounds,
                max_calls: chat.max_calls,
                mode: chat.mode.clone(),
                messages: chat
                    .messages
                    .iter()
                    .map(|message| crate::chats::StoredMessage {
                        role: role_name(message.role).to_string(),
                        text: message.text.clone(),
                        failed: message.failed,
                        model: message.model.clone(),
                        elapsed_ms: message.elapsed_ms,
                        tokens: message.tokens,
                        context: message.context,
                        cache_hits: message.cache_hits,
                        rounds: message.rounds,
                        round_tokens: message.round_tokens.clone(),
                        composition: message.composition.clone(),
                        tools_tokens: message.tools_tokens,
                        interrupted: message.interrupted,
                        kind: message.kind.key().to_string(),
                        input: message.input.clone(),
                        output: message.output.clone(),
                        open: message.open,
                        call: message.call.clone(),
                        path: message.path.clone(),
                        before: message.before.clone(),
                        after: message.after.clone(),
                        existed: message.existed,
                        undoable: message.undoable,
                    })
                    .collect(),
            })
            .collect();
        if let Err(error) = crate::chats::save(&self.ai.data_dir(), &stored) {
            self.banner = Some(Banner::error(error));
        }
    }

    /// Put the open conversation's unsent text and files on disk before the
    /// window goes away, so closing the app never costs what was typed.
    pub(crate) fn save_before_close(&mut self) {
        self.stash_draft();
        // A settings change typed a moment ago is still waiting for its timer.
        self.flush_settings();
        self.save_chats();
    }

    /// Open a blank conversation and put the caret in the composer.
    pub(crate) fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_rename();
        self.confirm_delete = None;
        self.undo_delete = None;
        let on_blank_last =
            self.chat().messages.is_empty() && self.active_chat == self.chats.len() - 1;
        if !on_blank_last {
            // The unfinished message stays with the conversation it was typed in;
            // the new one opens empty.
            self.stash_draft();
            self.remember_scroll();
            let dir = next_chat_dir(&self.chats);
            let mut chat = Chat::empty_with(dir, self.chat_defaults());
            chat.category = self.open_category_id();
            self.chats.push(chat);
            self.active_chat = self.chats.len() - 1;
            self.chat_find_at = 0;
            self.chat_find_query.clear();
            self.restore_draft(self.active_chat);
            // A new conversation opens on its newest message, never expanded.
            if let Some(dir) = self
                .chats
                .get(self.active_chat)
                .map(|chat| chat.dir.clone())
            {
                self.chat_from.remove(&dir);
            }
            // Scroll offsets and parsed blocks belong to the conversation that
            // was on screen.
            self.block_scroll.borrow_mut().clear();
            self.parsed.borrow_mut().clear();
            // The new chat is the one to reopen next time.
            self.save_categories();
            self.save_chats();
        }
        self.view = View::Chat;
        self.selected_text = None;
        self.text_dragging = false;
        self.close_composer_menus();
        self.focused = Some(Field::Composer);
        window.focus(&self.composer.focus_handle().clone(), cx);
        self.start_caret_blink(cx);
        self.scroll_chat_to_newest();
        self.refresh_tasks();
        cx.notify();
    }

    /// Show an existing conversation.
    pub(crate) fn select_chat(&mut self, index: usize) {
        self.commit_rename();
        let switching = index != self.active_chat;
        if switching {
            // Each conversation keeps its own unfinished message: typing, then
            // clicking another card, must not carry the text along.
            self.stash_draft();
            self.remember_scroll();
            // The instruction editor, if open, must reload before it can save
            // into a different conversation's folder.
            self.instructions_chat = None;
        }
        self.confirm_delete = None;
        self.undo_delete = None;
        self.active_chat = index;
        self.close_composer_menus();
        self.chat_find_at = 0;
        self.chat_find_query.clear();
        // A pinned request names the table of the conversation that is closing.
        self.context_request = None;
        self.context_round = None;
        self.context_item = None;
        // A changed-file list belongs to one answer of the conversation closing.
        self.changes_open = None;
        // A running answer is watched from a new screen now; the old offset
        // must not look like the reader scrolled this one.
        for pending in &mut self.pending {
            pending.viewing = false;
        }
        if switching && index < self.chats.len() {
            self.restore_draft(index);
        }
        // A selection names blocks of the old transcript, so it cannot follow.
        self.selected_text = None;
        self.text_dragging = false;
        // Code-block scroll offsets belong to the blocks that were on screen.
        if switching {
            self.block_scroll.borrow_mut().clear();
            self.parsed.borrow_mut().clear();
        }
        self.view = View::Chat;
        // Coming back to a conversation should land where it was left; a chat
        // that has never been open (or an empty one) still opens on its newest.
        if !(switching && self.restore_scroll(index)) {
            self.scroll_chat_to_newest();
        }
        if switching {
            // Reopening the app should land on the conversation last looked at.
            self.save_categories();
        }
        // The task panel is per conversation; a second of the old list would
        // read as if the tasks moved with the card.
        self.refresh_tasks();
    }

    /// Remember where the open conversation is scrolled to.
    fn remember_scroll(&mut self) {
        let Some(dir) = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.dir.clone())
        else {
            return;
        };
        let offset = f32::from(self.chat_scroll.offset().y);
        self.chat_offsets.insert(dir, offset);
    }

    /// Put a conversation back where it was left; false when it was never open.
    ///
    /// The layout clamps the offset to the transcript's current height, so a
    /// conversation that got shorter cannot leave the view parked past its end.
    fn restore_scroll(&mut self, index: usize) -> bool {
        let Some(offset) = self
            .chats
            .get(index)
            .and_then(|chat| self.chat_offsets.get(&chat.dir))
            .copied()
        else {
            return false;
        };
        self.chat_scroll.set_offset(point(px(0.), px(offset)));
        true
    }

    /// Park whatever is typed in the composer under the open conversation.
    fn stash_draft(&mut self) {
        let Some(dir) = self
            .chats
            .get(self.active_chat)
            .map(|chat| chat.dir.clone())
        else {
            return;
        };
        if self.composer.text.trim().is_empty() {
            self.chat_drafts.remove(&dir);
        } else {
            self.chat_drafts
                .insert(dir.clone(), self.composer.text.clone());
        }
        // Attached files follow the same conversation as the text.
        let attachments = std::mem::take(&mut self.attachments);
        if attachments.is_empty() {
            self.attachment_drafts.remove(&dir);
        } else {
            self.attachment_drafts.insert(dir, attachments);
        }
    }

    /// Bring back what was last typed in `index`'s conversation, if anything.
    pub(crate) fn restore_draft(&mut self, index: usize) {
        let (text, attachments) = match self.chats.get(index) {
            Some(chat) => (
                self.chat_drafts.get(&chat.dir).cloned().unwrap_or_default(),
                self.attachment_drafts
                    .get(&chat.dir)
                    .cloned()
                    .unwrap_or_default(),
            ),
            None => (String::new(), Vec::new()),
        };
        self.composer.set_text(text);
        self.attachments = attachments;
        self.composer_recall = None;
    }

    /// Start editing a conversation's title in the rename dialog.
    pub(crate) fn start_rename(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chat) = self.chats.get(index) else {
            return;
        };
        let mut field = TextField::new(cx);
        field.set_text(chat.title.clone());
        self.renaming = Some((index, field));
        self.focused = Some(Field::ChatTitle);
        if let Some((_, field)) = self.renaming.as_ref() {
            window.focus(&field.focus_handle().clone(), cx);
        }
        self.start_caret_blink(cx);
        cx.notify();
    }

    /// Keep whatever was typed as the title, unless it is blank.
    pub(crate) fn commit_rename(&mut self) {
        let Some((index, field)) = self.renaming.take() else {
            return;
        };
        let Some(title) = rename_to(&field.text) else {
            return;
        };
        if let Some(chat) = self.chats.get_mut(index) {
            chat.title = title;
        }
        self.save_chats();
    }

    /// Drop the edit and keep the old title.
    pub(crate) fn cancel_rename(&mut self) {
        self.renaming = None;
    }

    /// Close the rename dialog, keeping or dropping what was typed.
    pub(crate) fn finish_rename(
        &mut self,
        keep: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if keep {
            self.commit_rename();
        } else {
            self.cancel_rename();
        }
        self.focused = None;
        window.focus(&self.content_focus, cx);
        cx.notify();
    }

    /// Id of the category a new conversation belongs to.
    ///
    /// The open one wins; when every category is collapsed (or the saved id is
    /// gone) the default — the first category — takes the conversation.
    pub(crate) fn open_category_id(&self) -> String {
        open_category_id(&self.categories, &self.open_category)
    }

    /// Write the categories, which one is open, and the workflow cards.
    pub(crate) fn save_categories(&mut self) {
        let stored = crate::sidebar_store::StoredSidebar {
            categories: self
                .categories
                .iter()
                .map(|category| crate::sidebar_store::StoredCategory {
                    id: category.id.clone(),
                    name: category.name.clone(),
                })
                .collect(),
            open: self.open_category.clone(),
            width: Some(self.sidebar_width),
            active: self
                .chats
                .get(self.active_chat)
                .map_or_else(String::new, |chat| chat.dir.clone()),
        };
        let dir = self.ai.data_dir();
        if let Err(error) = crate::sidebar_store::save(&dir, &stored) {
            self.banner = Some(Banner::error(error));
        }
        let workflows = crate::workflow_store::collect(
            &self.workflow_categories,
            &self.hidden_workflows,
            &self.past_runs,
        );
        if let Err(error) = crate::workflow_store::save(&dir, &workflows) {
            self.banner = Some(Banner::error(error));
        }
    }

    /// Open one category and close the others: the list is an accordion.
    pub(crate) fn toggle_category(&mut self, id: &str, cx: &mut Context<Self>) {
        self.commit_category_rename();
        self.open_category = if self.open_category == id {
            String::new()
        } else {
            id.to_string()
        };
        self.save_categories();
        cx.notify();
    }

    /// Start typing a new category; Enter files it, Escape drops it, and so does
    /// clicking anywhere else.
    pub(crate) fn start_new_category(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.commit_category_rename();
        self.commit_new_category();
        self.new_category = Some(TextField::new(cx));
        self.focused = Some(Field::NewCategory);
        if let Some(field) = self.new_category.as_ref() {
            window.focus(&field.focus_handle().clone(), cx);
        }
        self.start_caret_blink(cx);
        cx.notify();
    }

    /// File the typed category name; a blank one is dropped.
    pub(crate) fn commit_new_category(&mut self) {
        let Some(field) = self.new_category.take() else {
            return;
        };
        let Some(name) = rename_to(&field.text) else {
            return;
        };
        let id = self.next_category_id();
        self.categories.push(Category {
            id: id.clone(),
            name,
        });
        // A brand new category opens, which folds the previous one away.
        self.open_category = id;
        self.focused = None;
        self.save_categories();
    }

    /// Drop the half-typed category.
    pub(crate) fn cancel_new_category(&mut self, cx: &mut Context<Self>) {
        if self.new_category.take().is_some() {
            self.focused = None;
        }
        cx.notify();
    }

    /// Remove a category from the row menu, handing its conversations to the
    /// default one; the tree always keeps at least one folder.
    pub(crate) fn delete_category(&mut self, id: &str, cx: &mut Context<Self>) {
        self.category_menu = None;
        let Some(default) = remove_category(&mut self.categories, &mut self.chats, id) else {
            cx.notify();
            return;
        };
        if self.open_category == id {
            self.open_category = default;
        }
        if self
            .rename_category
            .as_ref()
            .is_some_and(|(editing, _)| editing == id)
        {
            self.rename_category = None;
            self.focused = None;
        }
        self.save_categories();
        self.save_chats();
        cx.notify();
    }

    /// File a conversation under another category and unfold it, so the card the
    /// user just moved stays in sight.
    pub(crate) fn move_chat(&mut self, index: usize, category: &str, cx: &mut Context<Self>) {
        self.card_menu = None;
        if let Some(chat) = self.chats.get_mut(index)
            && chat.category != category
        {
            chat.category = category.to_string();
            self.save_chats();
        }
        self.open_category = category.to_string();
        self.save_categories();
        cx.notify();
    }

    /// File a workflow card under another category; the default one drops the
    /// filing instead of storing a redundant entry.
    pub(crate) fn move_workflow(&mut self, name: &str, category: &str, cx: &mut Context<Self>) {
        self.card_menu = None;
        let default = self.categories.first().map(|category| category.id.clone());
        self.workflow_categories
            .retain(|(workflow, _)| workflow != name);
        if default.as_deref() != Some(category) {
            self.workflow_categories
                .push((name.to_string(), category.to_string()));
        }
        self.open_category = category.to_string();
        self.save_categories();
        cx.notify();
    }

    /// Take a workflow card out of the 工作区 list.
    ///
    /// The manifest stays on disk: the card is a discovery result, so removing it
    /// from the list is the reversible option; the folder is not ours to delete.
    pub(crate) fn remove_workflow(&mut self, name: &str, cx: &mut Context<Self>) {
        self.card_menu = None;
        if !self.hidden_workflows.iter().any(|hidden| hidden == name) {
            self.hidden_workflows.push(name.to_string());
            self.workflow_categories
                .retain(|(workflow, _)| workflow != name);
        }
        // Nothing can stay selected in a pane whose workflow just left the list.
        if self
            .selected_flow
            .and_then(|index| self.workflows.get(index))
            .is_some_and(|entry| entry.name == name)
        {
            self.selected_flow = None;
            self.view = View::Chat;
        }
        self.save_categories();
        cx.notify();
    }

    /// The first free `c<n>` id.
    pub(crate) fn next_category_id(&self) -> String {
        let mut index = self.categories.len() + 1;
        loop {
            let id = format!("c{index}");
            if !self.categories.iter().any(|category| category.id == id) {
                return id;
            }
            index += 1;
        }
    }

    /// Start editing a category's name; the category opens so its header stays
    /// where the pointer left it.
    pub(crate) fn start_category_rename(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(category) = self.categories.iter().find(|category| category.id == id) else {
            return;
        };
        let mut field = TextField::new(cx);
        field.set_text(category.name.clone());
        field.select_all();
        self.rename_category = Some((id.to_string(), field));
        self.open_category = id.to_string();
        self.focused = Some(Field::CategoryName);
        if let Some((_, field)) = self.rename_category.as_ref() {
            window.focus(&field.focus_handle().clone(), cx);
        }
        self.start_caret_blink(cx);
        self.save_categories();
        cx.notify();
    }

    /// Keep whatever was typed as the name, unless it is blank.
    pub(crate) fn commit_category_rename(&mut self) {
        let Some((id, field)) = self.rename_category.take() else {
            return;
        };
        let Some(name) = rename_to(&field.text) else {
            return;
        };
        if let Some(category) = self
            .categories
            .iter_mut()
            .find(|category| category.id == id)
        {
            category.name = name;
        }
        self.save_categories();
    }

    /// Drop the edit and keep the old name.
    pub(crate) fn cancel_category_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename_category.take().is_some() {
            self.focused = None;
        }
        cx.notify();
    }

    /// Open a conversation and land on one of its messages.
    ///
    /// Used by the sidebar's search results: a hit deep in a long transcript is
    /// only useful if the card opens at that message rather than at the newest.
    pub(crate) fn open_chat_at_message(&mut self, index: usize, message: usize) {
        self.select_chat(index);
        let Some((dir, total)) = self
            .chats
            .get(index)
            .map(|chat| (chat.dir.clone(), chat.messages.len()))
        else {
            return;
        };
        let message = message.min(total.saturating_sub(1));
        // Draw from the hit onward: it becomes the first row, under the chip
        // that says older messages are hidden.
        self.chat_from.insert(dir, message);
        self.chat_scroll.scroll_to_item(usize::from(message > 0));
    }

    /// Forget one conversation.
    ///
    /// Only the transcript goes — its `chats\chat-N` folder in the data
    /// directory is removed with it, while the folder under the workspace is
    /// left alone, so files the assistant wrote are never destroyed behind the
    /// user's back. A request still running for that conversation is stopped,
    /// and a request for a later one keeps pointing at its own chat after the
    /// indices shift.
    pub(crate) fn delete_chat(&mut self, index: usize, cx: &mut Context<Self>) {
        self.confirm_delete = None;
        if index >= self.chats.len() {
            return;
        }
        let removed_dir = self.chats[index].dir.clone();
        if let Some(slot) = self
            .pending
            .iter()
            .position(|pending| pending.dir == removed_dir)
        {
            let pending = self.pending.remove(slot);
            pending.request.cancel();
        }

        let was_active = self.active_chat == index;
        let removed = self.chats.remove(index);
        // The schemas belonged to the deleted conversation; nothing will ask for
        // them again.
        self.tool_schemas.remove(&removed.dir);
        self.chat_drafts.remove(&removed.dir);
        self.attachment_drafts.remove(&removed.dir);
        self.chat_from.remove(&removed.dir);
        self.chat_offsets.remove(&removed.dir);
        let placeholder = self.chats.is_empty();
        if placeholder {
            self.chats
                .push(Chat::empty_with("chat-1".to_string(), self.chat_defaults()));
        }
        self.undo_delete = Some(DeletedChat {
            index,
            chat: removed,
            placeholder,
        });

        self.active_chat = selection_after_delete(self.chats.len(), self.active_chat, index);
        if was_active {
            // The draft went with the deleted conversation; the one now on screen
            // brings its own.
            self.restore_draft(self.active_chat);
        }
        self.refresh_tasks();
        self.save_categories();
        self.save_chats();
        cx.notify();
    }

    /// Put the conversation deleted last back where it was.
    pub(crate) fn undo_delete(&mut self, cx: &mut Context<Self>) {
        let Some(undo) = self.undo_delete.take() else {
            return;
        };
        // Deleting the last conversation leaves a blank one behind; the undo
        // replaces it rather than adding a second empty card.
        if undo.placeholder && self.chats.len() == 1 && self.chats[0].messages.is_empty() {
            self.chats.clear();
        }
        let index = reinsert(&mut self.chats, undo.index, undo.chat);
        self.active_chat = index;
        self.view = View::Chat;
        self.selected_text = None;
        self.restore_draft(index);
        self.refresh_tasks();
        self.save_categories();
        self.save_chats();
        cx.notify();
    }

    /// Show one workflow's plan.
    pub(crate) fn select_flow(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected_flow = Some(index);
        self.selected_run = None;
        self.selected_step = None;
        self.view = View::Flow;
        self.rebuild_inputs(cx);
    }

    /// Resize the sidebar from a pointer position in window coordinates.
    pub(crate) fn set_sidebar_width(&mut self, x: f32, cx: &mut Context<Self>) {
        let width = x.clamp(SIDEBAR_MIN, SIDEBAR_MAX);
        if (width - self.sidebar_width).abs() > 0.25 {
            self.sidebar_width = width;
            cx.notify();
        }
    }

    /// Hold one conversation at the top of its category, or put it back.
    pub(crate) fn toggle_pin(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(index) {
            chat.pinned = !chat.pinned;
        }
        self.card_menu = None;
        self.save_chats();
        cx.notify();
    }
}

/// The order conversation cards are drawn in: pinned first, then newest first.
pub(crate) fn chat_order(chats: &[Chat]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..chats.len()).collect();
    order.sort_by(|&left, &right| {
        chats[right]
            .pinned
            .cmp(&chats[left].pinned)
            .then_with(|| chats[right].updated_ms.cmp(&chats[left].updated_ms))
            .then_with(|| right.cmp(&left))
    });
    order
}

/// What the window takes from disk besides the conversations themselves.
pub(crate) struct LoadedChats {
    /// Conversations, newest last.
    pub(crate) chats: Vec<Chat>,
    /// Unsent text per conversation folder.
    pub(crate) drafts: Vec<(String, String)>,
    /// Unsent attached-file paths per conversation folder.
    pub(crate) files: Vec<(String, Vec<String>)>,
}

/// What `sidebar.json` said, with the default category filled in.
pub(crate) struct SavedSidebar {
    /// Categories in display order; the first one is the default.
    pub(crate) categories: Vec<Category>,
    /// Id of the open category; empty when they are all collapsed.
    pub(crate) open: String,
    /// Width the panel was left at.
    pub(crate) width: Option<f32>,
    /// Folder of the conversation that was on screen.
    pub(crate) active: String,
}

/// What the workflow record said, ready for the sidebar.
pub(crate) struct FiledWorkflows {
    /// Where each workflow card was moved to, as (workflow name, category id).
    pub(crate) categories: Vec<(String, String)>,
    /// Finished runs from earlier sessions, newest first.
    pub(crate) runs: Vec<crate::workflow_store::RunSummary>,
    /// Workflow cards taken out of the 工作区 list.
    pub(crate) hidden: Vec<String>,
}

/// Read the conversations, their drafts and the attachment paths of those drafts.
pub(crate) fn load_chats(dir: &Path, defaults: ChatDefaults) -> LoadedChats {
    crate::chats::adopt(dir);
    let saved = crate::chats::load(dir);
    let (drafts, files) = drafts_of(&saved);
    LoadedChats {
        chats: load_saved(saved, defaults),
        drafts,
        files,
    }
}

/// The unsent text and attachment paths held by saved conversations.
pub(crate) fn drafts_of(
    saved: &[crate::chats::StoredChat],
) -> (Vec<(String, String)>, Vec<(String, Vec<String>)>) {
    let mut drafts: Vec<(String, String)> = Vec::new();
    let mut files: Vec<(String, Vec<String>)> = Vec::new();
    for chat in saved {
        if !chat.draft.trim().is_empty() {
            drafts.push((chat.dir.clone(), chat.draft.clone()));
        }
        if !chat.draft_files.is_empty() {
            files.push((chat.dir.clone(), chat.draft_files.clone()));
        }
    }
    (drafts, files)
}

/// Build the conversations the window works with.
///
/// An empty store still opens one blank conversation, so the composer has
/// somewhere to go on a first run.
pub(crate) fn load_saved(
    saved: Vec<crate::chats::StoredChat>,
    defaults: ChatDefaults,
) -> Vec<Chat> {
    let chats: Vec<Chat> = saved
        .into_iter()
        .map(|stored| {
            let messages = stored
                .messages
                .into_iter()
                .map(|message| {
                    let mut restored = ChatMessage::restored(
                        role_from(&message.role),
                        message.text,
                        message.failed,
                    );
                    restored.model = message.model;
                    restored.elapsed_ms = message.elapsed_ms;
                    restored.tokens = message.tokens;
                    restored.context = message.context;
                    restored.cache_hits = message.cache_hits;
                    restored.rounds = message.rounds;
                    restored.round_tokens = message.round_tokens;
                    restored.composition = message.composition;
                    restored.tools_tokens = message.tools_tokens;
                    restored.interrupted = message.interrupted;
                    restored.kind = AttachmentKind::from_key(&message.kind);
                    restored.input = message.input;
                    restored.output = message.output;
                    restored.open = message.open;
                    restored.call = message.call;
                    restored.path = message.path;
                    restored.before = message.before;
                    restored.after = message.after;
                    restored.existed = message.existed;
                    restored.undoable = message.undoable;
                    restored
                })
                .collect();
            let mut chat = Chat::restored(stored.title, stored.dir, messages);
            chat.updated_ms = stored.updated_ms;
            chat.category = stored.category;
            chat.pinned = stored.pinned;
            chat.tools = stored.tools;
            chat.read_only = stored.read_only;
            chat.full_access = stored.full_access;
            chat.auto_actions = stored.auto_actions;
            chat.max_rounds = stored.max_rounds;
            chat.max_calls = stored.max_calls;
            chat.mode = stored.mode;
            chat.details = stored.details.into_iter().collect();
            chat.checks = stored.checks.into_iter().collect();
            chat
        })
        .collect();
    let mut chats = unique_dirs(chats);
    if chats.is_empty() {
        chats.push(Chat::empty_with("chat-1".to_string(), defaults));
    }
    chats
}

/// Give every conversation a folder of its own.
///
/// Two transcript files can name the same folder (a hand copy, an interrupted
/// save). Keeping both under that name would make drafts, scroll offsets and
/// pending answers collide, so the later one moves to a fresh folder and says so.
pub(crate) fn unique_dirs(chats: Vec<Chat>) -> Vec<Chat> {
    let mut names: std::collections::HashSet<String> =
        chats.iter().map(|chat| chat.dir.clone()).collect();
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<Chat> = Vec::with_capacity(chats.len());
    for mut chat in chats {
        if chat.dir.is_empty() || !used.insert(chat.dir.clone()) {
            chat.dir = next_free_dir(&mut names);
            used.insert(chat.dir.clone());
            chat.messages.push(ChatMessage::note(
                "这个对话记录与另一个用了同一个文件夹名，已改到独立文件夹；原来那个文件夹里的文件仍在原处。",
                false,
            ));
        }
        out.push(chat);
    }
    out
}

/// The next `chat-N` name that is not taken; marks it as taken.
fn next_free_dir(names: &mut std::collections::HashSet<String>) -> String {
    let mut number = names
        .iter()
        .filter_map(|name| name.strip_prefix("chat-"))
        .filter_map(|rest| rest.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    loop {
        number += 1;
        let candidate = format!("chat-{number}");
        if !names.contains(&candidate) {
            names.insert(candidate.clone());
            return candidate;
        }
    }
}

/// How a role is stored in `chat.json`.
pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::Note => "note",
        Role::Approval => "approval",
        Role::Attachment => "attachment",
    }
}

/// Read a stored role; anything unknown is kept as a note.
pub(crate) fn role_from(name: &str) -> Role {
    match name.trim().to_ascii_lowercase().as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        "approval" => Role::Approval,
        "attachment" => Role::Attachment,
        _ => Role::Note,
    }
}

/// The folder name the next conversation takes, counting up from `chat-N`.
pub(crate) fn next_chat_dir(chats: &[Chat]) -> String {
    let highest = chats
        .iter()
        .filter_map(|chat| chat.dir.strip_prefix("chat-"))
        .filter_map(|number| number.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("chat-{}", highest + 1)
}

/// Put a deleted conversation back at `index`, returning where it landed.
pub(crate) fn reinsert(chats: &mut Vec<Chat>, index: usize, chat: Chat) -> usize {
    let index = index.min(chats.len());
    chats.insert(index, chat);
    index
}

/// A name typed into a rename box, or `None` when it is only whitespace.
pub(crate) fn rename_to(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The category a new conversation goes into: the open one when it still
/// exists, otherwise the default.
pub(crate) fn open_category_id(categories: &[Category], open: &str) -> String {
    if let Some(found) = categories.iter().find(|category| category.id == open) {
        return found.id.clone();
    }
    categories
        .first()
        .map(|category| category.id.clone())
        .unwrap_or_default()
}

/// Move every conversation whose category is gone into the default one.
pub(crate) fn file_unknown_categories(chats: Vec<Chat>, categories: &[Category]) -> Vec<Chat> {
    let default = categories
        .first()
        .map(|category| category.id.clone())
        .unwrap_or_default();
    chats
        .into_iter()
        .map(|mut chat| {
            if !categories
                .iter()
                .any(|category| category.id == chat.category)
            {
                chat.category = default.clone();
            }
            chat
        })
        .collect()
}

/// Delete one category, moving its conversations to the one that takes over.
///
/// @returns the id of the category that became the default, or `None` when the
/// category does not exist or is the last one.
pub(crate) fn remove_category(
    categories: &mut Vec<Category>,
    chats: &mut [Chat],
    id: &str,
) -> Option<String> {
    if categories.len() <= 1 {
        return None;
    }
    let index = categories.iter().position(|category| category.id == id)?;
    categories.remove(index);
    let default = categories[0].id.clone();
    for chat in chats.iter_mut() {
        if chat.category == id {
            chat.category = default.clone();
        }
    }
    Some(default)
}

/// Which conversation to show after one was deleted.
pub(crate) fn selection_after_delete(len: usize, active: usize, removed: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if removed < active {
        active - 1
    } else {
        active.min(len - 1)
    }
}

/// The width the sidebar opens at: what was saved, clamped to the handle range.
pub(crate) fn sidebar_width(width: Option<f32>) -> f32 {
    width
        .unwrap_or(SIDEBAR_DEFAULT)
        .clamp(SIDEBAR_MIN, SIDEBAR_MAX)
}

/// Read the categories and the panel state, falling back to one default category.
pub(crate) fn load_sidebar(dir: &Path) -> SavedSidebar {
    let Some(stored) = crate::sidebar_store::load(dir) else {
        return SavedSidebar {
            categories: vec![Category::default_one()],
            open: String::new(),
            width: None,
            active: String::new(),
        };
    };
    let mut categories: Vec<Category> = stored
        .categories
        .into_iter()
        .map(|category| Category {
            id: category.id,
            name: category.name,
        })
        .collect();
    if categories.is_empty() {
        categories.push(Category::default_one());
    }
    SavedSidebar {
        categories,
        open: stored.open,
        width: stored.width,
        active: stored.active,
    }
}

/// Index of the conversation that was on screen, falling back to the newest.
pub(crate) fn active_chat_index(chats: &[Chat], active: &str) -> usize {
    chats
        .iter()
        .position(|chat| chat.dir == active)
        .unwrap_or_else(|| chats.len().saturating_sub(1))
}

/// Read where workflow cards were filed, which ones are hidden, and their runs.
pub(crate) fn load_workflows(dir: &Path, categories: &[Category]) -> FiledWorkflows {
    let stored = crate::workflow_store::load(dir);
    let mut categories_out: Vec<(String, String)> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();
    let mut runs: Vec<crate::workflow_store::RunSummary> = Vec::new();
    for workflow in stored {
        if !workflow.category.is_empty()
            && categories
                .iter()
                .any(|category| category.id == workflow.category)
        {
            categories_out.push((workflow.name.clone(), workflow.category));
        }
        if workflow.hidden {
            hidden.push(workflow.name.clone());
        }
        runs.extend(workflow.runs);
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.finished_ms));
    FiledWorkflows {
        categories: categories_out,
        runs,
        hidden,
    }
}

/// The category a workflow card belongs to: where it was filed, else the default.
pub(crate) fn workflow_category(
    filings: &[(String, String)],
    categories: &[Category],
    name: &str,
) -> String {
    let default = categories
        .first()
        .map(|category| category.id.clone())
        .unwrap_or_default();
    filings
        .iter()
        .find(|(workflow, _)| workflow == name)
        .map(|(_, category)| category.clone())
        .filter(|category| categories.iter().any(|known| known.id == *category))
        .unwrap_or(default)
}
