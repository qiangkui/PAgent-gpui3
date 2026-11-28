//! Behaviour tests for the whole application module.

use super::*;
#[test]
fn a_finished_draft_drops_its_suffix() {
    // The distill button appends 「（草稿·第N轮）」; while a placeholder is left it
    // stays, and once every step is real the card reads as the plain name.
    assert_eq!(
        super::util::workflow_display_name("新对话（草稿·第1轮）", 0),
        "新对话"
    );
    assert_eq!(
        super::util::workflow_display_name("新对话（草稿·第1轮）", 2),
        "新对话（草稿·第1轮）"
    );
    assert_eq!(
        super::util::workflow_display_name("maers-units-export", 0),
        "maers-units-export"
    );
}

#[test]
fn the_caption_buttons_are_named_by_the_pointer() {
    use super::{TITLEBAR_HEIGHT, window_button_at};
    use gpui::WindowControlArea;

    let width = 1200.;
    let bar = TITLEBAR_HEIGHT;
    // Close is the rightmost cell; the three cells are each 46px wide.
    assert_eq!(
        window_button_at(width - 1., 10., width, bar),
        Some(WindowControlArea::Close)
    );
    assert_eq!(
        window_button_at(width - 46., 10., width, bar),
        Some(WindowControlArea::Close)
    );
    assert_eq!(
        window_button_at(width - 46.5, 10., width, bar),
        Some(WindowControlArea::Max)
    );
    assert_eq!(
        window_button_at(width - 92., 10., width, bar),
        Some(WindowControlArea::Max)
    );
    assert_eq!(
        window_button_at(width - 92.5, 10., width, bar),
        Some(WindowControlArea::Min)
    );
    assert_eq!(
        window_button_at(width - 138., 10., width, bar),
        Some(WindowControlArea::Min)
    );
    // Left of the strip, above it, and past the right edge are no button.
    assert_eq!(window_button_at(width - 139., 10., width, bar), None);
    assert_eq!(window_button_at(width - 1., bar, width, bar), None);
    assert_eq!(window_button_at(width - 1., -1., width, bar), None);
    assert_eq!(window_button_at(width + 5., 10., width, bar), None);
}

#[test]
fn attachments_read_text_and_refuse_what_is_not_text() {
    let root = std::env::temp_dir().join(format!("dsh-attach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let text = root.join("note.md");
    std::fs::write(&text, "\u{feff}# 标题\n正文\n").unwrap();
    let attachment = read_attachment(&text).expect("text file");
    assert_eq!(attachment.text, "# 标题\n正文\n", "the BOM is dropped");
    assert!(!attachment.truncated);
    assert!(attachment.name.ends_with("note.md"));

    let binary = root.join("blob.bin");
    std::fs::write(&binary, [0_u8, 1, 2, 3]).unwrap();
    assert!(
        read_attachment(&binary)
            .unwrap_err()
            .contains("不是文本文件")
    );

    let empty = root.join("empty.txt");
    std::fs::write(&empty, "  \n").unwrap();
    assert!(read_attachment(&empty).unwrap_err().contains("空文件"));

    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    assert!(read_attachment(&folder).unwrap_err().contains("不是文件"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn attachments_keep_the_head_of_a_long_file() {
    let root = std::env::temp_dir().join(format!("dsh-attach-long-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let long = root.join("long.txt");
    std::fs::write(&long, "长".repeat(MAX_ATTACHMENT_CHARS + 10)).unwrap();
    let attachment = read_attachment(&long).expect("long file");
    assert!(attachment.truncated);
    assert_eq!(attachment.text.chars().count(), MAX_ATTACHMENT_CHARS);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn conversation_instructions_are_written_read_and_removed() {
    let root = std::env::temp_dir().join(format!("dsh-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("chat-1");

    let message = write_instructions(&dir, "  用中文回答 ").expect("save");
    assert!(message.contains("已保存"), "{message}");
    assert_eq!(read_instructions(&dir), "用中文回答");

    // A BOM written by Notepad is dropped on the way in.
    std::fs::write(dir.join(CHAT_RULES), "\u{feff}第二行").unwrap();
    assert_eq!(read_instructions(&dir), "第二行");

    let message = write_instructions(&dir, "   \n").expect("clear");
    assert!(message.contains("已清除"), "{message}");
    assert!(!dir.join(CHAT_RULES).exists());
    assert!(
        write_instructions(&dir, "").is_ok(),
        "clearing twice is fine"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn images_and_pdfs_travel_as_bytes() {
    let root = std::env::temp_dir().join(format!("dsh-attach-bytes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let png = root.join("shot.png");
    std::fs::write(
        &png,
        [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 1, 2],
    )
    .unwrap();
    let image = read_attachment(&png).expect("image");
    assert_eq!(image.kind, AttachmentKind::Image);
    assert_eq!(image.mime, "image/png");
    assert!(
        !image.bytes.is_empty(),
        "the bytes are kept for the request"
    );
    assert!(image.text.contains("图片"), "{}", image.text);
    assert!(!image.truncated);

    let pdf = root.join("spec.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\n1 0 obj\n").unwrap();
    let document = read_attachment(&pdf).expect("pdf");
    assert_eq!(document.kind, AttachmentKind::Pdf);
    assert_eq!(document.mime, "application/pdf");
    assert!(document.text.contains("PDF"), "{}", document.text);

    // A mail gateway may put junk before the header; the specification allows it.
    let prefixed = root.join("forwarded.pdf");
    std::fs::write(&prefixed, b"From: someone\r\n\r\n%PDF-1.5\n1 0 obj\n").unwrap();
    assert_eq!(
        read_attachment(&prefixed).expect("prefixed pdf").kind,
        AttachmentKind::Pdf
    );

    // A text file keeps its bytes empty and carries its content.
    let text = root.join("note.txt");
    std::fs::write(&text, "hello").unwrap();
    let note = read_attachment(&text).expect("text");
    assert_eq!(note.kind, AttachmentKind::Text);
    assert!(note.bytes.is_empty());
    assert_eq!(note.text, "hello");

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn history_sends_attachments_as_user_context() {
    let messages = vec![
        ChatMessage::attachment("C:\\work\\a.rs".to_string(), "fn main() {}".to_string()),
        ChatMessage::user("解释一下"),
    ];
    let turns = history_turns(&messages);
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].speaker, Speaker::User);
    assert_eq!(turns[0].text, "附件 C:\\work\\a.rs 的内容：\nfn main() {}");
    assert_eq!(turns[1].text, "解释一下");
    // The stored name is what a restart shows; the role survives the round trip.
    assert_eq!(role_name(Role::Attachment), "attachment");
    assert_eq!(role_from("attachment"), Role::Attachment);
}

#[test]
fn history_stats_count_what_the_next_request_carries() {
    let messages = vec![
        ChatMessage::attachment("a.txt".to_string(), "abc".to_string()),
        ChatMessage::user("你好"),
        ChatMessage::assistant("在"),
        ChatMessage::note("工具 · read", false),
    ];
    let (turns, chars, tokens, dropped) = history_stats(&messages);
    assert_eq!(turns, 3, "notes are not sent");
    assert_eq!(dropped, 0);
    assert!(
        chars >= 20,
        "attachment header + text and both turns: {chars}"
    );
    assert!(
        tokens >= 3,
        "each short turn costs at least one token: {tokens}"
    );
}

#[test]
fn token_estimates_follow_ascii_and_wide_characters() {
    assert_eq!(estimate_tokens(""), 0);
    assert_eq!(estimate_tokens("abcd"), 1);
    assert_eq!(estimate_tokens("abcde"), 2);
    assert_eq!(estimate_tokens("一"), 1);
    assert_eq!(estimate_tokens("中文"), 2);
    assert_eq!(estimate_tokens("a一"), 2);
}

#[test]
fn one_oversized_turn_keeps_both_ends() {
    let huge = format!("开头\n{}\n结尾", "长".repeat(MAX_HISTORY_TURN_TOKENS * 2));
    let matching = huge.chars().count();
    let turns = history_turns(&[ChatMessage::user(huge)]);
    assert_eq!(turns.len(), 1);
    assert!(turns[0].text.starts_with("开头"), "the question survives");
    assert!(turns[0].text.ends_with("结尾"), "the conclusion survives");
    assert!(turns[0].text.contains("中间省略"), "the cut is explained");
    let clipped = estimate_tokens(&turns[0].text);
    assert!(
        clipped <= MAX_HISTORY_TURN_TOKENS,
        "clipped to {clipped} tokens from {matching} characters"
    );
}

#[test]
fn a_long_transcript_opens_on_its_newest_window() {
    assert_eq!(window_start(None, 10), 0);
    assert_eq!(window_start(None, CHAT_WINDOW + 5), 5);
    assert_eq!(window_start(Some(20), 100), 20);
    assert_eq!(window_start(Some(999), 100), 100, "clamped to the list");
}

#[test]
fn a_huge_answer_narrows_the_window_by_blocks() {
    // Three small messages and one enormous answer: the answer stays, and the
    // older messages give way instead of all being re-laid every frame …but the
    // newest `min_messages` are always kept.
    let counts = [1, 1, 1, 500];
    assert_eq!(window_start_with_budget(&counts, None, 300, 2), 2);
    assert_eq!(window_start_with_budget(&counts, None, 300, 4), 0);
    // Small messages fit.
    assert_eq!(window_start_with_budget(&[5, 6, 7], None, 300, 2), 0);
    // An explicit start (the reader asked for more) is not second-guessed.
    assert_eq!(window_start_with_budget(&counts, Some(1), 300, 2), 1);
}

#[test]
fn find_highlights_every_hit_on_character_boundaries() {
    assert_eq!(find_ranges("Hello hello", "hello", 10), vec![0..5, 6..11]);
    assert_eq!(find_ranges("中文中文", "中文", 10), vec![0..6, 6..12]);
    assert_eq!(find_ranges("a😀b", "😀", 10), vec![1..5], "no split glyphs");
    assert_eq!(
        find_ranges("aaaa", "aa", 10),
        vec![0..2, 2..4],
        "no overlap"
    );
    assert_eq!(find_ranges("aaaa", "aa", 1), vec![0..2], "the limit holds");
    assert!(find_ranges("anything", "", 10).is_empty());
    assert!(find_ranges("", "x", 10).is_empty());
}

#[test]
fn exported_names_are_safe_file_names() {
    assert_eq!(export_name("统计 files/count.py"), "统计 files_count.py");
    assert_eq!(export_name("a:b?c*"), "a_b_c_");
    assert_eq!(export_name("  ...  "), "对话记录");
    assert_eq!(export_name(""), "对话记录");
    assert_eq!(export_name(&"长".repeat(60)).chars().count(), 40);
}

#[test]
fn a_stopped_answer_can_be_continued() {
    let stopped = stopped_messages("写到一半");
    assert_eq!(stopped.len(), 1);
    assert!(stopped[0].interrupted, "the partial answer is marked");

    let question = ChatMessage::user("写点东西");
    let partial = {
        let mut message = ChatMessage::assistant("写到一半");
        message.interrupted = true;
        message
    };
    assert!(continuable(&[question.clone(), partial.clone()]));
    // A failure note after it keeps the offer: the text above is still usable.
    assert!(continuable(&[
        question.clone(),
        partial.clone(),
        ChatMessage::note("超时", true)
    ]));
    // Complete answers, a lone question and an empty transcript are not.
    assert!(!continuable(&[
        question.clone(),
        ChatMessage::assistant("写完了")
    ]));
    assert!(!continuable(&[question]));
    assert!(!continuable(&[]));
    // An empty stop leaves nothing to continue.
    assert!(stopped_messages("   ").is_empty());
}

#[test]
fn a_table_copies_as_pipe_text() {
    use super::answer_block_texts;
    assert_eq!(
        answer_block_texts("| a | b |\n|---|---|\n| 1 | 2 |"),
        vec!["a | b\n1 | 2".to_string()]
    );
}

#[test]
fn a_quote_contributes_its_blocks_not_itself() {
    use super::answer_block_texts;
    // The quote takes one entry of its own (empty: the box draws no text) and
    // then the blocks inside it, in the order `block_rows` hands out numbers.
    assert_eq!(
        answer_block_texts("> - 一\n> - 二"),
        vec![String::new(), "一".to_string(), "二".to_string()]
    );
    // Blocks after the quote keep counting from there.
    assert_eq!(
        answer_block_texts("> 引用\n\n正文"),
        vec![String::new(), "引用".to_string(), "正文".to_string()]
    );
}

#[test]
fn an_answer_column_stops_at_its_maximum_and_splits_the_rest() {
    use super::{
        ANSWER_MAX_WIDTH, BUBBLE_PAD, MIN_ANSWER_TEXT, TRANSCRIPT_PAD, answer_text_width,
        transcript_pad,
    };
    // A wide pane stops at the conversation column's maximum: the spare room is
    // split between the two sides instead of stretching the lines.
    assert_eq!(
        answer_text_width(px(2000.)),
        ANSWER_MAX_WIDTH - 2. * BUBBLE_PAD
    );
    assert_eq!(transcript_pad(px(2000.)), (2000. - ANSWER_MAX_WIDTH) / 2.);
    // A narrower pane gives up its side padding first.
    assert_eq!(
        answer_text_width(px(600.)),
        600. - 2. * TRANSCRIPT_PAD - 2. * BUBBLE_PAD
    );
    assert_eq!(transcript_pad(px(600.)), TRANSCRIPT_PAD);
    // However small the window, text keeps a usable width.
    assert_eq!(answer_text_width(px(100.)), MIN_ANSWER_TEXT);
}

#[test]
fn a_failure_note_leaves_the_buttons_beside_it_inside_the_column() {
    use super::{BUBBLE_ACTIONS, BUBBLE_PAD, NOTE_MAX_WIDTH, note_width};
    // The note is the whole bubble here, so its cap is the room the row has
    // left once the retry and copy buttons are in.
    let row = |answer: f32| answer + 2. * BUBBLE_PAD;
    assert_eq!(note_width(900.), NOTE_MAX_WIDTH);
    for answer in [792., 526., 280.] {
        assert!(
            note_width(answer) + BUBBLE_ACTIONS <= row(answer),
            "a {answer} wide column leaves {} for a {} wide note",
            row(answer) - BUBBLE_ACTIONS,
            note_width(answer)
        );
    }
}

#[test]
fn a_question_bubble_leaves_the_icons_beside_it_inside_the_column() {
    use super::{BUBBLE_ACTIONS, BUBBLE_PAD, QUESTION_MAX_WIDTH, question_text_width};
    // The row a question sits in is the answer column plus the bubble's padding;
    // the pencil, the copy icon and their gaps come out of that same row.
    let bubble = |answer: f32| question_text_width(answer) + 2. * BUBBLE_PAD;
    let room = |answer: f32| answer + 2. * BUBBLE_PAD - BUBBLE_ACTIONS;
    // Wide windows stop the bubble at its own maximum, well inside the column.
    assert_eq!(bubble(900.), QUESTION_MAX_WIDTH);
    assert!(bubble(900.) < 900.);
    // Two real panes: the default window, and the narrowest one allowed.
    for answer in [792., 526., 280.] {
        assert!(
            bubble(answer) <= room(answer),
            "a {answer} wide column leaves {} for a {} wide bubble",
            room(answer),
            bubble(answer)
        );
    }
}

#[test]
fn a_selection_can_run_from_one_message_into_the_next() {
    use super::{TextPoint, TextSelection};
    let selection = TextSelection {
        anchor: TextPoint {
            message: 0,
            block: 0,
            index: 2,
        },
        focus: TextPoint {
            message: 2,
            block: 1,
            index: 3,
        },
    };
    // The block the drag started in gives up only its tail.
    assert_eq!(selection.part_of(0, 0, 10), Some(2..10));
    // Everything in between is taken whole.
    assert_eq!(selection.part_of(1, 0, 5), Some(0..5));
    assert_eq!(selection.part_of(2, 0, 4), Some(0..4));
    // The block it ended in gives up only its head.
    assert_eq!(selection.part_of(2, 1, 6), Some(0..3));
    // Nothing outside the two ends is highlighted.
    assert_eq!(selection.part_of(2, 2, 6), None);
    assert_eq!(selection.part_of(3, 0, 6), None);
}

#[test]
fn a_selection_ending_before_it_starts_reads_backwards() {
    use super::{TextPoint, TextSelection};
    let selection = TextSelection {
        anchor: TextPoint {
            message: 2,
            block: 1,
            index: 3,
        },
        focus: TextPoint {
            message: 0,
            block: 0,
            index: 2,
        },
    };
    assert_eq!(selection.part_of(0, 0, 10), Some(2..10));
    assert_eq!(selection.part_of(2, 1, 6), Some(0..3));
}

#[test]
fn an_answer_block_keeps_its_emphasis_on_the_drawn_text() {
    use super::{block_body, list_marker, markdown};
    let rich = markdown::parse_rich("- 用 `read` **读**");
    let block = &rich[0].block;
    let body = block_body(block);
    // The marker is drawn in its own column, so the body is just the words and
    // the runs cover them byte for byte.
    assert_eq!(list_marker(block), "•");
    assert_eq!(body, "用 read 读");
    assert_eq!(
        rich[0].spans.iter().map(|run| run.len).sum::<usize>(),
        body.len()
    );
    assert!(rich[0].spans.iter().any(|run| run.code));
    assert!(rich[0].spans.iter().any(|run| run.bold));
}

#[test]
fn a_task_block_draws_its_box_in_the_marker_column() {
    use super::{block_body, markdown};
    let rich = markdown::parse_rich("- [x] 用 `read` **读**");
    let body = block_body(&rich[0].block);
    // The box is drawn in the marker column like a bullet, so copying the item
    // gives its words and the runs still cover them byte for byte.
    assert_eq!(body, "用 read 读");
    assert_eq!(
        rich[0].spans.iter().map(|run| run.len).sum::<usize>(),
        body.len()
    );
}

#[test]
fn ctrl_a_covers_every_block_of_every_message() {
    use super::TextSelection;
    let selection =
        TextSelection::whole_transcript(&[(0, vec![3, 5]), (2, vec![4])]).expect("a selection");
    assert_eq!(selection.part_of(0, 0, 3), Some(0..3));
    assert_eq!(selection.part_of(0, 1, 5), Some(0..5));
    // A message between the two ends is covered from its first byte to its last;
    // tool rows and notes are never asked for, because `selected_slice` walks
    // only the entries that are drawn as text.
    assert_eq!(selection.part_of(1, 0, 7), Some(0..7));
    assert_eq!(selection.part_of(2, 0, 4), Some(0..4));
    // Nothing to select.
    assert!(TextSelection::whole_transcript(&[]).is_none());
    assert!(TextSelection::whole_transcript(&[(0, Vec::new())]).is_none());
}

#[test]
fn a_list_item_link_keeps_its_bytes() {
    use super::{block_body, markdown};
    let rich = markdown::parse_rich("- [文档](https://example.com) 之后");
    let body = block_body(&rich[0].block);
    assert_eq!(body, "文档 之后");
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].start, 0);
    assert_eq!(rich[0].links[0].end, "文档".len());
    assert_eq!(rich[0].links[0].url, "https://example.com");
}

#[test]
fn a_list_is_grouped_by_its_marker_kind() {
    use super::{ListKind, markdown};
    assert_eq!(
        ListKind::of(&markdown::Block::Bullet(0, "一".to_string())),
        Some(ListKind::Bullet)
    );
    assert_eq!(
        ListKind::of(&markdown::Block::Numbered(
            0,
            "1.".to_string(),
            "一".to_string()
        )),
        Some(ListKind::Numbered)
    );
    // A task is a bullet whose marker is a box: it belongs to the same list.
    assert_eq!(
        ListKind::of(&markdown::Block::Task(0, false, "一".to_string())),
        Some(ListKind::Bullet)
    );
    // Anything that is not an item ends the list, and a `-` list touching a
    // `1.` list is two lists.
    assert_eq!(
        ListKind::of(&markdown::Block::Paragraph("正文".to_string())),
        None
    );
    assert_ne!(ListKind::Bullet, ListKind::Numbered);
}

#[test]
fn answer_labels_name_the_model_the_time_and_the_cost() {
    use super::answer_label;
    assert_eq!(answer_label("", 0, 0), "助手");
    assert_eq!(answer_label("m", 0, 0), "助手 · m");
    assert_eq!(answer_label("", 2400, 0), "助手 · 2.4 s");
    assert_eq!(answer_label("m", 2400, 0), "助手 · m · 2.4 s");
    assert_eq!(
        answer_label("m", 2400, 845),
        "助手 · m · 2.4 s · 845 tokens"
    );
    assert_eq!(answer_label("", 0, 1200), "助手 · 1.2k tokens");
}

#[test]
fn the_chat_header_reports_spend_and_context() {
    use super::{Chat, ChatMessage, chat_context_percent, header_title};
    let mut chat = Chat::empty("chat-1".to_string());
    assert_eq!(chat_context_percent(&chat, "deepseek-chat"), 0.0);

    chat.messages.push(ChatMessage::user("q"));
    chat.messages.push(ChatMessage::assistant("a"));
    chat.messages.last_mut().unwrap().tokens = 300;
    chat.messages.last_mut().unwrap().context = 300;
    assert!(chat_context_percent(&chat, "deepseek-chat") < 1.0);

    // The last answer's request is what fills the window; earlier answers only
    // add to the running spend.
    chat.messages.last_mut().unwrap().tokens = 64_000;
    chat.messages.last_mut().unwrap().context = 64_000;
    assert_eq!(chat_context_percent(&chat, "deepseek-chat"), 50.0);

    // The billed total sums every tool round, so it can run past the window
    // while the request itself stays small: the capsule follows the request.
    chat.messages.last_mut().unwrap().tokens = 400_000;
    assert_eq!(chat_context_percent(&chat, "deepseek-chat"), 50.0);

    // Past 90% the capsule tints the figure, so the trim is not a surprise.
    chat.messages.last_mut().unwrap().context = 120_000;
    assert!(chat_context_percent(&chat, "deepseek-chat") >= 90.0);

    // A context larger than the window is clamped instead of showing 1200%.
    chat.messages.last_mut().unwrap().context = 2_000_000;
    assert_eq!(
        percent_label(chat_context_percent(&chat, "deepseek-chat")),
        "100%"
    );

    // Unknown models still get a denominator, so the figure is always usable.
    chat.messages.last_mut().unwrap().context = 64_000;
    assert_eq!(chat_context_percent(&chat, ""), 50.0);

    // Older saves carry no context figure at all, and an answer without one
    // must not borrow the spend number instead.
    chat.messages.last_mut().unwrap().context = 0;
    assert_eq!(chat_context_percent(&chat, "deepseek-chat"), 0.0);

    // The header row clips very long names so the figures stay beside it.
    assert_eq!(header_title("给出一段vb代码"), "给出一段vb代码");
    assert_eq!(header_title(&"字".repeat(18)), "字".repeat(18));
    assert_eq!(
        header_title(&"字".repeat(19)),
        format!("{}…", "字".repeat(18))
    );
}

#[test]
fn code_copy_ids_never_overflow() {
    use super::{DRAFT_MESSAGE, code_copy_id};
    // The draft slot is `usize::MAX`; this used to overflow and abort.
    let draft = code_copy_id(DRAFT_MESSAGE, 7);
    assert_ne!(draft, code_copy_id(DRAFT_MESSAGE, 7 + 1));
    assert_ne!(draft, code_copy_id(0, 7));
    assert_ne!(code_copy_id(0, 1), code_copy_id(1, 0));
    assert_ne!(code_copy_id(3, 2), code_copy_id(4, 2));
}

#[test]
fn an_undone_delete_lands_where_it_was() {
    use super::{Chat, reinsert};
    let mut chats = vec![
        Chat::empty("chat-1".to_string()),
        Chat::empty("chat-2".to_string()),
    ];
    chats[0].title = "第一".to_string();
    chats[1].title = "第二".to_string();
    let removed = Chat::restored("回来的".to_string(), "chat-9".to_string(), Vec::new());

    assert_eq!(reinsert(&mut chats, 1, removed), 1);
    assert_eq!(
        chats
            .iter()
            .map(|chat| chat.title.as_str())
            .collect::<Vec<_>>(),
        ["第一", "回来的", "第二"]
    );

    // An index past the end (the list shrank meanwhile) lands at the end.
    let extra = Chat::restored("末尾".to_string(), "chat-8".to_string(), Vec::new());
    assert_eq!(reinsert(&mut chats, 99, extra), 3);
    assert_eq!(chats.len(), 4);
}

#[test]
fn conversation_cards_say_when_they_were_used() {
    use super::{chat_meta, elapsed_label};
    assert_eq!(chat_meta(3, false, 0, false), "对话 · 3 条消息");
    assert_eq!(chat_meta(3, true, 0, false), "正在回答 · 3 条消息");
    // An unsent message is named instead of the neutral "对话"; an answer in
    // flight still wins, because that is the state the card is in.
    assert_eq!(chat_meta(3, false, 0, true), "草稿 · 3 条消息");
    assert_eq!(chat_meta(3, true, 0, true), "正在回答 · 3 条消息");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64);
    assert_eq!(
        chat_meta(3, false, stamp, false),
        format!("对话 · 3 条消息 · {}", elapsed_label(0))
    );
    assert_eq!(
        chat_meta(3, true, stamp, false),
        format!("正在回答 · 3 条消息 · {}", elapsed_label(0))
    );
}

#[test]
fn token_counts_stay_short() {
    use super::short_count;
    assert_eq!(short_count(0), "0");
    assert_eq!(short_count(999), "999");
    assert_eq!(short_count(1000), "1.0k");
    assert_eq!(short_count(12_340), "12k");
    assert_eq!(short_count(999_000), "999k");
}

#[test]
fn titles_come_from_the_first_question() {
    assert_eq!(title_from("统计文件"), "统计文件");
    assert_eq!(
        title_from("请帮我写一个统计文件数量并且生成报告的工作流程，越详细越好"),
        "请帮我写一个统计文件数量并且生成报告…"
    );
}

#[test]
fn conversation_folders_keep_counting_up() {
    assert_eq!(next_chat_dir(&[]), "chat-1");
    let chats = vec![
        Chat::empty("chat-1".to_string()),
        Chat::empty("chat-4".to_string()),
    ];
    assert_eq!(next_chat_dir(&chats), "chat-5");
    let trimmed = vec![Chat::empty("chat-2".to_string())];
    assert_eq!(next_chat_dir(&trimmed), "chat-3");
}

#[test]
fn stopping_keeps_the_text_that_was_already_written() {
    let kept = stopped_messages("  第一段\n\n第二段 ");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].role, Role::Assistant);
    assert_eq!(kept[0].text, "第一段\n\n第二段");

    // Nothing was written: stopping leaves no trace behind.
    assert!(stopped_messages(" \n ").is_empty());
}

#[test]
fn regenerate_finds_the_last_question() {
    let messages = vec![
        ChatMessage::user("第一个问题"),
        ChatMessage::assistant("回答一"),
        ChatMessage::user("第二个问题"),
        ChatMessage::tool("read · a.txt — 已读取 2 行", false),
        ChatMessage::assistant("回答"),
    ];
    assert_eq!(
        last_question(&messages),
        Some((2, "第二个问题".to_string()))
    );
}

#[test]
fn retry_sits_on_the_answer_unless_a_failure_ends_the_transcript() {
    assert_eq!(retry_message(&[]), None);
    assert_eq!(retry_message(&[ChatMessage::user("只有提问")]), None);
    assert_eq!(retry_message(&[ChatMessage::assistant("只有回答")]), None);
    assert_eq!(last_question(&[]), None);

    let answered = vec![ChatMessage::user("问题"), ChatMessage::assistant("回答")];
    assert_eq!(retry_message(&answered), Some(1));

    // A failure with no answer puts the control on the note itself.
    let failed = vec![ChatMessage::user("问题"), ChatMessage::note("出错了", true)];
    assert_eq!(retry_message(&failed), Some(1));
    assert_eq!(last_question(&failed), Some((0, "问题".to_string())));

    // Same after an earlier answer; the old answer must not carry it.
    let failed_later = vec![
        ChatMessage::user("问题一"),
        ChatMessage::assistant("回答一"),
        ChatMessage::user("问题"),
        ChatMessage::note("出错了", true),
    ];
    assert_eq!(retry_message(&failed_later), Some(3));

    // A note that is not a failure leaves the control on the answer before it.
    let noted = vec![
        ChatMessage::user("问题"),
        ChatMessage::assistant("半截回答"),
        ChatMessage::note("这只是一条说明，不是错误", false),
    ];
    assert_eq!(retry_message(&noted), Some(1));

    // A new question still waiting for its answer offers nothing yet.
    let awaiting = vec![
        ChatMessage::user("问题一"),
        ChatMessage::assistant("回答一"),
        ChatMessage::user("问题"),
    ];
    assert_eq!(retry_message(&awaiting), None);
}

#[test]
fn a_transcript_copies_as_markdown() {
    use super::transcript_text;
    let messages = vec![
        ChatMessage::user("  问题  "),
        ChatMessage::tool("read · a.txt — 已读取 2 行", false),
        ChatMessage::tool("grep · TODO — 3 处匹配", false),
        ChatMessage::assistant("回答"),
        ChatMessage::note("出错了", true),
    ];
    let text = transcript_text("标题", &messages);
    assert_eq!(
        text,
        "# 标题\n\n## 你\n\n问题\n\n\
         - 工具 · read · a.txt — 已读取 2 行\n- 工具 · grep · TODO — 3 处匹配\n\n\
         ## 助手\n\n回答\n\n> 出错了\n"
    );

    // Model, time and cost ride along with the heading only when they exist.
    let mut answered = ChatMessage::assistant("带用量的回答");
    answered.model = "deepseek-chat".to_string();
    answered.elapsed_ms = 2400;
    answered.tokens = 833;
    assert_eq!(
        transcript_text("", &[answered]),
        "## 助手（deepseek-chat · 2.4 s · 833 tokens）\n\n带用量的回答\n"
    );
}

#[test]
fn taking_the_last_question_drops_everything_after_it() {
    use super::take_last_question;
    let mut messages = vec![
        ChatMessage::user("问题一"),
        ChatMessage::assistant("回答一"),
        ChatMessage::tool("read · a.txt", false),
        ChatMessage::note("出错了", true),
        ChatMessage::user("问题"),
        ChatMessage::assistant("回答"),
    ];
    assert_eq!(take_last_question(&mut messages), Some("问题".to_string()));
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[3].role, Role::Note);

    assert_eq!(
        take_last_question(&mut messages),
        Some("问题一".to_string())
    );
    assert!(messages.is_empty());
    assert_eq!(take_last_question(&mut messages), None);
}

#[test]
fn a_saved_sidebar_width_is_clamped_to_the_handle() {
    use super::{SIDEBAR_DEFAULT, SIDEBAR_MAX, SIDEBAR_MIN, sidebar_width};
    assert_eq!(sidebar_width(None), SIDEBAR_DEFAULT);
    assert_eq!(sidebar_width(Some(300.)), 300.);
    assert_eq!(sidebar_width(Some(10.)), SIDEBAR_MIN);
    assert_eq!(sidebar_width(Some(9999.)), SIDEBAR_MAX);
}

#[test]
fn startup_opens_the_conversation_that_was_open_last_time() {
    use super::active_chat_index;
    let chats = vec![
        Chat::empty("chat-1".to_string()),
        Chat::empty("chat-2".to_string()),
        Chat::empty("chat-3".to_string()),
    ];
    assert_eq!(active_chat_index(&chats, "chat-2"), 1);
    // A folder that no longer exists (or a first run) falls back to the newest.
    assert_eq!(active_chat_index(&chats, "chat-9"), 2);
    assert_eq!(active_chat_index(&chats, ""), 2);
    assert_eq!(active_chat_index(&[], "chat-1"), 0);
}

/// One category, for the sidebar tests.
fn category(id: &str, name: &str) -> Category {
    Category {
        id: id.to_string(),
        name: name.to_string(),
    }
}

/// One conversation filed under a category, for the sidebar tests.
fn filed_chat(dir: &str, category: &str) -> Chat {
    let mut chat = Chat::empty(dir.to_string());
    chat.category = category.to_string();
    chat
}

#[test]
fn a_new_conversation_goes_into_the_open_category() {
    let categories = vec![category("c1", "对话"), category("c2", "笔记")];
    // The open one wins.
    assert_eq!(open_category_id(&categories, "c2"), "c2");
    // Every category folded: the default takes it.
    assert_eq!(open_category_id(&categories, ""), "c1");
    // A saved id that no longer exists still lands somewhere real.
    assert_eq!(open_category_id(&categories, "gone"), "c1");
}

#[test]
fn conversations_filed_under_a_missing_category_fall_back_to_the_default() {
    let categories = vec![category("c1", "对话"), category("c2", "笔记")];
    let chats = vec![
        filed_chat("chat-1", "c2"),
        filed_chat("chat-2", "c1"),
        // Older files have no category at all.
        filed_chat("chat-3", ""),
        filed_chat("chat-4", "gone"),
    ];
    let filed = file_unknown_categories(chats, &categories);

    assert_eq!(filed[0].category, "c2");
    assert_eq!(filed[1].category, "c1");
    assert_eq!(filed[2].category, "c1");
    assert_eq!(filed[3].category, "c1");
}

#[test]
fn a_workflow_falls_back_to_the_default_category() {
    let categories = vec![category("c1", "对话"), category("c2", "文件")];
    let filings = vec![("repo-health".to_string(), "c2".to_string())];

    // Filed ones keep their folder:
    // …everything else sits in the default one.
    assert_eq!(workflow_category(&filings, &categories, "triage"), "c1");
    // A filing that points at a folder that is gone is ignored.
    let stale = vec![("triage".to_string(), "gone".to_string())];
    assert_eq!(workflow_category(&stale, &categories, "triage"), "c1");
}

#[test]
fn deleting_a_category_hands_its_conversations_to_the_default() {
    let mut categories = vec![category("c1", "对话"), category("c2", "笔记")];
    let mut chats = vec![filed_chat("chat-1", "c2"), filed_chat("chat-2", "c1")];

    let default = remove_category(&mut categories, &mut chats, "c1").expect("removed");
    assert_eq!(default, "c2");
    assert_eq!(categories.len(), 1);
    // The conversation that lived in the deleted folder moved, the other stayed.
    assert_eq!(chats[1].category, "c2");
    assert_eq!(chats[0].category, "c2");

    // Deleting a category nobody is filed under still removes it.
    let mut categories = vec![category("c1", "对话"), category("c2", "笔记")];
    let mut chats = vec![filed_chat("chat-1", "c1")];
    assert_eq!(
        remove_category(&mut categories, &mut chats, "c2").as_deref(),
        Some("c1")
    );
    assert_eq!(chats[0].category, "c1");
}

#[test]
fn the_last_category_cannot_be_deleted() {
    let mut categories = vec![category("c1", "对话")];
    let mut chats = vec![filed_chat("chat-1", "c1")];
    assert!(remove_category(&mut categories, &mut chats, "c1").is_none());
    assert_eq!(categories.len(), 1);
    // An unknown id is a no-op too.
    let mut categories = vec![category("c1", "对话"), category("c2", "笔记")];
    assert!(remove_category(&mut categories, &mut chats, "gone").is_none());
    assert_eq!(categories.len(), 2);
}

#[test]
fn deleting_a_conversation_keeps_a_sensible_selection() {
    // Deleting above the open chat keeps the same conversation open.
    assert_eq!(selection_after_delete(3, 2, 0), 1);
    // Deleting below it does not move the selection.
    assert_eq!(selection_after_delete(3, 0, 2), 0);
    // Deleting the open chat shows what took its place.
    assert_eq!(selection_after_delete(3, 1, 1), 1);
    // Deleting the last chat falls back to the new last one.
    assert_eq!(selection_after_delete(2, 2, 2), 1);
    // The only chat is replaced by a fresh one.
    assert_eq!(selection_after_delete(0, 0, 0), 0);
}

#[test]
fn a_parsed_answer_is_reused_until_its_text_changes() {
    let mut cache = std::collections::HashMap::new();
    let first = super::chat::cached_parse(&mut cache, 3, "**粗*");
    let again = super::chat::cached_parse(&mut cache, 3, "**粗*");
    assert!(Rc::ptr_eq(&first, &again));
    // New text re-parses; another message is its own entry.
    let changed = super::chat::cached_parse(&mut cache, 3, "*斜");
    assert!(!Rc::ptr_eq(&first, &changed));
    let other = super::chat::cached_parse(&mut cache, 4, "**粗*");
    assert!(!Rc::ptr_eq(&first, &other));
}

#[test]
fn pinned_conversations_sit_on_top_of_their_category() {
    let mut chats = vec![
        Chat::empty("chat-1".to_string()),
        Chat::empty("chat-2".to_string()),
        Chat::empty("chat-3".to_string()),
    ];
    // With nothing pinned the newest card is first, as before.
    assert_eq!(chat_order(&chats), vec![2, 1, 0]);
    chats[0].pinned = true;
    assert_eq!(chat_order(&chats), vec![0, 2, 1]);
    // Two pinned cards keep the newest-first rule among themselves.
    chats[2].pinned = true;
    assert_eq!(chat_order(&chats), vec![2, 0, 1]);
}

#[test]
fn saved_conversations_come_back() {
    let saved = vec![
        crate::chats::StoredChat {
            title: "统计文件".to_string(),
            dir: "chat-2".to_string(),
            category: "c1".to_string(),
            updated_ms: 0,
            details: vec![(1, 0)],
            checks: vec![(0, 2)],
            draft: String::new(),
            draft_files: Vec::new(),
            pinned: false,
            tools: true,
            read_only: false,
            full_access: true,
            auto_actions: false,
            auto_review: false,
            max_rounds: crate::config::MAX_ROUNDS_CHOICE,
            max_calls: crate::config::MAX_CALLS_CHOICE,
            mode: crate::config::MODE_FULL.to_string(),
            web_search: None,
            plan: false,
            todos: Vec::new(),
            messages: vec![
                crate::chats::StoredMessage {
                    role: "user".to_string(),
                    text: "有多少文件？".to_string(),
                    failed: false,
                    model: String::new(),
                    elapsed_ms: 0,
                    model_ms: 0,
                    tool_ms: 0,
                    tokens: 0,
                    context: 0,
                    cache_hits: 0,
                    rounds: 0,
                    round_tokens: Vec::new(),
                    round_reasoning: Vec::new(),
                    composition: Vec::new(),
                    tools_tokens: 0,
                    compaction_layer: String::new(),
                    compaction_saved: 0,
                    interrupted: false,
                    reasoning: String::new(),
                    server_tool_calls: Vec::new(),
                    round: 0,
                    kind: String::new(),
                    input: String::new(),
                    output: String::new(),
                    open: false,
                    call: String::new(),
                    path: String::new(),
                    before: String::new(),
                    after: String::new(),
                    existed: false,
                    undoable: false,
                    image_mime: String::new(),
                    image_path: String::new(),
                    files: Vec::new(),
                },
                crate::chats::StoredMessage {
                    role: "assistant".to_string(),
                    text: "12 个。".to_string(),
                    failed: false,
                    model: "deepseek-chat".to_string(),
                    elapsed_ms: 1300,
                    model_ms: 0,
                    tool_ms: 0,
                    tokens: 0,
                    context: 0,
                    cache_hits: 0,
                    rounds: 0,
                    round_tokens: Vec::new(),
                    round_reasoning: Vec::new(),
                    composition: Vec::new(),
                    tools_tokens: 0,
                    compaction_layer: String::new(),
                    compaction_saved: 0,
                    interrupted: false,
                    reasoning: String::new(),
                    server_tool_calls: Vec::new(),
                    round: 0,
                    kind: String::new(),
                    input: String::new(),
                    output: String::new(),
                    open: false,
                    call: String::new(),
                    path: String::new(),
                    before: String::new(),
                    after: String::new(),
                    existed: false,
                    undoable: false,
                    image_mime: String::new(),
                    image_path: String::new(),
                    files: Vec::new(),
                },
                crate::chats::StoredMessage {
                    role: "odd".to_string(),
                    text: "旧版本写的角色".to_string(),
                    failed: true,
                    model: String::new(),
                    elapsed_ms: 0,
                    model_ms: 0,
                    tool_ms: 0,
                    tokens: 0,
                    context: 0,
                    cache_hits: 0,
                    rounds: 0,
                    round_tokens: Vec::new(),
                    round_reasoning: Vec::new(),
                    composition: Vec::new(),
                    tools_tokens: 0,
                    compaction_layer: String::new(),
                    compaction_saved: 0,
                    interrupted: false,
                    reasoning: String::new(),
                    server_tool_calls: Vec::new(),
                    round: 0,
                    kind: String::new(),
                    input: String::new(),
                    output: String::new(),
                    open: false,
                    call: String::new(),
                    path: String::new(),
                    before: String::new(),
                    after: String::new(),
                    existed: false,
                    undoable: false,
                    image_mime: String::new(),
                    image_path: String::new(),
                    files: Vec::new(),
                },
            ],
        },
        crate::chats::StoredChat {
            title: "第二个".to_string(),
            dir: "chat-3".to_string(),
            category: String::new(),
            updated_ms: 0,
            details: Vec::new(),
            checks: Vec::new(),
            draft: String::new(),
            draft_files: Vec::new(),
            pinned: false,
            tools: true,
            read_only: false,
            full_access: true,
            auto_actions: false,
            auto_review: false,
            max_rounds: crate::config::MAX_ROUNDS_CHOICE,
            max_calls: crate::config::MAX_CALLS_CHOICE,
            mode: crate::config::MODE_FULL.to_string(),
            web_search: None,
            plan: false,
            todos: Vec::new(),
            messages: Vec::new(),
        },
    ];
    let chats = load_saved(saved, ChatDefaults::default());
    assert_eq!(chats.len(), 2);
    assert_eq!(chats[0].title, "统计文件");
    assert_eq!(chats[0].dir, "chat-2");
    assert_eq!(chats[0].messages.len(), 3);
    assert_eq!(chats[0].messages[0].role, Role::User);
    assert_eq!(chats[0].messages[1].role, Role::Assistant);
    // An unknown role is kept as a note rather than dropped.
    assert_eq!(chats[0].messages[2].role, Role::Note);
    assert!(chats[0].messages[2].failed);
    // The `<details>` sections someone opened come back with the chat.
    assert!(chats[0].details.contains(&(1, 0)));
    // So do the task boxes they ticked.
    assert!(chats[0].checks.contains(&(0, 2)));
}

#[test]
fn unsent_text_and_files_come_back_with_the_conversation() {
    let mut saved = crate::chats::StoredChat {
        title: "草稿".to_string(),
        dir: "chat-4".to_string(),
        category: String::new(),
        updated_ms: 0,
        details: Vec::new(),
        checks: Vec::new(),
        draft: "还没发的问题".to_string(),
        draft_files: vec!["C:\\tmp\\note.txt".to_string()],
        pinned: false,
        tools: true,
        read_only: false,
        full_access: true,
        auto_actions: false,
        auto_review: false,
        max_rounds: crate::config::MAX_ROUNDS_CHOICE,
        max_calls: crate::config::MAX_CALLS_CHOICE,
        mode: crate::config::MODE_FULL.to_string(),
        web_search: None,
        plan: false,
        todos: Vec::new(),
        messages: Vec::new(),
    };
    let (drafts, files) = drafts_of(std::slice::from_ref(&saved));
    assert_eq!(
        drafts,
        vec![("chat-4".to_string(), "还没发的问题".to_string())]
    );
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "chat-4");
    assert_eq!(files[0].1, vec!["C:\\tmp\\note.txt".to_string()]);
    // Whitespace on its own is not a draft: nothing to hand back.
    saved.draft = "   \n".to_string();
    saved.draft_files.clear();
    let (drafts, files) = drafts_of(std::slice::from_ref(&saved));
    assert!(drafts.is_empty());
    assert!(files.is_empty());
}

#[test]
fn an_empty_store_still_opens_one_conversation() {
    let chats = load_saved(Vec::new(), ChatDefaults::default());
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].dir, "chat-1");
    assert!(chats[0].messages.is_empty());
    // The next new conversation continues the numbering, not restarts it.
    assert_eq!(next_chat_dir(&chats), "chat-2");
}

#[test]
fn new_conversations_start_on_the_configured_defaults() {
    let defaults = ChatDefaults {
        mode: ToolMode::ReadOnly,
        max_rounds: 6,
        max_calls: 20,
    };
    let chat = Chat::empty_with("chat-1".to_string(), defaults);
    assert_eq!(ToolMode::of(&chat), ToolMode::ReadOnly);
    assert_eq!(chat.max_rounds, 6);
    assert_eq!(chat.max_calls, 20);

    // The built-in level is the fixed default: workspace / 6 / 20.
    let builtin = Chat::empty("chat-1".to_string());
    assert_eq!(ToolMode::of(&builtin), ToolMode::Workspace);
    assert_eq!(builtin.max_rounds, crate::config::DEFAULT_MAX_ROUNDS);
    assert_eq!(builtin.max_calls, crate::config::DEFAULT_MAX_CALLS);
}

#[test]
fn saved_conversations_keep_their_own_settings() {
    let defaults = ChatDefaults {
        mode: ToolMode::Full,
        max_rounds: 24,
        max_calls: 80,
    };
    let saved = crate::chats::StoredChat {
        title: "旧对话".to_string(),
        dir: "chat-2".to_string(),
        category: String::new(),
        updated_ms: 0,
        details: Vec::new(),
        checks: Vec::new(),
        draft: String::new(),
        draft_files: Vec::new(),
        pinned: false,
        tools: true,
        read_only: true,
        full_access: false,
        auto_actions: false,
        auto_review: false,
        max_rounds: 12,
        max_calls: 40,
        mode: crate::config::MODE_FULL.to_string(),
        web_search: None,
        plan: false,
        todos: Vec::new(),
        messages: Vec::new(),
    };
    let chats = load_saved(vec![saved], defaults);
    assert_eq!(
        ToolMode::of(&chats[0]),
        ToolMode::ReadOnly,
        "the stored level wins over the new default"
    );
    assert_eq!(chats[0].max_rounds, 12);
    assert_eq!(chats[0].max_calls, 40);

    // A first run has no saved values, so it takes the defaults.
    let blank = load_saved(Vec::new(), defaults);
    assert_eq!(ToolMode::of(&blank[0]), ToolMode::Full);
    assert_eq!(blank[0].max_rounds, 24);
    assert_eq!(blank[0].max_calls, 80);
}

#[test]
fn role_names_round_trip() {
    for role in [
        Role::User,
        Role::Assistant,
        Role::Tool,
        Role::Thinking,
        Role::Approval,
        Role::Note,
    ] {
        assert_eq!(role_from(role_name(role)), role);
    }
    assert_eq!(role_from("assistant"), Role::Assistant);
}

#[test]
fn a_blank_rename_keeps_the_old_title() {
    // The rule the UI follows: a title is only replaced by real text.
    assert_eq!(rename_to("新名字"), Some("新名字".to_string()));
    assert_eq!(rename_to("  新名字 "), Some("新名字".to_string()));
    assert_eq!(rename_to("   "), None);
    assert_eq!(rename_to(""), None);
}

#[test]
fn run_ages_read_naturally() {
    assert_eq!(elapsed_label(0), "刚刚");
    assert_eq!(elapsed_label(4), "刚刚");
    assert_eq!(elapsed_label(9), "9 秒前");
    assert_eq!(elapsed_label(120), "2 分钟前");
    assert_eq!(elapsed_label(3 * 3600), "3 小时前");
    assert_eq!(elapsed_label(2 * 86_400), "2 天前");
}

#[test]
fn a_saved_run_colours_the_card() {
    let palette = crate::theme::ThemeKind::Light.palette();
    let ok = RunBadge {
        ok: true,
        cancelled: false,
        finished_ms: 0,
    };
    let failed = RunBadge {
        ok: false,
        cancelled: false,
        finished_ms: 0,
    };
    let stopped = RunBadge {
        ok: false,
        cancelled: true,
        finished_ms: 0,
    };
    assert_eq!(ok.color(palette), palette.ok);
    assert_eq!(failed.color(palette), palette.failed);
    assert_eq!(stopped.color(palette), palette.warn);
}

#[test]
fn pasted_text_keeps_lines_only_in_the_composer() {
    assert_eq!(clean_paste("a\r\nb\rc", true), "a\nb\nc");
    assert_eq!(clean_paste("a\r\nb\rc", false), "a b c");
    assert_eq!(clean_paste("tab\there\u{7}", true), "tab\there");
    assert_eq!(clean_paste("中文\n第二行", true), "中文\n第二行");
}

#[test]
fn search_finds_conversations_by_title_and_text() {
    let mut chat = Chat::restored(
        "Repo health".to_string(),
        "chat-1".to_string(),
        vec![
            ChatMessage::user("这个流程做了什么？"),
            ChatMessage::assistant("它检查 TODO 注释和空文件。"),
        ],
    );
    assert!(chat_matches(&chat, "repo"));
    assert!(chat_matches(&chat, "todo"));
    assert!(chat_matches(&chat, "检查"));
    assert!(!chat_matches(&chat, "deepseek"));

    // Titles are matched case-insensitively, and message text too.
    chat.title = "统计文件".to_string();
    assert!(chat_matches(&chat, "统计"));
    assert!(!chat_matches(&chat, "repo"));
}

#[test]
fn history_keeps_questions_and_answers_only() {
    let messages = vec![
        ChatMessage::user("第一个问题"),
        ChatMessage::tool("read · a.txt — 已读取 2 行", false),
        ChatMessage::assistant("第一段回答"),
        ChatMessage::note("已取消这次请求。", false),
        ChatMessage::user("   "),
        ChatMessage::user("第二个问题"),
    ];
    let turns = history_turns(&messages);
    assert_eq!(
        turns,
        vec![
            Turn {
                speaker: Speaker::User,
                text: "第一个问题".to_string(),
                server_tool_calls: Vec::new(),
            },
            Turn {
                speaker: Speaker::Assistant,
                text: "第一段回答".to_string(),
                server_tool_calls: Vec::new(),
            },
            Turn {
                speaker: Speaker::User,
                text: "第二个问题".to_string(),
                server_tool_calls: Vec::new(),
            },
        ]
    );
}

#[test]
fn undo_paths_follow_the_file_scope() {
    let root = std::env::temp_dir().join(format!("dsh-flow-ui-undo-{}", std::process::id()));
    let work = root.join("chat-1");
    let outside = std::env::temp_dir().join(format!("dsh-flow-ui-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let workspace = Some(root.as_path());

    let resolved = resolve_scoped(&work, workspace, "a.txt").expect("inside path");
    assert_eq!(
        resolved.file_name().and_then(|name| name.to_str()),
        Some("a.txt")
    );
    assert!(resolved.starts_with(root.canonicalize().unwrap()));

    // Default scope: `..` is fine while the result stays under the workspace.
    assert!(resolve_scoped(&work, workspace, "../sub/a.txt").is_some());
    assert!(resolve_scoped(&work, workspace, "../../escape.txt").is_none());
    let absolute_inside = root.join("sub").join("b.txt");
    assert!(resolve_scoped(&work, workspace, absolute_inside.to_str().unwrap()).is_some());
    assert!(resolve_scoped(&work, workspace, "").is_none());

    // Full access: paths outside the workspace are allowed.
    let absolute_outside = outside.join("x.txt");
    assert!(resolve_scoped(&work, None, absolute_outside.to_str().unwrap()).is_some());
    assert!(resolve_scoped(&work, None, "../anything.txt").is_some());

    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn a_find_query_matches_messages_case_insensitively() {
    let messages = vec![
        ChatMessage::user("统计文件数量"),
        ChatMessage::assistant("找到 TODO 三处"),
        ChatMessage::tool("grep · TODO — 3 处匹配", false),
        ChatMessage::note("出错了", true),
    ];
    assert_eq!(find_messages(&messages, "todo"), vec![1, 2]);
    assert_eq!(find_messages(&messages, " 统计 "), vec![0]);
    assert_eq!(find_messages(&messages, "出错了"), vec![3]);
    assert!(find_messages(&messages, "   ").is_empty());
    assert!(find_messages(&messages, "没有").is_empty());
}

#[test]
fn undo_restores_only_an_unchanged_write() {
    let root = std::env::temp_dir().join(format!("dsh-flow-ui-undo-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("a.txt");

    std::fs::write(&file, "new").unwrap();
    assert!(restore_file(&file, "new", "old", true).is_ok());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "old");

    std::fs::write(&file, "later").unwrap();
    let error = restore_file(&file, "new", "old", true).unwrap_err();
    assert!(error.contains("已被修改"), "{error}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "later");

    let created = root.join("created.txt");
    std::fs::write(&created, "").unwrap();
    assert!(restore_file(&created, "", "", false).is_ok());
    assert!(!created.exists());

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn history_drops_oldest_turns_past_the_token_budget() {
    // Each turn fits on its own and stays under the per-turn clip; three of
    // them together exceed the window, so the oldest one goes.
    let big = "长".repeat(MAX_HISTORY_TURN_TOKENS - 1_000);
    let messages = vec![
        ChatMessage::user(big.clone()),
        ChatMessage::user(big.clone()),
        ChatMessage::user(big.clone()),
    ];
    let turns = history_turns(&messages);
    assert_eq!(turns.len(), 2, "the oldest big turn is dropped");
    assert_eq!(turns[0].text, big);
    assert_eq!(turns[1].text, big);
}

#[test]
fn history_drops_the_oldest_turns() {
    let messages: Vec<ChatMessage> = (0..MAX_TURNS + 5)
        .map(|index| ChatMessage::user(format!("问题 {index}")))
        .collect();
    let turns = history_turns(&messages);
    assert_eq!(turns.len(), MAX_TURNS);
    assert_eq!(turns[0].text, "问题 5");
    assert_eq!(
        turns[turns.len() - 1].text,
        format!("问题 {}", MAX_TURNS + 4)
    );
    assert!(history_turns(&[]).is_empty());
}

#[test]
fn history_reports_how_many_turns_it_dropped() {
    let big = "长".repeat(MAX_HISTORY_TOKENS / 2 + 1);
    let messages = vec![
        ChatMessage::user(big.clone()),
        ChatMessage::assistant("回答一"),
        ChatMessage::user(big),
    ];
    let (turns, dropped) = history_with_dropped(&messages);
    assert_eq!(turns.len(), 2);
    assert_eq!(dropped, 1);

    let many: Vec<ChatMessage> = (0..MAX_TURNS + 5)
        .map(|index| ChatMessage::user(format!("问题 {index}")))
        .collect();
    let (turns, dropped) = history_with_dropped(&many);
    assert_eq!(turns.len(), MAX_TURNS);
    assert_eq!(dropped, 5);

    let (turns, dropped) = history_with_dropped(&[]);
    assert!(turns.is_empty());
    assert_eq!(dropped, 0);
}

#[test]
fn merges_plan_and_results() {
    let spec = crate::manifest::parse(
        "name: demo\nsteps:\n  - id: a\n    run: python a.py\n  - id: b\n    set: { x: 1 }\n",
    );
    let results = vec![StepView {
        id: "b".to_string(),
        kind: "set".to_string(),
        status: "ok".to_string(),
        duration_ms: 3,
        detail: String::new(),
        command: None,
        stderr: None,
        stdout: None,
        text: None,
        model: None,
        cached: false,
        summary: None,
        artifacts: Vec::new(),
        error: None,
    }];
    let rows = build_rows(&spec, &results);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "a");
    assert_eq!(rows[0].status, "pending");
    assert_eq!(rows[1].id, "b");
    assert_eq!(rows[1].status, "ok");
    assert_eq!(rows[1].duration_ms, 3);
}

#[test]
fn keeps_results_that_are_missing_from_the_plan() {
    let spec = crate::manifest::parse("name: demo\nsteps:\n  - id: a\n    run: python a.py\n");
    let results = vec![StepView {
        id: "ghost".to_string(),
        kind: "run".to_string(),
        status: "failed".to_string(),
        duration_ms: 9,
        detail: "boom".to_string(),
        command: None,
        stderr: None,
        stdout: None,
        text: None,
        model: None,
        cached: false,
        summary: None,
        artifacts: Vec::new(),
        error: None,
    }];
    let rows = build_rows(&spec, &results);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].id, "ghost");
    assert_eq!(rows[1].status, "failed");
}

#[test]
fn a_finished_approval_says_what_happened() {
    assert_eq!(approval_state("shell", true), "已执行");
    assert_eq!(approval_state("shell", false), "执行失败");
    assert_eq!(approval_state("move", true), "已移动");
    assert_eq!(approval_state("move", false), "移动失败");
    assert_eq!(approval_state("delete", true), "已删除");
    assert_eq!(approval_state("delete", false), "删除失败");
    assert_eq!(approval_state("write", true), "已写入");
    assert_eq!(approval_state("edit", false), "写入失败");
    assert_eq!(approval_label("shell"), "命令确认");
    assert_eq!(approval_label("python"), "代码确认");
    assert_eq!(approval_label("node"), "代码确认");
    assert_eq!(approval_label("move"), "移动确认");
    assert_eq!(approval_label("delete"), "删除确认");
    assert_eq!(approval_label("write"), "写入确认");
    assert_eq!(approval_label("edit"), "写入确认");
    assert_eq!(approval_hint("delete"), "允许后才会删除；删除后不能撤销");
    assert_eq!(approval_state("node", true), "已执行");
    assert_eq!(approval_state("python", false), "执行失败");
    // A plugin tool and an MCP tool run code or an RPC call; neither writes a
    // file, so neither may borrow the write wording.
    assert_eq!(approval_label("plugin_echo"), "插件确认");
    assert_eq!(approval_label("mcp"), "MCP 确认");
    assert_eq!(approval_label("mcp_files_read"), "MCP 确认");
    assert_eq!(approval_state("plugin_echo", true), "已执行");
    assert_eq!(approval_state("plugin_echo", false), "执行失败");
    assert_eq!(approval_state("mcp_files_read", true), "已执行");
    assert!(approval_hint("plugin_echo").contains("插件代码"));
    assert!(approval_hint("mcp_files_read").contains("MCP"));
    // Only an exact command or snippet may be remembered for the session.
    assert!(!allow_session_button("plugin_echo", "{\"text\":\"hi\"}"));
}

#[test]
fn a_plugin_card_counts_what_the_plugin_contributed() {
    let tools = vec![
        ("demo".to_string(), "demo_echo".to_string()),
        ("demo".to_string(), "demo_clock".to_string()),
        ("other".to_string(), "other_thing".to_string()),
    ];
    let prompts = vec![
        ("demo".to_string(), "先看 README".to_string()),
        ("other".to_string(), "再跑测试".to_string()),
        ("other".to_string(), "最后写总结".to_string()),
    ];
    assert_eq!(
        plugin_contribution("demo", &tools, &prompts),
        " · 2 个工具 · 1 段提示"
    );
    assert_eq!(
        plugin_contribution("other", &tools, &prompts),
        " · 1 个工具 · 2 段提示"
    );
    // A plugin that contributed nothing (or is switched off) adds no suffix.
    assert_eq!(plugin_contribution("idle", &tools, &prompts), "");
    assert_eq!(plugin_contribution("demo", &[], &[]), "");
    // Tools count first, then prompts.
    assert_eq!(
        plugin_contribution("demo", &tools[..1], &prompts[..0]),
        " · 1 个工具"
    );
    assert_eq!(
        plugin_contribution("demo", &[], &prompts[..1]),
        " · 1 段提示"
    );
}

#[test]
fn basic_mode_hides_the_mcp_gateway() {
    let basic = crate::config::ModeSelection::default();
    let tools = mode_tools(&basic);
    assert!(!tools.contains(&"mcp"));
    assert!(tools.contains(&"read"), "built-in tools stay available");

    let with_mcp = crate::config::ModeSelection {
        mcp_servers: vec!["server-a".to_string()],
        ..crate::config::ModeSelection::default()
    };
    assert!(mode_tools(&with_mcp).contains(&"mcp"));
}

#[test]
fn the_tool_list_offers_code_runners() {
    assert!(ALL_TOOLS.contains(&"python"), "python 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"node"), "node 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"symbols"), "symbols 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"task"), "task 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"replace"), "replace 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"http"), "http 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"copy"), "copy 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"mkdir"), "mkdir 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"env"), "env 要出现在允许列表里");
    assert!(
        ALL_TOOLS.contains(&"read_image"),
        "read_image 要出现在允许列表里"
    );
    assert!(
        ALL_TOOLS.contains(&"screenshot"),
        "screenshot 要出现在允许列表里"
    );
    assert!(ALL_TOOLS.contains(&"browser"), "browser 要出现在允许列表里");
    assert!(ALL_TOOLS.contains(&"show"), "show 要出现在允许列表里");
    assert!(
        ALL_TOOLS.contains(&"git_write"),
        "git_write 要出现在允许列表里"
    );
    assert!(
        ALL_TOOLS.contains(&"wait_for"),
        "wait_for 要出现在允许列表里"
    );
    assert!(
        ALL_TOOLS.contains(&"run_tests"),
        "run_tests 要出现在允许列表里"
    );
    assert!(
        ALL_TOOLS.contains(&"diagnostics"),
        "diagnostics 要出现在允许列表里"
    );
    assert!(ALL_TOOLS.contains(&"mcp"), "mcp 要出现在允许列表里");
    assert_eq!(ALL_TOOLS.len(), 29);
}

#[test]
fn copy_and_mkdir_read_as_their_own_actions() {
    assert_eq!(approval_label("copy"), "复制确认");
    assert_eq!(approval_state("copy", true), "已复制");
    assert_eq!(approval_state("copy", false), "复制失败");
    assert!(approval_hint("copy").contains("覆盖"));

    assert_eq!(approval_label("mkdir"), "建目录确认");
    assert_eq!(approval_state("mkdir", true), "已创建");
    assert_eq!(approval_state("mkdir", false), "创建失败");
    assert!(approval_hint("mkdir").contains("目录"));
}

#[test]
fn a_task_approval_reads_as_an_operation_on_a_background_command() {
    assert_eq!(approval_label("task"), "任务确认");
    // The same card covers `input`, `close` and `stop`, so the state cannot say
    // "ended" for a call that only wrote a line into its stdin.
    assert_eq!(approval_state("task", true), "已完成");
    assert_eq!(approval_state("task", false), "操作失败");
    assert!(approval_hint("task").contains("后台任务"));
    assert!(approval_hint("task").contains("stdin"));
}

#[test]
fn a_new_chat_starts_on_the_default_preset() {
    let chat = Chat::empty("chat-1".to_string());
    assert!(chat.tools);
    assert!(!chat.read_only);
    assert!(!chat.full_access);
    assert!(!chat.auto_actions);
    assert_eq!(ToolMode::of(&chat), ToolMode::Workspace);
    assert_eq!(chat.max_rounds, crate::config::DEFAULT_MAX_ROUNDS);
    assert_eq!(chat.max_calls, crate::config::DEFAULT_MAX_CALLS);
}

#[test]
fn every_tool_mode_round_trips_into_the_chat_flags() {
    let mut chat = Chat::empty("chat-1".to_string());
    for mode in ToolMode::ALL {
        mode.apply(&mut chat);
        assert_eq!(ToolMode::of(&chat), mode, "{mode:?}");
    }
    ToolMode::Full.apply(&mut chat);
    assert!(chat.tools && chat.full_access && chat.auto_actions && !chat.read_only);
    ToolMode::ReadOnly.apply(&mut chat);
    assert!(chat.tools && chat.read_only && !chat.full_access && !chat.auto_actions);
    // A conversation saved with tools switched off reads as 仅可查看, and
    // picking any level turns them back on.
    chat.tools = false;
    assert_eq!(ToolMode::of(&chat), ToolMode::ReadOnly);
    ToolMode::Workspace.apply(&mut chat);
    assert!(chat.tools && !chat.read_only && !chat.full_access && !chat.auto_actions);
}

#[test]
fn tool_modes_keep_dsh_names_labels_and_glyphs() {
    assert_eq!(ToolMode::ALL.len(), 3);
    for (mode, label) in [
        (ToolMode::ReadOnly, "仅可查看"),
        (ToolMode::Workspace, "工作区内修改"),
        (ToolMode::Full, "完全权限"),
    ] {
        assert_eq!(mode.label(), label);
        assert!(mode.icon().starts_with("icons/permission-"));
    }
}

#[test]
fn project_rules_prefer_the_chat_folder_and_fall_back() {
    let root = std::env::temp_dir().join(format!("dsh-rules-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let workspace = root.join("ws");
    let chat = workspace.join("chat-1");
    std::fs::create_dir_all(&chat).unwrap();

    std::fs::write(workspace.join("AGENTS.md"), "workspace rules\n").unwrap();
    let rules = project_rules(&workspace, &chat);
    assert!(rules.contains("workspace rules"), "{rules}");
    assert!(rules.contains("AGENTS.md"), "{rules}");

    std::fs::write(chat.join("AGENTS.md"), "chat rules\n").unwrap();
    let rules = project_rules(&workspace, &chat);
    assert!(rules.contains("chat rules"), "{rules}");
    assert!(!rules.contains("workspace rules"), "{rules}");

    std::fs::remove_file(chat.join("AGENTS.md")).unwrap();
    std::fs::remove_file(workspace.join("AGENTS.md")).unwrap();
    std::fs::create_dir_all(workspace.join(".github")).unwrap();
    std::fs::write(
        workspace.join(".github/copilot-instructions.md"),
        "copilot rules\n",
    )
    .unwrap();
    let rules = project_rules(&workspace, &chat);
    assert!(rules.contains("copilot rules"), "{rules}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn project_rules_are_capped_and_skip_empty_files() {
    let root = std::env::temp_dir().join(format!("dsh-rules-cap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let workspace = root.join("ws");
    let chat = workspace.join("chat-1");
    std::fs::create_dir_all(&chat).unwrap();

    std::fs::write(workspace.join("AGENTS.md"), "  \n").unwrap();
    assert!(
        project_rules(&workspace, &chat).is_empty(),
        "blank file is skipped"
    );

    std::fs::write(
        workspace.join("AGENTS.md"),
        "长".repeat(PROJECT_RULES_MAX + 20),
    )
    .unwrap();
    let rules = project_rules(&workspace, &chat);
    assert!(rules.contains("已截断"), "{}", rules.chars().count());
    assert!(
        rules.chars().count() <= PROJECT_RULES_MAX + 100,
        "{}",
        rules.chars().count()
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_tool_recap_carries_only_the_previous_turn() {
    let mut old = ChatMessage::tool("read · old.txt — 已读取 1 行", false);
    old.output = "1\told".to_string();
    let mut fresh = ChatMessage::tool("read · new.txt — 已读取 2 行", false);
    fresh.output = "1\talpha\n2\tbeta".to_string();
    let messages = vec![
        ChatMessage::user("第一问"),
        old,
        ChatMessage::assistant("第一答"),
        ChatMessage::user("第二问"),
        fresh,
        ChatMessage::assistant("第二答"),
    ];
    let recap = tool_recap(&messages);
    assert!(recap.contains("new.txt"), "{recap}");
    assert!(recap.contains("alpha"), "{recap}");
    assert!(!recap.contains("old.txt"), "{recap}");
    assert!(recap.contains("可能已经改动"), "{recap}");
}

#[test]
fn a_tool_recap_is_empty_without_tools() {
    let messages = vec![ChatMessage::user("问题"), ChatMessage::assistant("回答")];
    assert!(tool_recap(&messages).is_empty());
}

#[test]
fn a_tool_recap_keeps_the_last_rows_and_caps_its_size() {
    let mut messages = vec![ChatMessage::user("问题")];
    for index in 0..12 {
        let mut tool = ChatMessage::tool(format!("read · f{index}.txt"), false);
        tool.output = "x".repeat(3_000);
        messages.push(tool);
    }
    messages.push(ChatMessage::assistant("回答"));
    let recap = tool_recap(&messages);
    let kept: Vec<usize> = (0..12)
        .filter(|index| recap.contains(&format!("f{index}.txt")))
        .collect();
    assert!(kept.len() < 12, "the budget drops whole rows");
    assert_eq!(kept.last(), Some(&11), "the newest row survives the budget");
    assert_eq!(
        kept,
        (kept[0]..12).collect::<Vec<_>>(),
        "only a contiguous run of the newest rows survives"
    );
    assert!(kept[0] > 0, "the oldest rows fall off");
    assert!(
        recap.chars().count() <= RECAP_CHARS + 200,
        "recap is {} characters",
        recap.chars().count()
    );
}

#[test]
fn an_approval_opens_its_diff_and_folds_when_settled() {
    let mut chat = Chat::empty("chat-1".to_string());
    chat.messages.push(ChatMessage::approval(
        "c1".to_string(),
        "a.txt".to_string(),
        "命令确认".to_string(),
        "$ node -e …".to_string(),
        String::new(),
        String::new(),
        false,
        false,
    ));
    assert!(chat.messages[0].open, "the diff is visible while deciding");
    settle_interactions(&mut chat, "已取消");
    assert_eq!(chat.messages[0].output, "已取消");
    assert!(!chat.messages[0].open, "a settled card folds");
}

#[test]
fn turn_writes_lists_only_undoable_writes_from_that_answer() {
    let write = |call: &str, path: &str, state: &str, undoable: bool| {
        let mut message = ChatMessage::approval(
            call.to_string(),
            path.to_string(),
            "写入确认".to_string(),
            "+ hi".to_string(),
            String::new(),
            "hi".to_string(),
            false,
            undoable,
        );
        message.tool = "write".to_string();
        message.output = state.to_string();
        message
    };
    let messages = vec![
        ChatMessage::user("第一问"),
        write("c1", "old.txt", "已写入", true),
        ChatMessage::assistant("第一答"),
        ChatMessage::user("第二问"),
        write("c2", "kept.txt", "已写入", true),
        write("c3", "failed.txt", "写入失败", true),
        write("c4", "big.txt", "已写入", false),
        ChatMessage::assistant("第二答"),
    ];
    assert_eq!(turn_writes(&messages, 2), vec![1]);
    assert_eq!(turn_writes(&messages, 7), vec![4]);
    assert!(
        turn_writes(&messages, 4).is_empty(),
        "an answer that has not finished owns nothing yet"
    );
}

#[test]
fn a_write_run_without_asking_is_still_undoable_from_its_tool_row() {
    // 完全权限不画确认卡：撤销的那一对写在工具行上。
    let mut row = ChatMessage::tool("write · a.txt — 已写入 a.txt（2 字符）", false);
    row.call = "call-1".to_string();
    row.path = "a.txt".to_string();
    row.before = "old".to_string();
    row.after = "new".to_string();
    row.existed = true;
    row.undoable = true;
    // A plain tool row with no write behind it is not undoable.
    let mut plain = ChatMessage::tool("shell · echo hi — 退出码 0", false);
    plain.call = "call-2".to_string();
    let messages = vec![
        ChatMessage::user("改一下 a.txt"),
        row,
        plain,
        ChatMessage::assistant("已改好"),
    ];
    assert_eq!(turn_writes(&messages, 3), vec![1]);
    // After undoing, the pair is cleared so a second undo finds nothing.
    let mut undone = messages.clone();
    undone[1].undoable = false;
    undone[1].before.clear();
    undone[1].after.clear();
    undone[1].output = "已撤销".to_string();
    assert!(turn_writes(&undone, 3).is_empty());
}

#[test]
fn live_output_keeps_the_newest_line_and_a_bounded_tail() {
    assert_eq!(
        last_output_line("alpha\nbeta\n\n"),
        Some("beta".to_string())
    );
    assert_eq!(last_output_line("  \n"), None);
    assert_eq!(
        last_output_line(&"x".repeat(200)).map(|line| line.chars().count()),
        Some(120)
    );

    let mut output = String::new();
    append_live_output(&mut output, &"a".repeat(LIVE_OUTPUT_MAX + 50));
    assert_eq!(output.chars().count(), LIVE_OUTPUT_MAX);
    assert!(output.chars().all(|char| char == 'a'));

    assert_eq!(
        live_row_text("shell", "npm test", Some("ok")),
        "shell · npm test — ok"
    );
    assert_eq!(live_row_text("shell", "", Some("ok")), "shell — ok");
    assert_eq!(live_row_text("shell", "npm test", None), "shell · npm test");
}

#[test]
fn a_turn_summarises_files_and_diff_lines() {
    let write = |call: &str, path: &str, diff: &str, state: &str| {
        let mut message = ChatMessage::approval(
            call.to_string(),
            path.to_string(),
            "写入确认".to_string(),
            diff.to_string(),
            String::new(),
            diff.to_string(),
            false,
            true,
        );
        message.tool = "write".to_string();
        message.output = state.to_string();
        message
    };
    let messages = vec![
        ChatMessage::user("第一问"),
        write("c1", "old.rs", "--- 修改前\n+++ 修改后\n+ one\n", "已写入"),
        ChatMessage::assistant("第一答"),
        ChatMessage::user("第二问"),
        write(
            "c2",
            "a.rs",
            "--- 修改前\n+++ 修改后\n+ one\n+ two\n- old\n",
            "已写入",
        ),
        write("c3", "a.rs", "--- 修改前\n+++ 修改后\n+ three\n", "已写入"),
        write("c4", "b.rs", "--- 修改前\n+++ 修改后\n+ gone\n", "已撤销"),
        ChatMessage::assistant("第二答"),
    ];
    let change = turn_change(&messages, 7).expect("the turn changed something");
    assert_eq!(change.files, 1, "the same file twice counts once");
    assert_eq!(change.added, 3);
    assert_eq!(change.removed, 1);
    assert_eq!(
        turn_change(&messages, 2).map(|change| change.added),
        Some(1)
    );
    assert!(turn_change(&messages, 1).is_none(), "no answer yet");
}

#[test]
fn epoch_days_become_calendar_dates_including_a_leap_day() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    assert_eq!(civil_from_days(19_783), (2024, 3, 1));
}

#[test]
fn a_shell_approval_yields_its_exact_command() {
    assert_eq!(command_from_approval("$ npm test"), "npm test");
    assert_eq!(
        command_from_approval("# 工作目录：sub\n$ npm test -- --run"),
        "npm test -- --run"
    );
    assert_eq!(
        command_from_approval("$ npm test\necho done"),
        "npm test\necho done",
        "a multi-line command keeps every line"
    );
    assert_eq!(command_from_approval("--- 修改前\n+++ 修改后\n+ hi"), "");
}

#[test]
fn an_unanswered_dialog_is_forgotten_before_it_blocks_a_click() {
    assert!(!picker_stale(Duration::from_secs(0)));
    assert!(!picker_stale(Duration::from_secs(179)));
    assert!(picker_stale(Duration::from_secs(181)));
}

#[test]
fn only_a_command_or_a_whole_snippet_may_be_allowed_for_the_session() {
    assert!(allow_session_button("shell", "$ npm test"));
    assert!(allow_session_button("python", "$ python -c\nprint(1)"));
    assert!(allow_session_button("node", "$ node -e\nconsole.log(1)"));
    assert!(!allow_session_button("write", "--- 修改前\n+++ 修改后\n"));
    assert!(!allow_session_button("delete", "删除文件：a.txt"));
    assert!(
        !allow_session_button(
            "python",
            "$ python -c\nprint(1)\n…（共 9000 字符，确认卡只显示前 4000 字符；执行的是完整代码）"
        ),
        "a clipped card cannot name the snippet it would remember"
    );
}

#[test]
fn a_quiet_answer_is_given_up_on_but_a_card_is_not() {
    let limit = STALL_LIMIT;
    // Nothing has arrived for longer than the limit, and no card is open.
    assert!(answer_stalled(limit, false));
    assert!(answer_stalled(limit + Duration::from_secs(600), false));
    // Progress arrived recently.
    assert!(!answer_stalled(Duration::from_secs(30), false));
    // A card, a question or a plan is waiting for the person: that is not a hang.
    assert!(!answer_stalled(limit + Duration::from_secs(600), true));
}

#[test]
fn a_credential_card_is_never_answered_by_a_rule() {
    // 完全权限, 会话记忆 and 自动审查 all leave it to the person.
    assert!(!approval_auto_answered(true, false, true, true, true));
    // A write the engine already ran still needs no card.
    assert!(approval_auto_answered(false, true, false, false, false));
    // And the ordinary rules still answer what they always answered.
    assert!(approval_auto_answered(false, false, true, false, false));
    assert!(approval_auto_answered(false, false, false, true, false));
    assert!(approval_auto_answered(false, false, false, false, true));
    assert!(!approval_auto_answered(false, false, false, false, false));
}

#[test]
fn a_file_card_may_be_allowed_for_the_session_by_its_path() {
    assert!(allow_file_session_button("write", "a.txt"));
    assert!(allow_file_session_button("edit", "src/main.rs"));
    // A card with no path cannot name what it would remember.
    assert!(!allow_file_session_button("write", "   "));
    // `replace` spans several files, and `delete` is not a write.
    assert!(!allow_file_session_button("replace", ""));
    assert!(!allow_file_session_button("delete", "a.txt"));
    assert!(!allow_file_session_button("shell", "a.txt"));
}

#[test]
fn a_file_session_key_names_one_normalised_path() {
    // The same file written three ways is one key.
    assert_eq!(file_session_key("a.txt"), file_session_key("./a.txt"));
    assert_eq!(file_session_key("a\\b.txt"), file_session_key("a/b.txt"));
    assert_eq!(file_session_key(" a.txt "), file_session_key("a.txt"));
    assert_ne!(file_session_key("a.txt"), file_session_key("b.txt"));
    // A file key never collides with a command key for the same text.
    assert_ne!(
        file_session_key("npm test"),
        session_key("shell", "npm test")
    );
}

#[test]
fn a_session_key_never_mixes_a_snippet_with_a_command() {
    let command = session_key("shell", "npm test");
    let snippet = session_key("python", "npm test");
    assert_ne!(command, snippet);
    assert_eq!(command, session_key("shell", "npm test"));
    // The key of a code card is the code the card shows, tool included.
    assert_eq!(
        session_key("python", &command_from_approval("$ python -c\nprint(1)")),
        session_key("python", "python -c\nprint(1)")
    );
}

#[test]
fn a_field_row_is_measured_the_way_the_layout_draws_it() {
    // A one-line box is 30px tall with a 20px line: the text starts 5px down.
    assert_eq!(field_text_top(30., 1, false), 5.);
    assert_eq!(field_row_at(0., 30., 1, false), 0);
    assert_eq!(field_row_at(29., 30., 1, false), 0);

    // A block of lines starts under the box's own padding instead.
    assert_eq!(field_text_top(120., 3, true), 4.);
    assert_eq!(
        field_row_at(0., 120., 3, true),
        0,
        "the padding picks line 0"
    );
    assert_eq!(field_row_at(4., 120., 3, true), 0);
    assert_eq!(field_row_at(24., 120., 3, true), 1);
    assert_eq!(field_row_at(44., 120., 3, true), 2);
    assert_eq!(
        field_row_at(999., 120., 3, true),
        2,
        "clamped to the last line"
    );

    // A pasted multi-line draft in the composer is centred as a block.
    assert_eq!(field_text_top(80., 2, false), 20.);
    assert_eq!(field_row_at(20., 80., 2, false), 0);
    assert_eq!(field_row_at(40., 80., 2, false), 1);
}

#[test]
fn wrapped_rows_start_where_the_caret_and_the_click_look_for_them() {
    // One row per manual line, and an empty last line still gets its own row so
    // a caret after the final newline has somewhere to sit.
    assert_eq!(logical_rows("ab\ncd"), vec![0..2, 3..5]);
    assert_eq!(logical_rows(""), vec![0..0]);
    assert_eq!(logical_rows("a\n"), vec![0..1, 2..2]);

    let rows = logical_rows("one\ntwo\nthree");
    assert_eq!(row_of_caret(&rows, 0), 0);
    assert_eq!(
        row_of_caret(&rows, 3),
        0,
        "the break belongs to the row above"
    );
    assert_eq!(row_of_caret(&rows, 4), 1);
    assert_eq!(row_of_caret(&rows, 12), 2, "past the end still finds a row");

    // A folded row is a slice of one manual line: the caret at the break sits at
    // the start of the second piece, and the ranges stay inside the text.
    let folded = vec![0..3, 3..7, 8..10];
    assert_eq!(row_of_caret(&folded, 3), 1);
    assert_eq!(row_of_caret(&folded, 7), 1);
    assert_eq!(row_of_caret(&folded, 8), 2);
}

#[test]
fn a_failed_turn_only_falls_back_before_any_text_or_action() {
    assert!(should_fallback("", false, false));
    assert!(should_fallback("  \n", false, false));
    assert!(
        !should_fallback("半截回答", false, false),
        "text already shown must not be repeated"
    );
    assert!(
        !should_fallback("", true, false),
        "a turn that ran a tool must not run it again"
    );
    assert!(
        !should_fallback("", false, true),
        "a pending approval is still the user's decision"
    );
}

#[test]
fn a_checkpoint_round_trips_through_the_conversation_folder() {
    let dir = std::env::temp_dir().join(format!("dsh-compact-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(read_checkpoint(&dir).is_none(), "a fresh folder has none");

    let checkpoint = Checkpoint {
        summary: "## 主要请求与意图\n- 把压缩做完".to_string(),
        turns: 12,
        model: "deepseek-chat".to_string(),
        at_ms: 7,
        span: 42,
    };
    write_checkpoint(&dir, &checkpoint).unwrap();
    assert_eq!(read_checkpoint(&dir), Some(checkpoint.clone()));

    // A damaged file is treated as "no checkpoint", never as an error: the
    // recap would otherwise be lost for good.
    std::fs::write(dir.join(COMPACT_FILE), "{").unwrap();
    assert!(read_checkpoint(&dir).is_none());

    // An empty summary is not worth injecting either.
    std::fs::write(dir.join(COMPACT_FILE), r#"{"summary":"  "}"#).unwrap();
    assert!(read_checkpoint(&dir).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_checkpoint_prompt_names_the_model_and_clips_a_runaway_summary() {
    let checkpoint = Checkpoint {
        summary: "长".repeat(COMPACT_NOTE_TOKENS * 4 + 500),
        turns: 3,
        model: "m".to_string(),
        at_ms: 0,
        span: 0,
    };
    let text = checkpoint_prompt(&checkpoint);
    assert!(text.contains("由 m 压缩"), "{text}");
    assert!(text.contains("中间省略"), "the clipped middle is marked");
    assert!(
        estimate_tokens(&text) <= COMPACT_NOTE_TOKENS + 200,
        "{} tokens",
        estimate_tokens(&text)
    );

    let plain = Checkpoint {
        summary: "摘要".to_string(),
        ..Checkpoint::default()
    };
    assert!(!checkpoint_prompt(&plain).contains("压缩）"));
}

#[test]
fn compaction_only_asks_for_turns_the_window_dropped() {
    let short = vec![ChatMessage::user("问题"), ChatMessage::assistant("回答")];
    assert!(compaction_span(&short, None).is_none());

    let mut messages = Vec::new();
    for index in 0..(MAX_TURNS + 4) {
        messages.push(ChatMessage::user(format!("问题 {index}")));
        messages.push(ChatMessage::assistant(format!("回答 {index}")));
    }
    let (span, fingerprint) = compaction_span(&messages, None).expect("a span to condense");
    assert!(span.len() <= COMPACT_SPAN_TURNS);
    // The window keeps the newest `MAX_TURNS` turns, so 48 of these 88 were
    // dropped; the span takes the newest of those, ending at the last one.
    assert_eq!(
        span.last().map(|turn| turn.text.as_str()),
        Some("回答 23"),
        "the span ends at the last dropped turn"
    );

    // The same content is not summarized twice.
    let stored = Checkpoint {
        summary: "摘要".to_string(),
        turns: span.len(),
        model: "m".to_string(),
        at_ms: 1,
        span: fingerprint,
    };
    assert!(compaction_span(&messages, Some(&stored)).is_none());

    // New turns change the fingerprint, so a later question does compact again.
    messages.push(ChatMessage::user("问题 额外".to_string()));
    messages.push(ChatMessage::assistant("回答 额外".to_string()));
    assert!(compaction_span(&messages, Some(&stored)).is_some());
}

#[test]
fn a_long_conversation_keeps_a_recap_of_its_opening() {
    let mut messages = Vec::new();
    for index in 0..(MAX_TURNS + 6) {
        messages.push(ChatMessage::user(format!("问题 {index}")));
        messages.push(ChatMessage::assistant(format!("回答 {index}")));
    }
    let recap = history_recap(&messages);
    assert!(recap.contains("最早的"), "{recap}");
    assert!(
        recap.contains("问题 0"),
        "the opening request survives: {recap}"
    );
    assert!(!recap.contains("问题 99"), "{recap}");
    assert!(
        estimate_tokens(&recap) <= HISTORY_RECAP_TOKENS + 40,
        "recap is {} tokens",
        estimate_tokens(&recap)
    );
}

#[test]
fn a_short_conversation_needs_no_recap() {
    let messages = vec![ChatMessage::user("问题"), ChatMessage::assistant("回答")];
    assert!(history_recap(&messages).is_empty());
}

#[test]
fn the_workspace_map_lists_shallow_structure_and_skips_noise() {
    let root = std::env::temp_dir().join(format!("dsh-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src").join("app")).unwrap();
    std::fs::create_dir_all(root.join("node_modules").join("pkg")).unwrap();
    std::fs::create_dir_all(root.join("workspace").join("chat-1")).unwrap();
    std::fs::create_dir_all(root.join("chat-7")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "").unwrap();
    std::fs::write(root.join("src").join("main.rs"), "").unwrap();
    std::fs::write(root.join("workspace").join("chat-1").join("draft.txt"), "").unwrap();

    let map = workspace_map(
        &root,
        Some(&root.join("workspace").join("chat-1")),
        2,
        180,
        4_000,
    );
    assert!(map.contains("Cargo.toml"), "{map}");
    assert!(map.contains("src/"), "{map}");
    assert!(map.contains("  main.rs"), "{map}");
    assert!(!map.contains("node_modules"), "{map}");
    assert!(
        !map.contains("chat-1"),
        "the chat folder is not mapped: {map}"
    );
    assert!(
        !map.contains("chat-7"),
        "other chat folders are not mapped: {map}"
    );

    let capped = workspace_map(&root, None, 1, 1, 4_000);
    assert!(capped.contains("省略"), "{capped}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_environment_prompt_names_platform_model_and_folder() {
    let prompt = environment_prompt(std::path::Path::new("C:\\work\\chat-1"), "deepseek-chat");
    assert!(prompt.contains(platform_label()), "{prompt}");
    assert!(prompt.contains("deepseek-chat"), "{prompt}");
    assert!(prompt.contains("chat-1"), "{prompt}");
    assert!(prompt.contains("UTC 日期"), "{prompt}");
}

#[test]
fn an_approval_diff_uses_the_answer_column_and_leaves_room_for_its_bar() {
    let (width, text) = approval_detail_widths(664.);
    // The card leaves the answer column minus its own 1px border on each side.
    assert_eq!(width, 662.);
    assert_eq!(text, width - DETAIL_BAR - 2. * DETAIL_PAD - 12.);
    assert!(width > text + DETAIL_BAR, "the bar has its own column");
    let (narrow, narrow_text) = approval_detail_widths(0.);
    assert!(narrow >= 120. && narrow_text >= 80.);
    assert!(approval_detail_widths(500.).0 < approval_detail_widths(700.).0);
}

#[test]
fn a_tool_detail_shrinks_with_the_window_and_keeps_a_right_margin() {
    let (width, text) = tool_detail_widths(664.);
    // Column is 664 + 24; the detail gives up its indent and a margin.
    assert_eq!(width, 664. + 24. - DETAIL_MARGIN - DETAIL_INDENT);
    assert_eq!(text, width - DETAIL_BAR - 2. * DETAIL_PAD - 12.);
    // A narrow window still leaves a usable box instead of a negative width.
    let (width, text) = tool_detail_widths(0.);
    assert!(width >= 120., "{width}");
    assert!(text >= 80., "{text}");
    // Shrinking the column never grows the box.
    assert!(tool_detail_widths(500.).0 < tool_detail_widths(700.).0);
}

#[test]
fn a_tool_detail_shows_its_bar_only_when_there_is_more_to_read() {
    // Content that fits, or only a hair over: a thumb that cannot move is noise.
    assert!(bar_geometry(px(320.), px(0.)).is_none());
    assert!(bar_geometry(px(320.), px(8.)).is_none());
    assert!(bar_geometry(px(0.), px(400.)).is_none());
    let (track, thumb) = bar_geometry(px(320.), px(480.)).expect("content overflows");
    assert_eq!(track, px(316.));
    // 320 visible of 800 total -> a little under half the track.
    assert!(thumb > px(120.) && thumb < px(130.), "{thumb:?}");
    // A very long result stops shrinking at 15% of the track, so the thumb
    // stays big enough to grab.
    let (track, thumb) = bar_geometry(px(320.), px(6000.)).expect("content overflows");
    assert_eq!(track, px(316.));
    assert_eq!(thumb, px(316. * 0.15));
}

#[test]
fn a_wheel_delta_never_pushes_a_table_past_its_ends() {
    // Offsets run from 0 at the top to -max at the end: the delta is added and
    // then pinned, which is what keeps a table that cannot move from handing
    // the wheel to the panel behind it.
    let max = px(200.);
    assert_eq!(wheel_offset(px(0.), px(-40.), max), px(-40.));
    assert_eq!(wheel_offset(px(-40.), px(-200.), max), px(-200.));
    assert_eq!(wheel_offset(px(0.), px(40.), max), px(0.));
    assert_eq!(wheel_offset(px(-180.), px(-40.), max), px(-200.));
    // Nothing to scroll: the offset stays at the top whatever the delta says.
    assert_eq!(wheel_offset(px(0.), px(-40.), px(0.)), px(0.));
}

#[test]
fn dragging_a_tool_bar_moves_by_the_same_ratio_as_its_thumb() {
    let (track, thumb) = bar_geometry(px(320.), px(480.)).expect("content overflows");
    let travel = track - thumb;
    // Pulling the thumb from the top to the bottom of its travel covers the
    // whole 480px of hidden content.
    let max = px(480.);
    let at_end = (px(0.) - travel) * (max / travel);
    assert_eq!(at_end, -max);
    let halfway = (px(0.) - travel / 2.) * (max / travel);
    assert!(halfway < px(0.) && halfway > -max, "{halfway:?}");
}

#[test]
fn a_file_card_badge_names_the_kind_of_file() {
    use super::file_badge;
    assert_eq!(file_badge("report.pdf"), "PDF");
    assert_eq!(file_badge("数据.xlsx"), "XLSX");
    // The last dot decides, so a double extension reads as the outer one.
    assert_eq!(file_badge("archive.tar.gz"), "GZ");
    // Nothing worth reading: no dot, a leading dot, or a tail longer than an
    // extension ever is (`docx`, `json` and `html` are four characters).
    assert_eq!(file_badge("Makefile"), "文件");
    assert_eq!(file_badge(".env"), "文件");
    assert_eq!(file_badge("notes.longer"), "文件");
}

#[test]
fn a_step_file_needs_a_path_but_takes_its_name_from_it() {
    // The window draws a card per entry; one without a path would open nothing.
    let listed = serde_json::json!({
        "name": "",
        "path": "C:\\work\\报告.pdf",
        "bytes": 2048,
    });
    let file = crate::ai::step_file(&listed).expect("有路径就该算数");
    assert_eq!(file.name, "报告.pdf");
    assert_eq!(file.bytes, 2048);
    assert!(crate::ai::step_file(&serde_json::json!({ "name": "x.pdf" })).is_none());
    assert!(crate::ai::step_file(&serde_json::json!({ "path": "  " })).is_none());
}

#[test]
fn a_tool_row_summary_leaves_room_for_its_badge_and_chip() {
    let width = tool_row_text_width(664.);
    assert!(width < 664. + 24. - 104., "{width}");
    assert!(width > 400., "{width}");
    assert_eq!(tool_row_text_width(0.), 80.);
}

#[test]
fn joined_prompt_keeps_the_separators_the_engine_expects() {
    // The breakdown is built from blocks that are joined back into the single
    // string the model receives, so the separators have to be exact: a stray
    // blank line changes every request the window sends.
    let blocks = vec![
        PromptBlock {
            kind: PromptKind::System,
            label: "基础指令",
            sep: "",
            text: "A".to_string(),
        },
        PromptBlock {
            kind: PromptKind::System,
            label: "运行环境",
            sep: "\n\n",
            text: "B".to_string(),
        },
        PromptBlock {
            kind: PromptKind::System,
            label: "预算与权限",

            sep: "\n",
            text: "C".to_string(),
        },
        PromptBlock {
            kind: PromptKind::Injected,
            label: "空块",
            sep: "\n\n",
            text: String::new(),
        },
        PromptBlock {
            kind: PromptKind::Injected,
            label: "项目指令",
            sep: "\n\n",
            text: "D".to_string(),
        },
    ];
    assert_eq!(joined_prompt(&blocks), "A\n\nB\nC\n\nD");
    assert_eq!(joined_prompt(&[]), "");
}

#[test]
fn every_context_category_has_its_own_name_and_colour() {
    // The panel lists them and the bar draws them, so a duplicated label or a
    // repeated colour would make two categories indistinguishable.
    let mut labels: Vec<&str> = ContextKind::ALL.iter().map(|kind| kind.label()).collect();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), 9);
    let mut colours: Vec<u32> = ContextKind::ALL.iter().map(|kind| kind.color()).collect();
    colours.sort_unstable();
    colours.dedup();
    assert_eq!(colours.len(), 9);
    // The one category that can be empty still says where to turn it on.
    assert_eq!(
        ContextKind::Skills.empty_note(),
        Some("没有启用的技能（左侧 Skill 页开启）")
    );
    assert_eq!(ContextKind::System.empty_note(), None);
}

#[test]
fn every_context_category_round_trips_through_its_stored_key() {
    // `requests.jsonl` names categories by key, so a duplicated or unreadable
    // key would put a request's pieces in the wrong row after a restart.
    for kind in ContextKind::ALL {
        assert_eq!(ContextKind::from_key(kind.key()), Some(kind));
    }
    let mut keys: Vec<&str> = ContextKind::ALL.iter().map(|kind| kind.key()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 9);
    // A key this build does not know is skipped, not guessed at.
    assert_eq!(ContextKind::from_key("cache"), None);
}

#[test]
fn the_status_capsule_writes_its_figures_the_way_it_reads_them() {
    // Questions then round trips: `3 问 7 步`, or a bare zero before anything
    // was asked. `问` is not `轮` because `轮` already means one model round
    // trip, and the first figure here is how many questions were sent.
    assert_eq!(asked_label(0, 0), "0 问 0 步");
    assert_eq!(asked_label(1, 3), "1 问 3 步");
    assert_eq!(asked_label(12, 40), "12 问 40 步");

    // A speed needs both a token count and a time, so an unknown one is silent
    // instead of printing a meaningless `0 tok/s`.
    assert_eq!(speed_label(0), None);
    assert_eq!(speed_label(1), Some("1 tok/s".to_string()));
    assert_eq!(speed_label(74), Some("74 tok/s".to_string()));

    // Cache shares: a provider that reports nothing reads `—`, and a sliver is
    // `<1%` rather than a rounded-away `0%`.
    assert_eq!(share_text(false, 0.0), "—");
    assert_eq!(share_text(true, 0.0), "0%");
    assert_eq!(share_text(true, 0.0005), "<1%");
    assert_eq!(share_text(true, 0.5), "50%");
    assert_eq!(share_text(true, 0.952), "95.2%");
    assert_eq!(share_text(true, 0.999), "99.9%");
    assert_eq!(share_text(true, 1.5), "100%");
    assert_eq!(cache_share_label(true, 0.96), "缓存命中 96%");
    assert_eq!(cache_share_label(false, 0.0), "缓存命中 —");

    // The window gauge clamps at both ends and never prints `-0%`.
    assert_eq!(percent_label(0.0), "0%");
    assert_eq!(percent_label(0.4), "<1%");
    assert_eq!(percent_label(12.4), "12%");
    assert_eq!(percent_label(94.6), "95%");
    assert_eq!(percent_label(180.0), "100%");

    // Token totals shrink the same way the rest of the interface does.
    assert_eq!(token_amount(0), "0");
    assert_eq!(token_amount(792), "792");
    assert_eq!(token_amount(1_800), "1.8K");
    assert_eq!(token_amount(38_112), "38.1K");
    assert_eq!(token_amount(1_234_567), "1.23M");
}

#[test]
fn the_capsule_counts_questions_steps_and_the_billed_total() {
    use super::{Chat, ChatMessage, chat_metrics};
    let mut chat = Chat::empty("chat-1".to_string());
    let model = "deepseek-chat";
    // Nothing has been asked yet: every figure stays quiet instead of lying.
    let bare = chat_metrics(&chat, model);
    assert_eq!(bare.asked, 0);
    assert_eq!(bare.steps, 0);
    assert_eq!(bare.speed, 0);
    assert_eq!(bare.billed, 0);
    assert!(!bare.cache_known);
    assert_eq!(bare.cache_share, 0.0);
    assert_eq!(bare.context_percent, 0.0);

    chat.messages.push(ChatMessage::user("q"));
    chat.messages.push(ChatMessage::tool("read · a.txt", false));
    let mut answer = ChatMessage::assistant("a");
    answer.tokens = 900;
    answer.elapsed_ms = 3_000;
    // The engine's own counters: two round trips, the first calling two tools and
    // the second being the answer itself.
    answer.round_tokens = vec![(60_000, 400, 2), (61_000, 500, 0)];
    answer.rounds = 2;
    answer.context = 62_000;
    answer.cache_hits = 118_000;
    chat.messages.push(answer);

    let metrics = chat_metrics(&chat, model);
    // One question is one 问; its two round trips are two 步, whatever the rows
    // in the transcript happen to hold.
    assert_eq!(asked_label(metrics.asked, metrics.steps), "1 问 2 步");
    assert_eq!(metrics.speed, 300, "900 tokens in 3 s");
    // The billed total is what the endpoint reported for the answer.
    assert_eq!(metrics.billed, 900);
    assert!(metrics.cache_known);
    // 118k cached out of a 121k prompt: the conversation was nearly all cache.
    assert!(metrics.cache_share > 0.97);

    // A second question adds a 问 and its own round, and both figures count the
    // whole conversation instead of the newest answer alone.
    chat.messages.push(ChatMessage::user("q2"));
    let mut second = ChatMessage::assistant("b");
    second.tokens = 100;
    second.elapsed_ms = 1_000;
    second.round_tokens = vec![(70_000, 100, 0)];
    second.rounds = 1;
    second.context = 80_000;
    second.cache_hits = 0;
    chat.messages.push(second);

    let metrics = chat_metrics(&chat, model);
    assert_eq!(asked_label(metrics.asked, metrics.steps), "2 问 3 步");
    assert_eq!(metrics.speed, 100, "the newest answer sets the speed");
    assert_eq!(metrics.billed, 1_000, "900 + 100 across both answers");
    // The share covers both requests: 118k cached out of 191k sent.
    assert!((metrics.cache_share - 118_000f64 / 191_000.).abs() < 1e-9);
    assert!(metrics.cache_known);
}

#[test]
fn an_answer_saved_without_round_counts_still_counts_one_trip() {
    use super::{Chat, ChatMessage, asked_label, chat_metrics};
    let mut chat = Chat::empty("chat-1".to_string());
    chat.messages.push(ChatMessage::user("q"));
    chat.messages.push(ChatMessage::tool("read · a.txt", false));
    chat.messages.push(ChatMessage::tool("grep · TODO", false));
    // The oldest shape: neither the round list nor the counts survived, so the
    // one trip an answer must have taken is all that can be claimed.
    let mut answer = ChatMessage::assistant("a");
    answer.tokens = 240;
    chat.messages.push(answer);

    let metrics = chat_metrics(&chat, "deepseek-chat");
    assert_eq!(asked_label(metrics.asked, metrics.steps), "1 问 1 步");
    assert_eq!(metrics.billed, 240);
}

#[test]
fn two_transcripts_naming_the_same_folder_are_split_apart() {
    use super::unique_dirs;
    let make = |dir: &str, title: &str| {
        let mut chat = Chat::empty(dir.to_string());
        chat.title = title.to_string();
        chat
    };
    let chats = unique_dirs(vec![
        make("chat-1", "一"),
        make("chat-1", "二"),
        make("", "三"),
    ]);
    assert_eq!(chats.len(), 3);
    assert_eq!(chats[0].dir, "chat-1");
    assert_ne!(
        chats[1].dir, "chat-1",
        "the duplicate got a folder of its own"
    );
    assert!(!chats[2].dir.is_empty(), "an empty name got one too");
    let dirs: std::collections::HashSet<&str> =
        chats.iter().map(|chat| chat.dir.as_str()).collect();
    assert_eq!(dirs.len(), 3, "every folder is unique now");
}

#[test]
fn removing_a_connection_keeps_the_active_one_selected() {
    use super::active_after_removal;
    // The active card is the second one; taking the first out moves it down.
    assert_eq!(active_after_removal(1, 0, 2), 0);
    // The active card itself was removed: the one after it takes over.
    assert_eq!(active_after_removal(2, 0, 2), 1);
    assert_eq!(active_after_removal(0, 0, 2), 0);
    // Removing a later card changes nothing.
    assert_eq!(active_after_removal(1, 2, 2), 1);
    // Nothing left to be active.
    assert_eq!(active_after_removal(0, 0, 0), 0);
}
#[test]
fn a_sidebar_hit_names_the_message_it_found() {
    use super::{Chat, ChatMessage, chat_match};
    let mut chat = Chat::empty("chat-1".to_string());
    chat.messages.push(ChatMessage::user("你好"));
    chat.messages.push(ChatMessage::assistant(
        "关于 borrow checker 的解释：它保证引用有效。",
    ));
    let (index, snippet) = chat_match(&chat, "borrow").expect("hit");
    assert_eq!(index, 1);
    assert!(snippet.contains("borrow"), "{snippet}");
    assert!(!snippet.contains('\n'), "the snippet is one line");
    assert!(chat_match(&chat, "missing").is_none());
    assert!(chat_match(&chat, "").is_none());
}

#[test]
fn an_exported_conversation_is_readable_markdown() {
    use super::{Chat, ChatMessage, transcript_markdown};
    let mut chat = Chat::empty("chat-1".to_string());
    chat.title = "测试对话".to_string();
    chat.messages.push(ChatMessage::user("第一个问题"));
    let mut answer = ChatMessage::assistant("回答\n第二行");
    answer.model = "deepseek-chat".to_string();
    chat.messages.push(answer);
    chat.messages
        .push(ChatMessage::tool("read · a.txt\nsecond line", false));
    chat.messages.push(ChatMessage::note("提示", false));

    let text = transcript_markdown(&chat);
    assert!(text.starts_with("# 测试对话\n\n"));
    assert!(text.contains("## 你\n\n第一个问题"));
    assert!(text.contains("## 助手（deepseek-chat）"));
    assert!(text.contains("回答\n第二行"));
    assert!(text.contains("- 工具：read · a.txt second line"));
    assert!(text.contains("> 提示"));
}

#[test]
fn a_code_gutter_numbers_every_drawn_line() {
    use super::message::line_numbers;
    assert_eq!(line_numbers(""), (1, "1".to_string()));
    assert_eq!(line_numbers("a"), (1, "1".to_string()));
    assert_eq!(line_numbers("a\nb"), (2, "1\n2".to_string()));
    // A trailing newline draws no extra line, and a blank line inside does.
    assert_eq!(line_numbers("a\nb\n"), (2, "1\n2".to_string()));
    assert_eq!(line_numbers("a\n\nb"), (3, "1\n2\n3".to_string()));
}

#[test]
fn interaction_cards_settle_when_the_request_ends() {
    use crate::app::util::settle_interactions;

    let mut chat = Chat::empty("chat-1".to_string());
    chat.plan = true;
    chat.todos = vec![TodoItem {
        text: "读代码".to_string(),
        status: TodoStatus::Active,
    }];
    chat.messages.push(ChatMessage::question(
        "q1".to_string(),
        "选哪个方案？".to_string(),
        r#"{"options":["甲","乙"],"allowCustom":false}"#.to_string(),
    ));
    chat.messages.push(ChatMessage::plan_review(
        "p1".to_string(),
        "# 计划\n\n先读代码".to_string(),
    ));
    settle_interactions(&mut chat, "已失效");
    assert_eq!(chat.messages[0].output, "已失效");
    assert_eq!(chat.messages[1].output, "已失效");
    // The todo list and plan flag are conversation state, not interaction cards:
    // settling a finished request must not silently clear them.
    assert!(chat.plan);
    assert_eq!(chat.todos.len(), 1);
    assert_eq!(chat.todos[0].status, TodoStatus::Active);
}

#[test]
fn auto_review_only_grants_low_risk_actions() {
    use crate::app::util::auto_review_allows;

    // The window calls the rule with the commands the person added; these cases
    // are about the built-in list, so the added list is empty.
    let allows = |tool: &str, command: &str, path: &str, diff: &str| {
        auto_review_allows(tool, command, path, diff, &[])
    };

    // Ordinary workspace writes.
    assert!(allows("write", "", "src/main.rs", "diff"));
    assert!(allows("edit", "", "README.md", "diff"));
    // Copying and folder creation are the same low-risk class: the target is
    // what gets checked, and it may not leave the workspace or touch secrets.
    assert!(allows("copy", "", "assets/logo.png", ""));
    assert!(allows("mkdir", "", "out/reports", ""));
    assert!(!allows("copy", "", "../outside.png", ""));
    assert!(!allows("mkdir", "", "C:\\Windows\\x", ""));
    assert!(!allows("mkdir", "", ".git/hooks", ""));
    // Outside the workspace, sensitive files, or a huge rewrite.
    assert!(!allows("write", "", "C:\\Windows\\x.txt", "d"));
    assert!(!allows("write", "", "../outside.txt", "d"));
    assert!(!allows("write", "", ".env", "d"));
    assert!(!allows("write", "", "keys/server.pem", "d"));
    assert!(!allows("write", "", ".git/config", "d"));
    assert!(!allows("write", "", "big.txt", &"x".repeat(200_001)));
    // Read-only commands.
    assert!(allows("shell", "git status", "", ""));
    assert!(allows("shell", "git branch --show-current", "", ""));
    assert!(allows("shell", "cargo test --release", "", ""));
    assert!(allows("shell", "dir", "", ""));
    // Anything that can chain, delete, publish or run code still asks.
    assert!(!allows("shell", "git status && rm -rf .", "", ""));
    assert!(!allows("shell", "git branch -D main", "", ""));
    assert!(!allows("shell", "git diff --output=diff.patch", "", ""));
    assert!(!allows("shell", "cargo clippy --fix", "", ""));
    assert!(!allows("shell", "git push", "", ""));
    assert!(!allows("shell", "cargo fmt", "", ""));
    assert!(!allows("shell", "npm test", "", ""));
    assert!(!allows("python", "", "", ""));
    assert!(!allows("delete", "", "gone.txt", ""));
    // Command substitution runs a second command inside an allowed one, so it
    // must ask instead of slipping through `cat` / `git diff`.
    assert!(!allows("shell", "cat $(rm -rf .)", "", ""));
    assert!(!allows("shell", "git log $(curl -s https://x/y)", "", ""));
    assert!(!allows("shell", "cat `rm -rf .`", "", ""));
}

#[test]
fn recent_images_are_only_the_last_turns_pictures() {
    let mut messages = vec![
        ChatMessage::user("第一问"),
        ChatMessage::assistant("第一答"),
        ChatMessage::user("第二问"),
    ];
    let picture = |name: &str| {
        let mut message = ChatMessage::tool("screenshot · 全屏", false);
        message.image_mime = "image/png".to_string();
        message.image_path = name.to_string();
        message
    };
    messages.push(picture("old.png"));
    messages.push(ChatMessage::assistant("第二答"));
    messages.push(ChatMessage::user("第三问"));
    messages.push(picture("a.png"));
    messages.push(picture("b.png"));
    messages.push(picture("c.png"));
    messages.push(ChatMessage::assistant("第三答"));

    // Only the turn that just finished, and only the newest two: a picture from
    // three questions ago is not what a follow-up is about.
    assert_eq!(
        recent_images(&messages, 2),
        vec![
            ("image/png".to_string(), "b.png".to_string()),
            ("image/png".to_string(), "c.png".to_string())
        ]
    );
    assert!(recent_images(&messages, 0).is_empty());

    let no_pictures = vec![
        ChatMessage::user("问"),
        ChatMessage::tool("read · a.txt", false),
        ChatMessage::assistant("答"),
    ];
    assert!(recent_images(&no_pictures, 2).is_empty());
}

#[test]
fn a_text_only_endpoint_is_not_offered_the_image_tools() {
    use crate::config::ModeSelection;

    let selection = ModeSelection {
        plugins: Vec::new(),
        skills: Vec::new(),
        mcp_servers: Vec::new(),
    };
    let seen = mode_tools_for(&selection, true);
    assert!(seen.contains(&"read_image"));
    assert!(seen.contains(&"screenshot"));
    let text_only = mode_tools_for(&selection, false);
    assert!(!text_only.contains(&"read_image"));
    assert!(!text_only.contains(&"screenshot"));
    // Nothing else depends on the endpoint: only the two image tools drop out.
    assert_eq!(text_only.len() + 2, seen.len());
}

#[test]
fn a_request_only_records_the_tools_it_actually_sends() {
    // The cache from the previous run: a built-in, a plugin tool, two server
    // tools and one nobody claims any more.
    let cached = vec![
        ("read".to_string(), "read-schema".to_string()),
        ("office_word".to_string(), "office-schema".to_string()),
        ("mcp_files_read".to_string(), "files-schema".to_string()),
        ("mcp_git_log".to_string(), "git-schema".to_string()),
        ("stale_tool".to_string(), "stale-schema".to_string()),
    ];
    let plugin_tools = vec![("office".to_string(), "office_word".to_string())];
    let mcp_tools = vec![
        ("files".to_string(), "mcp_files_read".to_string()),
        ("git".to_string(), "mcp_git_log".to_string()),
    ];
    let builtins: Vec<&str> = vec!["read", "write"];

    // Full mode as it was: everything the previous run reported and still valid.
    let all = request_schemas(
        &cached,
        true,
        &builtins,
        &["office".to_string()],
        &["files".to_string(), "git".to_string()],
        &plugin_tools,
        &mcp_tools,
    );
    assert_eq!(all.len(), 4, "{all:?}");
    assert!(!all.iter().any(|(name, _)| name == "stale_tool"));

    // Every MCP server switched off between two questions: their definitions are
    // not sent, so the record must not name them either.
    let without_mcp = request_schemas(
        &cached,
        true,
        &builtins,
        &["office".to_string()],
        &[],
        &plugin_tools,
        &mcp_tools,
    );
    assert_eq!(
        without_mcp
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        vec!["read", "office_word"]
    );

    // Basic mode: no plugins, no servers, and no built-in outside the list.
    let basic = request_schemas(
        &cached,
        true,
        &builtins,
        &[],
        &[],
        &plugin_tools,
        &mcp_tools,
    );
    assert_eq!(basic, vec![("read".to_string(), "read-schema".to_string())]);

    // Tools switched off in the conversation: nothing travels at all.
    assert!(
        request_schemas(
            &cached,
            false,
            &builtins,
            &[],
            &[],
            &plugin_tools,
            &mcp_tools
        )
        .is_empty()
    );

    // The plugin prompts follow the same rule: a plugin switched off keeps none
    // of its injected text in the next request's record.
    let prompts = vec![
        ("office".to_string(), "office-prompt".to_string()),
        ("win32-input".to_string(), "input-prompt".to_string()),
    ];
    assert_eq!(request_prompts(&prompts, &[]), Vec::new());
    assert_eq!(
        request_prompts(&prompts, &["office".to_string()]),
        vec![("office".to_string(), "office-prompt".to_string())]
    );
}

#[test]
fn auto_review_accepts_commands_the_person_added() {
    use crate::app::util::auto_review_allows;

    let extra = vec!["npm test".to_string(), "pytest".to_string()];
    // The entry covers the whole line, arguments included.
    assert!(auto_review_allows("shell", "npm test", "", "", &extra));
    assert!(auto_review_allows(
        "shell",
        "npm test -- --watch",
        "",
        "",
        &extra
    ));
    assert!(auto_review_allows("shell", "pytest -q", "", "", &extra));
    // A prefix covers its own program, not a neighbour that merely starts alike.
    assert!(!auto_review_allows("shell", "npm install", "", "", &extra));
    assert!(!auto_review_allows("shell", "npm testx", "", "", &extra));
    // The guards still apply: chaining, redirecting or substituting commands from
    // the list still asks, which is the only thing keeping the list from being a
    // full-access switch.
    assert!(!auto_review_allows(
        "shell",
        "npm test && rm -rf .",
        "",
        "",
        &extra
    ));
    assert!(!auto_review_allows(
        "shell",
        "pytest > out.txt",
        "",
        "",
        &extra
    ));
    assert!(!auto_review_allows(
        "shell",
        "pytest $(curl -s https://x/y)",
        "",
        "",
        &extra
    ));
    // Code runners are not shell commands, so a configured prefix never applies.
    assert!(!auto_review_allows("python", "", "", "", &extra));
    // `wait_for` only polls, so a dev loop does not need a card for every check;
    // deletion and batch replace still ask.
    assert!(auto_review_allows("wait_for", "", "", "", &extra));
    assert!(!auto_review_allows("delete", "", "a.txt", "", &extra));
    assert!(!auto_review_allows("replace", "", "", "", &extra));
}

#[test]
fn sliders_land_on_the_notch_their_value_stands_for() {
    // Presets an older config may hold, and figures with no notch at all: the
    // knob has to sit somewhere honest, and picking it writes the notch back.
    // The default is not a notch of its own any more — it parks the knob on the
    // value the endpoint uses, so "恢复默认" is a move, not a blank rail.
    assert_eq!(
        max_tokens_notch(0, Reasoning::Off),
        2,
        "no thinking: the endpoint gives 8k"
    );
    assert_eq!(max_tokens_notch(0, Reasoning::Default), 5, "thinking: 64k");
    assert_eq!(max_tokens_notch(0, Reasoning::High), 5);
    assert_eq!(max_tokens_notch(0, Reasoning::Max), 6, "top effort: 128k");
    assert_eq!(max_tokens_notch(2_048, Reasoning::Off), 0);
    assert_eq!(max_tokens_notch(8_192, Reasoning::Off), 2);
    assert_eq!(max_tokens_notch(6_000, Reasoning::Off), 1, "nearest below");
    assert_eq!(
        max_tokens_notch(999_999, Reasoning::Off),
        7,
        "nearest above"
    );

    assert_eq!(
        window_notch(0, "deepseek-flash"),
        5,
        "the name table gives this model 1M"
    );
    assert_eq!(window_notch(0, "some-local-model"), 2, "unknown: 128k");
    assert_eq!(window_notch(32_768, "x"), 0);
    assert_eq!(window_notch(262_144, "x"), 3);
    assert_eq!(window_notch(200_000, "x"), 3, "closer to 256k than 128k");
    assert_eq!(window_notch(1_000_000, "x"), 5);

    assert_eq!(reasoning_notch(Reasoning::Off), 0);
    assert_eq!(reasoning_notch(Reasoning::Low), 1);
    assert_eq!(reasoning_notch(Reasoning::High), 2);
    assert_eq!(reasoning_notch(Reasoning::Max), 3);
    assert_eq!(
        reasoning_notch(Reasoning::Default),
        reasoning_notch(Reasoning::High),
        "endpoint thinks at high"
    );

    // The temperature row parks an unset value where DeepSeek sits, and reads a
    // stored figure back to the nearest quarter.
    assert_eq!(temperature_notch(None), 4);
    assert_eq!(temperature_notch(Some(0.0)), 0);
    assert_eq!(temperature_notch(Some(1.0)), 4);
    assert_eq!(temperature_notch(Some(1.25)), 5);
    assert_eq!(temperature_notch(Some(0.73)), 3);
    assert_eq!(temperature_notch(Some(9.0)), TEMPERATURE_NOTCHES - 1);
}

#[test]
fn input_cards_fit_the_panel_width() {
    // One card needs 240px; every further card another 252 (240 + the 12px gap).
    assert_eq!(input_columns(0.), 1);
    assert_eq!(input_columns(-10.), 1);
    assert_eq!(input_columns(f32::NAN), 1);
    assert_eq!(input_columns(200.), 1, "a narrow panel still gets the card");
    assert_eq!(input_columns(240.), 1);
    assert_eq!(input_columns(491.), 1);
    assert_eq!(input_columns(492.), 2);
    assert_eq!(input_columns(743.), 2);
    assert_eq!(input_columns(744.), 3);
    assert_eq!(input_columns(996.), 4);
    // The number of cards a row holds times the card, plus its gaps, fits.
    for columns in 1..=6 {
        let needed = INPUT_CARD_WIDTH * columns as f32 + INPUT_CARD_GAP * (columns as f32 - 1.);
        assert_eq!(input_columns(needed), columns);
        assert_eq!(input_columns(needed - 1.), (columns - 1).max(1));
    }
}

#[test]
fn code_sections_cap_and_label_source_files() {
    assert_eq!(code_language("steps/units.py"), "python");
    assert_eq!(code_language("steps/smoke.js"), "javascript");
    assert_eq!(code_language("src/cli.ts"), "typescript");
    assert_eq!(code_language("flows/login/flow.yml"), "yaml");
    assert_eq!(code_language("no-extension"), "text");
    let long: String = (0..805)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (body, cut) = clip_code(&long);
    assert!(cut, "a file past the cap says so");
    assert_eq!(body.lines().count(), 800, "and only the head is drawn");
    let (whole, cut) = clip_code("one\ntwo\n");
    assert!(!cut);
    assert_eq!(whole, "one\ntwo");
}

#[test]
fn a_masked_value_is_one_dot_per_character_and_keeps_the_caret() {
    assert_eq!(masked_text("abc"), "•••");
    assert_eq!(masked_text("a中b"), "•••", "a CJK character is one dot too");
    assert_eq!(masked_text("😀x"), "••");
    assert_eq!(masked_text(""), "");

    // The caret is measured in the dots on screen, not in the hidden bytes.
    let dot = "•".len();
    assert_eq!(masked_caret("abc", 0), 0);
    assert_eq!(masked_caret("abc", 1), dot);
    assert_eq!(masked_caret("中文", 3), dot);
    assert_eq!(masked_caret("中文", 6), 2 * dot);

    // A click lands on a dot; the character it hides is what the field edits.
    assert_eq!(masked_index("abc", 2 * dot), 2);
    assert_eq!(masked_index("中文", dot), 3);
    assert_eq!(masked_index("", dot), 0);
    assert_eq!(masked_index("中文", 999), 6, "past the end is the end");
}
