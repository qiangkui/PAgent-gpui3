//! A step's detail block, the workflow status line, and the composer settings.

use super::*;

impl DshFlowApp {
    /// Pick one combined permission level for this conversation.
    pub(crate) fn set_tool_mode(&mut self, mode: ToolMode, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            mode.apply(chat);
        }
        self.close_composer_menus();
        self.save_chats();
        cx.notify();
    }

    /// How many model round-trips this conversation's questions may take.
    ///
    /// Chosen from the composer's menu, which holds several settings at once, so
    /// the menu stays open: the figure is one of a row of chips, and closing on
    /// each pick would make setting two of them a four-step job.
    pub(crate) fn set_max_rounds(&mut self, value: u32, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.max_rounds = value.clamp(1, crate::config::MAX_ROUNDS_CHOICE);
        }
        self.save_chats();
        cx.notify();
    }

    /// How many tool calls this conversation's questions may make.
    pub(crate) fn set_max_calls(&mut self, value: u32, cx: &mut Context<Self>) {
        if let Some(chat) = self.chats.get_mut(self.active_chat) {
            chat.max_calls = value.clamp(1, crate::config::MAX_CALLS_CHOICE);
        }
        self.save_chats();
        cx.notify();
    }

    /// Close every menu that floats over the composer.
    pub(crate) fn close_composer_menus(&mut self) {
        self.model_menu = false;
        self.tools_menu = false;
        self.mode_menu = false;
        self.attach_menu = false;
        self.distill_menu = false;
    }
}
