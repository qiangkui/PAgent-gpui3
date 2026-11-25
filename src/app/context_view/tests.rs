use super::{ChatMessage, ContextKind};
use super::{TimelineFlag, anchor_parts, request_rows, timeline_points};

#[test]
fn every_category_sits_at_its_own_index() {
    // Compositions are stored by position and read back with `as usize`, so
    // the declaration order and `ALL` have to stay identical; a category
    // inserted in the middle would otherwise be read as its neighbour.
    for (index, kind) in ContextKind::ALL.iter().enumerate() {
        assert_eq!(*kind as usize, index);
    }
}

#[test]
fn a_request_row_keeps_the_cache_hits_it_was_billed_with() {
    use super::request_rows;

    let mut answer = ChatMessage::assistant("a");
    answer.cache_hits = 640;
    answer.round_tokens = vec![(1000, 20, 0)];
    let (rows, _) = request_rows(&[answer], &std::collections::HashMap::new());
    assert_eq!(rows[0].cache_hits, 640);
    // Nothing reported leaves zero, which the table draws as "—".
    let (plain, ..) = request_rows(
        &[ChatMessage::assistant("a")],
        &std::collections::HashMap::new(),
    );
    assert_eq!(plain[0].cache_hits, 0);
}

#[test]
fn the_window_bar_keeps_room_for_compaction_and_the_answer() {
    use super::{WINDOW_PRESSURE, window_usage};

    // Half the window filled leaves headroom up to the engine's 80% line,
    // and the answer's reserved output sits in the slice at the far right.
    let half = window_usage(64_000, 128_000, 8_192).expect("known window");
    assert!((half.filled - 0.5).abs() < 1e-6, "filled {}", half.filled);
    assert!(
        (half.headroom - 0.3).abs() < 1e-6,
        "headroom {}",
        half.headroom
    );
    assert!(
        (half.reserve - 0.064).abs() < 1e-6,
        "reserve {}",
        half.reserve
    );
    assert_eq!(half.pressure, WINDOW_PRESSURE);
    // Past the line there is no headroom left, and the marker is not drawn.
    let full = window_usage(120_000, 128_000, 8_192).expect("known window");
    assert_eq!(full.headroom, 0.);
    assert!(full.filled > full.pressure);
    // A request bigger than the window cannot paint outside the bar.
    let over = window_usage(900_000, 128_000, 8_192).expect("known window");
    assert_eq!(over.filled, 1.);
    assert_eq!(over.headroom, 0.);
    // An unknown window size hides the bar instead of inventing a scale.
    assert!(window_usage(1_000, 0, 8_192).is_none());
}

#[test]
fn a_request_row_keeps_the_timing_it_was_billed_with() {
    use super::request_rows;

    let mut answer = ChatMessage::assistant("a");
    answer.elapsed_ms = 3_000;
    answer.model_ms = 2_100;
    answer.tool_ms = 640;
    let mut second = ChatMessage::assistant("b");
    second.model_ms = 400;
    second.tool_ms = 60;
    let (rows, _) = request_rows(&[answer, second], &std::collections::HashMap::new());
    // Rows come newest first; each carries its own split.
    assert_eq!(rows[0].model_ms, 400);
    assert_eq!(rows[0].tool_ms, 60);
    assert_eq!(rows[1].model_ms, 2_100);
    assert_eq!(rows[1].tool_ms, 640);
    // Nothing reported leaves zero, which hides the timing card.
    let (plain, _) = request_rows(
        &[ChatMessage::assistant("a")],
        &std::collections::HashMap::new(),
    );
    assert_eq!(plain[0].model_ms, 0);
    assert_eq!(plain[0].tool_ms, 0);
}

#[test]
fn the_timing_split_names_what_the_engine_did_not_measure() {
    use super::request_timing;

    // Model and tools are the engine's own clock; the card's third bucket is
    // the leftover of the answer's total, so nothing is counted twice.
    let split = request_timing(2_100, 640, 3_000).expect("measured");
    assert_eq!(split.model_ms, 2_100);
    assert_eq!(split.tool_ms, 640);
    assert_eq!(split.other_ms, 260);
    assert_eq!(split.total_ms, 3_000);
    // Two phases that add up to the whole answer leave nothing over.
    assert_eq!(request_timing(1_000, 0, 1_000).unwrap().other_ms, 0);
    // A measured phase can outlast the answer's stopwatch: the bar follows
    // the sum it can prove instead of printing a negative slice.
    let overflow = request_timing(1_200, 300, 1_000).expect("measured");
    assert_eq!(overflow.total_ms, 1_500);
    assert_eq!(overflow.other_ms, 0);
    // An older engine reported neither, and the card stays hidden.
    assert!(request_timing(0, 0, 3_000).is_none());
    assert!(request_timing(0, 0, 0).is_none());
}

#[test]
fn thinking_is_split_by_its_rows_when_the_engine_sends_no_figure() {
    use crate::chats::{StoredRequest, StoredRequestItem};

    // A gateway may omit `roundReasoning`; the thinking rows still say which
    // round each stretch belonged to, so the panel splits it where it
    // happened instead of repeating the question's total in every round.
    let mut answer = ChatMessage::assistant("a");
    answer.rounds = 2;
    answer.round_tokens = vec![(100, 10, 0), (200, 20, 0)];
    let messages = vec![
        ChatMessage::user("q"),
        ChatMessage::thinking("第一轮想", 1),
        ChatMessage::tool("read · a.txt", false),
        ChatMessage::thinking("第二轮多想一想", 2),
        answer,
    ];
    let stored = StoredRequest {
        turn: 1,
        at_ms: 1,
        model: "deepseek-flash".to_string(),
        tokens: 0,
        context: 0,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: "第一轮想\n\n第二轮多想一想".to_string(),
        reasoning_tokens: 0,
        items: vec![StoredRequestItem {
            kind: ContextKind::User.key().to_string(),
            label: "q".to_string(),
            text: "q".to_string(),
            tokens: 5,
        }],
    };
    let stored = std::collections::HashMap::from([(1u64, stored)]);
    let (rows, ..) = request_rows(&messages, &stored);
    let row = &rows[0];
    assert_eq!(
        row.round_reasoning_text,
        vec!["第一轮想".to_string(), "第二轮多想一想".to_string()]
    );
    let slot = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Reasoning)
        .expect("the category exists");
    assert_eq!(row.round_composition.len(), 2);
    assert!(row.round_reasoning[0] > 0 && row.round_reasoning[1] > 0);
    // Each round carries its own figure, and the two differ — the bug was
    // every round showing the question's whole total.
    assert_ne!(row.round_reasoning[0], row.round_reasoning[1]);
    assert_eq!(
        row.round_composition[0][slot],
        row.round_reasoning[0] as usize
    );
    assert_eq!(
        row.round_composition[1][slot],
        row.round_reasoning[1] as usize
    );
    let total = row.round_reasoning[0].saturating_add(row.round_reasoning[1]);
    assert_eq!(row.composition[slot], total);
}
#[test]
fn a_thinking_row_is_neither_a_question_nor_a_tool() {
    use super::request_rows;

    // Thinking rows sit in the transcript like tool rows, but they answer no
    // question and run nothing: the request list must not count them.
    let messages = vec![
        ChatMessage::user("q"),
        ChatMessage::thinking("先想一下", 1),
        ChatMessage::tool("read · a.txt", false),
        ChatMessage::assistant("a"),
    ];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tools, 1);
    assert_eq!(rows[0].turn, 1);
}
#[test]
fn the_stored_request_opens_its_thinking() {
    use crate::chats::{StoredRequest, StoredRequestItem, append_request};

    let dir = std::env::temp_dir().join(format!("dsh-flow-ui-thinking-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let stored = StoredRequest {
        turn: 1,
        at_ms: 1,
        model: "m".to_string(),
        tokens: 0,
        context: 0,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: "先想一下".to_string(),
        reasoning_tokens: 120,
        items: vec![StoredRequestItem {
            kind: ContextKind::User.key().to_string(),
            label: "q".to_string(),
            text: "q".to_string(),
            tokens: 5,
        }],
    };
    append_request(&dir, "chat-1", &stored).expect("append");
    let parts = super::stored_parts(&dir, "chat-1", 1).expect("parts");
    let part = parts
        .iter()
        .find(|part| part.kind == ContextKind::Reasoning)
        .expect("a reasoning row");
    // The text is openable and carries the tokens it cost, without being
    // counted twice in the request's make-up.
    assert_eq!(part.tokens, 120);
    assert_eq!(part.items.len(), 1);
    assert_eq!(part.items[0].text, "先想一下");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn thinking_lands_in_its_own_slot_of_the_round_that_produced_it() {
    use super::{ContextKind, add_reasoning};

    let slot = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Reasoning)
        .expect("the category exists");
    let mut rounds = vec![
        vec![0usize; ContextKind::ALL.len()],
        vec![0usize; ContextKind::ALL.len()],
    ];
    add_reasoning(&mut rounds, &[120, 40]);
    assert_eq!(rounds[0][slot], 120);
    assert_eq!(rounds[1][slot], 40);
    // A provider that sent no thinking adds nothing at all.
    let mut silent = vec![vec![0usize; ContextKind::ALL.len()]];
    add_reasoning(&mut silent, &[]);
    assert!(silent[0].iter().all(|tokens| *tokens == 0));
    // Rounds without a figure are left alone rather than guessed at.
    let mut short = vec![vec![0usize; ContextKind::ALL.len()]];
    add_reasoning(&mut short, &[120, 40]);
    assert_eq!(short[0][slot], 120);
}

#[test]
fn a_request_row_keeps_the_thinking_of_each_round() {
    use crate::chats::{StoredRequest, StoredRequestItem};

    let mut answer = ChatMessage::assistant("a");
    answer.round_tokens = vec![(100, 10, 0), (200, 20, 0)];
    answer.rounds = 2;
    answer.round_reasoning = vec![120, 40];
    // Only the thinking is recorded here, so every other slot stays zero.
    answer.composition = vec![0; ContextKind::ALL.len()];
    let stored = StoredRequest {
        turn: 1,
        at_ms: 0,
        model: "deepseek-reasoner".to_string(),
        tokens: 0,
        context: 0,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: String::new(),
        reasoning_tokens: 0,
        items: vec![StoredRequestItem {
            kind: ContextKind::User.key().to_string(),
            label: "q".to_string(),
            text: "q".to_string(),
            tokens: 100,
        }],
    };
    let stored = std::collections::HashMap::from([(1u64, stored)]);
    let (rows, ..) = request_rows(&[answer], &stored);
    let row = &rows[0];
    assert_eq!(row.round_reasoning, vec![120, 40]);
    let slot = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Reasoning)
        .expect("the category exists");
    // Each round carries its own figure, and the request's total is their sum.
    assert_eq!(row.round_composition[0][slot], 120);
    assert_eq!(row.round_composition[1][slot], 40);
    assert_eq!(row.composition[slot], 160);
}

#[test]
fn actions_are_read_from_the_tool_rows_of_one_request() {
    use super::ActionKind;
    use super::request_actions;

    let mut write = ChatMessage::tool("write · note.txt", false);
    write.tool = "write".to_string();
    write.input = r#"{"path":"note.txt","content":"hi"}"#.to_string();
    let mut shell = ChatMessage::tool("shell · npm test", true);
    shell.tool = "shell".to_string();
    shell.input = r#"{"command":"npm test -- --run\n"}"#.to_string();
    let mut plugin = ChatMessage::tool("demo_echo · hi", false);
    plugin.tool = "demo_echo".to_string();
    plugin.input = r#"{"text":"hi"}"#.to_string();

    let messages = vec![
        ChatMessage::user("q1"),
        write,
        shell,
        ChatMessage::assistant("a1"),
        ChatMessage::user("q2"),
        plugin,
        ChatMessage::assistant("a2"),
    ];

    let first = request_actions(&messages, 1);
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].kind, ActionKind::File);
    assert_eq!(first[0].detail, "note.txt");
    assert!(first[0].ok);
    assert_eq!(first[1].kind, ActionKind::Command);
    assert_eq!(first[1].detail, "npm test -- --run");
    assert!(!first[1].ok);

    let second = request_actions(&messages, 2);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].kind, ActionKind::Plugin);
    assert_eq!(second[0].detail, "hi");

    // A request without tool rows has no actions; a turn past the end is
    // empty rather than a panic.
    assert!(request_actions(&messages, 3).is_empty());
    assert!(request_actions(&[ChatMessage::assistant("a")], 1).is_empty());
}

#[test]
fn anchoring_spreads_a_measured_number_over_the_estimate() {
    // The provider reported 1000 tokens; the parts keep their proportions
    // and still add up to exactly that number.
    let scaled = anchor_parts(&[30, 90], 1000);
    assert_eq!(scaled, vec![250, 750]);
    assert_eq!(scaled.iter().sum::<usize>(), 1000);
    // Rounding goes to the largest part, so no token is invented or lost.
    let thirds = anchor_parts(&[1, 1, 1], 100);
    assert_eq!(thirds.iter().sum::<usize>(), 100);
    // Flooring first keeps the total exact even when it is smaller than the
    // number of parts.
    let thinned = anchor_parts(&[1, 1, 1, 1, 1, 1, 1], 3);
    assert_eq!(thinned.iter().sum::<usize>(), 3);
    assert_eq!(thinned.iter().filter(|tokens| **tokens > 0).count(), 3);
    // Nothing measured means nothing to scale.
    assert_eq!(anchor_parts(&[0, 0], 500), vec![0, 0]);
    assert_eq!(anchor_parts(&[10, 20], 0), vec![10, 20]);
}

#[test]
fn the_timeline_stacks_one_bar_per_request_and_flags_its_events() {
    // Oldest first, so the chart reads left to right like the conversation.
    let mut first = ChatMessage::assistant("a1");
    first.context = 1_000;
    first.elapsed_ms = 2_000;
    // The structured layer is what a new save carries; the note stays as a
    // fallback for conversations written before it existed.
    first.compaction_layer = "summary".to_string();
    first.compaction_saved = 900;
    let mut second = ChatMessage::assistant("a2");
    second.context = 6_000;
    second.elapsed_ms = 9_000;
    let mut third = ChatMessage::assistant("a3");
    third.context = 7_000;
    third.elapsed_ms = 1_000;
    // A compaction note lands before the answer it belongs to.
    let messages = vec![
        ChatMessage::note(
            "上下文接近窗口上限：最早的 2 条消息已压成摘要（约省 900 tokens）。",
            false,
        ),
        first,
        ChatMessage::user("q2"),
        second,
        ChatMessage::user("q3"),
        ChatMessage::note(
            "上下文接近窗口上限：已精简 1 段过长的工具结果（约省 200 tokens）。",
            false,
        ),
        third,
    ];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    let noted = super::noted_compactions(&messages);
    let points = timeline_points(&rows, &noted);
    assert_eq!(
        points.iter().map(|point| point.turn).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        points.iter().map(|point| point.context).collect::<Vec<_>>(),
        vec![1_000, 6_000, 7_000]
    );
    // The second request owned the biggest jump and the slowest answer; the
    // first carried a compaction, the third only the legacy note, and the
    // second must not inherit either.
    assert!(points[0].flags.contains(&TimelineFlag::Compacted));
    assert!(!points[1].flags.contains(&TimelineFlag::Compacted));
    assert!(points[2].flags.contains(&TimelineFlag::Compacted));
    assert!(points[1].flags.contains(&TimelineFlag::BiggestJump));
    assert!(points[1].flags.contains(&TimelineFlag::Longest));
    assert!(!points[2].flags.contains(&TimelineFlag::BiggestJump));
    // No composition was recorded, so each bar is one neutral block.
    assert!(points.iter().all(|point| point.flat));
}

#[test]
fn the_timeline_flags_a_model_switch_and_skill_text() {
    let mut first = ChatMessage::assistant("a1");
    first.model = "deepseek-chat".to_string();
    first.composition = vec![100, 50, 20, 0, 40, 10, 5, 8];
    let mut second = ChatMessage::assistant("a2");
    second.model = "deepseek-reasoner".to_string();
    second.composition = vec![100, 50, 20, 0, 40, 10, 5, 9];
    let messages = vec![first, second];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    let points = timeline_points(&rows, &std::collections::HashSet::new());
    assert!(points[0].flags.contains(&TimelineFlag::Skills));
    assert!(!points[0].flags.contains(&TimelineFlag::ModelChange));
    // The same skills were already there, so only the model changed.
    assert!(points[1].flags.contains(&TimelineFlag::Skills));
    assert!(points[1].flags.contains(&TimelineFlag::ModelChange));
    // Segments are shares of the bar, largest category first in ALL order.
    let total: u32 = messages[0].composition.iter().sum();
    let expected = messages[0].composition[0] as f32 / total as f32;
    assert!((points[0].segments[0].1 - expected).abs() < 1e-6);
    assert!(!points[0].flat);
}

#[test]
fn the_request_table_lists_answers_newest_first() {
    use super::{REQUEST_ROWS, request_rows};
    let mut messages = Vec::new();
    let answers = REQUEST_ROWS + 6;
    for index in 1..=answers as u64 {
        messages.push(ChatMessage::user("q"));
        let mut answer = ChatMessage::assistant("a");
        answer.tokens = index * 10;
        answer.context = index * 5;
        answer.elapsed_ms = index;
        messages.push(answer);
    }
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    // The newest answer is first, and every row is kept: the panel decides
    // how many to draw (and the timeline follows the same window).
    assert_eq!(rows.len(), answers);
    assert_eq!(rows[0].turn, answers as u64);
    assert_eq!(rows[0].tokens, answers as u64 * 10);
    assert_eq!(rows[0].context, answers as u64 * 5);
    assert_eq!(rows[REQUEST_ROWS - 1].turn, 7);
    assert_eq!(rows[answers - 1].turn, 1);
    // A question on its own has nothing to bill yet.
    assert!(
        request_rows(&[ChatMessage::user("q")], &std::collections::HashMap::new())
            .0
            .is_empty()
    );
}

#[test]
fn a_request_without_round_counts_still_has_a_bill() {
    use super::request_rows;
    // A question sent with the tools switched off reports its total only.
    // The panel must keep that bill and say the per-round split was never
    // reported, instead of making the request look free.
    let mut answer = ChatMessage::assistant("a");
    answer.tokens = 700;
    answer.context = 650;
    let (rows, sums) = request_rows(
        &[ChatMessage::user("q"), answer],
        &std::collections::HashMap::new(),
    );
    assert_eq!(rows[0].tokens, 700);
    assert_eq!(rows[0].prompt_tokens, 0);
    assert_eq!(rows[0].answer_tokens, 0);
    assert_eq!(rows[0].context, 650);
    assert_eq!(sums.prompt, 0);
    assert_eq!(sums.answer, 0);
    // 提示合计 + 回答合计 + 未回报 = 计费, so the chips can still add up.
    assert_eq!(sums.unreported, 700);
    assert_eq!(sums.prompt + sums.answer + sums.unreported, 700);
}

#[test]
fn a_request_keeps_the_totals_its_answer_recorded() {
    use super::{recorded_totals, request_rows};
    use crate::app::context_view::ContextKind;
    let mut answer = ChatMessage::assistant("a");
    answer.context = 900;
    answer.composition = vec![100, 200, 300, 0, 0, 250, 50];
    let (rows, ..) = request_rows(&[answer], &std::collections::HashMap::new());
    let totals = recorded_totals(&rows[0]).expect("a legacy save still has a shape");
    // Same order as `ContextKind::ALL`, so the ring and the legend line up.
    // The old save has no output slot, so it is padded with a zero.
    assert_eq!(totals, vec![100, 200, 300, 0, 0, 250, 50, 0, 0]);
    assert_eq!(totals.len(), ContextKind::ALL.len());
    // A current save carries the output slot itself.
    let mut current = ChatMessage::assistant("a");
    current.context = 900;
    current.composition = vec![100, 200, 300, 0, 0, 0, 50, 25];
    let (rows, ..) = request_rows(&[current], &std::collections::HashMap::new());
    assert_eq!(
        recorded_totals(&rows[0]).expect("nine figures were recorded"),
        vec![100, 200, 300, 0, 0, 0, 50, 25, 0]
    );
    // An answer stored before the figures were kept has no shape to show.
    let mut old = ChatMessage::assistant("a");
    old.context = 900;
    assert!(
        recorded_totals(&request_rows(&[old], &std::collections::HashMap::new()).0[0]).is_none()
    );
    // A run of the wrong length is not a shape either.
    let mut broken = ChatMessage::assistant("a");
    broken.composition = vec![1, 2, 3];
    assert!(
        recorded_totals(&request_rows(&[broken], &std::collections::HashMap::new()).0[0]).is_none()
    );
}

#[test]
fn an_old_save_gets_its_answers_counted_back_in() {
    use super::{ContextKind, RequestItem, request_rows, round_estimates};
    use crate::app::context_view::ContextKind as Kind;
    // A save from before the answers were counted: the assistant slot holds
    // only the earlier turns replayed into the prompt.
    let mut answer = ChatMessage::assistant("a");
    answer.context = 900;
    answer.rounds = 2;
    answer.round_tokens = vec![(200, 60, 1), (250, 40, 0)];
    answer.composition = vec![300, 0, 30, 0, 0, 0, 40];
    let (rows, ..) = request_rows(&[answer], &std::collections::HashMap::new());
    let slot = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Assistant)
        .unwrap_or(0);
    // The 100 tokens the request itself wrote are back in the slot, so the
    // classified total no longer falls short of the bill by the answers.
    assert_eq!(rows[0].composition[slot], 100);
    assert_eq!(rows[0].composition.iter().sum::<u32>(), 470);
    assert_eq!(rows[0].composition.len(), ContextKind::ALL.len());
    // A current save counts them in the output slot, so nothing is added
    // twice there either.
    let mut current = ChatMessage::assistant("a");
    current.rounds = 2;
    current.round_tokens = vec![(200, 60, 1), (250, 40, 0)];
    current.composition = vec![300, 0, 30, 0, 0, 0, 40, 100];
    let (rows, ..) = request_rows(&[current], &std::collections::HashMap::new());
    let output = ContextKind::ALL
        .iter()
        .position(|kind| *kind == ContextKind::Output)
        .unwrap_or(0);
    assert_eq!(rows[0].composition[slot], 0);
    assert_eq!(rows[0].composition[output], 100);
    assert_eq!(rows[0].composition.iter().sum::<u32>(), 470);
    // The helper itself is what the fallback uses, so cover it directly too.
    let items = vec![RequestItem {
        kind: Kind::System,
        label: "system".to_string(),
        text: "system".to_string(),
        tokens: 100,
    }];
    let rounds = round_estimates(&items, &[(50, 25, 0)], 1);
    // What the round wrote is counted where the output lives, not beside
    // the replayed history.
    assert_eq!(rounds[0][slot], 0);
    assert_eq!(rounds[0][output], 25);
}

#[test]
fn each_request_keeps_the_tool_calls_it_made() {
    use super::request_rows;
    let mut first = ChatMessage::assistant("a");
    first.tokens = 100;
    let mut read = ChatMessage::tool("read · a.txt — 已读取 2 行", false);
    read.output = "1\talpha\n2\tbeta\n".to_string();
    let mut write = ChatMessage::tool("write · b.txt — 已写入", false);
    write.output = "已写入 1 行".to_string();
    let mut second = ChatMessage::assistant("b");
    second.tokens = 200;
    second.rounds = 4;
    second.round_tokens = vec![(100, 10, 1), (120, 12, 2)];
    let messages = vec![
        ChatMessage::user("q1"),
        read,
        write,
        first,
        ChatMessage::user("q2"),
        ChatMessage::tool("list · . — 3 项", false),
        second,
    ];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    // The two calls before the first answer belong to it, the one after the
    // second question belongs to the second answer.
    assert_eq!(rows[1].turn, 1);
    assert_eq!(rows[1].tools, 2);
    assert_eq!(rows[0].turn, 2);
    assert_eq!(rows[0].tools, 1);
    // How many round trips it took, which explains the bill.
    assert_eq!(rows[0].rounds, 4);
    assert_eq!(rows[1].rounds, 0);
    // And each round's own counts, so the repeated prompts can be shown.
    assert_eq!(rows[0].round_tokens, vec![(100, 10, 1), (120, 12, 2)]);
    assert!(rows[1].round_tokens.is_empty());
    // A question that used no tools has nothing to report.
    let (plain, ..) = request_rows(
        &[ChatMessage::user("q"), ChatMessage::assistant("a")],
        &std::collections::HashMap::new(),
    );
    assert_eq!(plain[0].tools, 0);
}

#[test]
fn a_round_view_keeps_only_the_tool_results_it_could_see() {
    use super::{ContextItem, PartItems};
    let item = |label: &str, tokens: usize| ContextItem {
        label: label.to_string(),
        text: format!("body of {label}"),
        tokens,
    };
    let items = vec![
        item("r1", 10),
        item("web_search · q", 5),
        item("r2", 20),
        item("r3", 30),
    ];
    // The first round ran before any local result existed; the server result
    // is already there because it ran inside that round.
    let first = PartItems::ToolResults {
        items: &items,
        upto: 0,
    };
    assert_eq!(first.count_and_estimate(), (1, 5));
    assert_eq!(
        first
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["web_search · q"]
    );
    // The second round already carried the first local result.
    let second = PartItems::ToolResults {
        items: &items,
        upto: 1,
    };
    assert_eq!(
        second
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["r1", "web_search · q"]
    );
    assert_eq!(second.count_and_estimate(), (2, 15));
    // Asking for more results than exist keeps them all.
    assert_eq!(
        PartItems::ToolResults {
            items: &items,
            upto: 9
        }
        .count_and_estimate(),
        (4, 65)
    );
    // A category that does not narrow keeps the stored order.
    assert_eq!(PartItems::All(&items).count_and_estimate(), (4, 65));
}

#[test]
fn the_tool_rows_of_the_run_become_part_of_its_request() {
    use super::posted_tool_items;
    let mut read = ChatMessage::tool("read · a.txt — 已读取 2 行", false);
    read.input = "{\"path\":\"a.txt\"}".to_string();
    read.output = "1\talpha\n".to_string();
    let mut empty = ChatMessage::tool("list · . — 空目录", false);
    empty.output = "   ".to_string();
    let messages = vec![
        ChatMessage::user("上一问"),
        ChatMessage::tool("read · old.txt — 已读取", false),
        ChatMessage::assistant("上一答"),
        ChatMessage::user("这一问"),
        ChatMessage::note("上下文超出上限", false),
        read,
        empty,
    ];
    let items = posted_tool_items(&messages);
    // Only this run's tool rows; the one with nothing to send is skipped.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "read · a.txt — 已读取 2 行");
    assert_eq!(items[0].kind, ContextKind::ToolResults);
    // What the call cost is both halves: the arguments the model wrote and
    // what came back, so neither side is missing from the total.
    assert_eq!(
        items[0].text,
        "参数：{\"path\":\"a.txt\"}\n\n返回：1\talpha"
    );
    assert_eq!(
        items[0].tokens,
        crate::app::util::estimate_tokens(&items[0].text)
    );
    assert!(items[0].tokens > crate::app::util::estimate_tokens("1\talpha"));
    // Nothing after the question: nothing to add.
    assert!(posted_tool_items(&[ChatMessage::user("q")]).is_empty());
}

#[test]
fn the_closing_answer_of_a_turn_comes_from_the_transcript() {
    use super::closing_answer;
    let messages = vec![
        ChatMessage::user("q1"),
        ChatMessage::assistant("第一答"),
        ChatMessage::tool("read · a.txt", false),
        ChatMessage::user("q2"),
        ChatMessage::assistant("  第二答  "),
        ChatMessage::user("q3"),
        ChatMessage::assistant("   "),
    ];
    assert_eq!(closing_answer(&messages, 1).as_deref(), Some("第一答"));
    assert_eq!(closing_answer(&messages, 2).as_deref(), Some("第二答"));
    // A stopped answer has nothing to show, and so does a turn that has
    // not been answered yet.
    assert_eq!(closing_answer(&messages, 3), None);
    assert_eq!(closing_answer(&messages, 4), None);
    assert_eq!(closing_answer(&messages, 0), None);
}

#[test]
fn the_text_a_request_wrote_is_kept_with_that_request() {
    use super::output_item;
    let item = output_item("  第一行\n第二行 \n").expect("an answer is kept");
    assert_eq!(item.kind, ContextKind::Output);
    assert_eq!(item.label, "最终输出");
    assert_eq!(item.text, "第一行\n第二行");
    assert_eq!(
        item.tokens,
        crate::app::util::estimate_tokens("第一行\n第二行")
    );
    // A stopped or empty answer has nothing to keep.
    assert!(output_item("   ").is_none());
}

#[test]
fn the_first_question_of_a_conversation_gets_its_tool_schemas_later() {
    use super::{add_missing_tool_schemas, composition_of, request_items};
    let schemas = vec![("read".to_string(), "abcd".repeat(3))];
    let mut items = request_items(&[], &[], &[], &[], "q");
    // Recorded before the engine reported anything: no tool definition yet.
    assert_eq!(composition_of(&items)[1], 0);
    add_missing_tool_schemas(&mut items, &schemas);
    assert_eq!(composition_of(&items)[1], 3);
    assert_eq!(items.last().unwrap().label, "read");
    assert_eq!(items.last().unwrap().tokens, 3);
    // A request that already recorded its schemas is left alone.
    let before = composition_of(&items);
    add_missing_tool_schemas(&mut items, &schemas);
    assert_eq!(composition_of(&items), before);
    // No schemas reported (a chat with tools off) adds nothing.
    let mut bare = request_items(&[], &[], &[], &[], "q");
    add_missing_tool_schemas(&mut bare, &[]);
    assert_eq!(composition_of(&bare)[1], 0);
}

#[test]
fn a_request_records_every_piece_under_its_category() {
    use super::{composition_of, parts_of, request_items};
    use crate::ai::{Speaker, Turn};
    use crate::app::context_view::{PromptBlock, PromptKind};
    let blocks = vec![
        PromptBlock {
            kind: PromptKind::System,
            label: "base",
            sep: "",
            text: "abcd".repeat(2),
        },
        PromptBlock {
            kind: PromptKind::Injected,
            label: "notes",
            sep: "\n\n",
            text: "efgh".repeat(3),
        },
        PromptBlock {
            kind: PromptKind::Injected,
            label: "done",
            sep: "\n\n",
            text: "ijkl".repeat(4),
        },
    ];
    let schemas = vec![("read".to_string(), "mnop".repeat(5))];
    let prompts = vec![("demo".to_string(), "qrst".repeat(6))];
    let history = vec![Turn {
        speaker: Speaker::Assistant,
        text: "yzab".repeat(8),
        server_tool_calls: Vec::new(),
    }];
    let items = request_items(&blocks, &schemas, &prompts, &history, &"uvwx".repeat(7));
    // Four ASCII characters make one token, so the counts are the lengths
    // over four; the order is the legend's order: system, tools, user,
    // injected, skills, assistant, tool results, output. The two injected
    // blocks and the plugin prompt land in the same category (3 + 4 + 6),
    // and nothing is counted as a tool result here: the recap block is
    // injected content.
    assert_eq!(composition_of(&items), vec![2, 5, 7, 13, 0, 8, 0, 0, 0]);
    let parts = parts_of(&items);
    assert_eq!(parts.len(), 9);
    assert_eq!(parts[2].tokens, 7);
    // The question is recorded as a piece of its own, not merged away.
    assert_eq!(parts[2].items.len(), 1);
    assert_eq!(parts[2].items[0].tokens, 7);
    assert_eq!(parts[5].items.len(), 1);
    assert!(parts[4].items.is_empty());
    // Nothing to send still gives a full row of zeroes, never a short list.
    assert_eq!(
        composition_of(&request_items(&[], &[], &[], &[], "")),
        vec![0; 9]
    );
    // Skills land in their own category rather than with injected content.
    let skill = request_items(
        &[PromptBlock {
            kind: PromptKind::Skills,
            label: "启用技能",
            sep: "\n\n",
            text: "efgh".repeat(2),
        }],
        &[],
        &[],
        &[],
        "",
    );
    assert_eq!(composition_of(&skill)[4], 2);
    assert_eq!(composition_of(&skill)[3], 0);
}

#[test]
fn a_request_adds_every_rounds_pieces_together() {
    use super::{ContextKind, RequestItem, composition_counts, round_estimates, summed_rounds};
    let item = |kind: ContextKind, label: &str, tokens: usize| RequestItem {
        kind,
        label: label.to_string(),
        text: label.to_string(),
        tokens,
    };
    let items = vec![
        item(ContextKind::System, "system", 100),
        item(ContextKind::User, "question", 10),
        item(ContextKind::ToolResults, "r1", 10),
        item(ContextKind::ToolResults, "r2", 20),
    ];
    let rounds = round_estimates(&items, &[(200, 5, 1), (250, 6, 1), (300, 7, 0)], 3);
    assert_eq!(rounds.len(), 3);
    // The first round ran before any tool result existed. What it wrote is
    // in the output slot: billed, but never replayed into a prompt.
    assert_eq!(rounds[0], vec![100, 0, 10, 0, 0, 0, 0, 5, 0]);
    // Each later round replays the results the earlier ones produced.
    assert_eq!(rounds[1], vec![100, 0, 10, 0, 0, 0, 10, 6, 0]);
    assert_eq!(rounds[2], vec![100, 0, 10, 0, 0, 0, 30, 7, 0]);
    // The request's own figures are the element-wise sum, not one copy.
    let totals = summed_rounds(&rounds);
    assert_eq!(totals, vec![300, 0, 30, 0, 0, 0, 40, 18, 0]);
    assert_eq!(
        composition_counts(&totals),
        vec![300, 0, 30, 0, 0, 0, 40, 18, 0]
    );
    // The categories now carry every round's answer as well: the tokens the old
    // version dropped were exactly the completions.
    let answers: usize = 5 + 6 + 7;
    let prompts_only = 3 * 100 + 3 * 10 + 40;
    assert_eq!(totals.iter().sum::<usize>(), prompts_only + answers);
    assert_eq!(totals[ContextKind::Assistant as usize], 0);
    assert_eq!(totals[ContextKind::Output as usize], answers);
    // With no per-round counts the whole request stays one entry, so an old
    // answer is never split by a guess.
    let single = round_estimates(&items, &[], 3);
    assert_eq!(single.len(), 1);
    assert_eq!(single[0], vec![100, 0, 10, 0, 0, 0, 30, 0, 0]);
    // A round the endpoint did not count still sent the whole request.
    let tail = round_estimates(&items, &[(200, 5, 1)], 3);
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0], vec![100, 0, 10, 0, 0, 0, 0, 5, 0]);
    assert_eq!(tail[2], vec![100, 0, 10, 0, 0, 0, 30, 0, 0]);
}

#[test]
fn a_round_can_make_two_calls_and_their_results_arrive_together() {
    use super::{ContextKind, RequestItem, round_estimates};
    let item = |kind: ContextKind, label: &str, tokens: usize| RequestItem {
        kind,
        label: label.to_string(),
        text: label.to_string(),
        tokens,
    };
    let mut items = vec![
        item(ContextKind::System, "system", 100),
        item(ContextKind::User, "question", 10),
    ];
    // Seven calls over four tool rounds: two, two, two, then one.
    for result in 1..=7 {
        items.push(item(
            ContextKind::ToolResults,
            &format!("result {result}"),
            1,
        ));
    }
    let rounds = round_estimates(
        &items,
        &[
            (100, 5, 2),
            (110, 6, 2),
            (120, 7, 2),
            (130, 8, 1),
            (140, 9, 0),
        ],
        5,
    );
    assert_eq!(rounds.len(), 5);
    let tool_results = |round: &[usize]| round[ContextKind::ToolResults as usize];
    // Round one starts before any result exists; the two calls it makes
    // return together, so round two sees both — not one per round.
    assert_eq!(tool_results(&rounds[0]), 0);
    assert_eq!(tool_results(&rounds[1]), 2);
    assert_eq!(tool_results(&rounds[2]), 4);
    assert_eq!(tool_results(&rounds[3]), 6);
    assert_eq!(tool_results(&rounds[4]), 7);
}

#[test]
fn a_request_row_uses_the_sum_of_its_rounds() {
    use super::{ContextKind, request_rows};
    use crate::chats::{StoredRequest, StoredRequestItem};
    let stored = StoredRequest {
        turn: 1,
        at_ms: 0,
        model: "m".to_string(),
        tokens: 900,
        context: 500,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: String::new(),
        reasoning_tokens: 0,
        items: vec![
            StoredRequestItem {
                kind: "system".to_string(),
                label: "system".to_string(),
                text: "system".to_string(),
                tokens: 100,
            },
            StoredRequestItem {
                kind: "user".to_string(),
                label: "question".to_string(),
                text: "question".to_string(),
                tokens: 10,
            },
            StoredRequestItem {
                kind: "toolResults".to_string(),
                label: "r1".to_string(),
                text: "r1".to_string(),
                tokens: 10,
            },
            StoredRequestItem {
                kind: "toolResults".to_string(),
                label: "r2".to_string(),
                text: "r2".to_string(),
                tokens: 20,
            },
        ],
    };
    let mut details = std::collections::HashMap::new();
    details.insert(1, stored);
    let mut answer = ChatMessage::assistant("a");
    answer.model = "m".to_string();
    answer.tokens = 900;
    answer.context = 500;
    answer.rounds = 3;
    answer.round_tokens = vec![(200, 5, 1), (250, 6, 1), (300, 7, 0)];
    // What an older transcript recorded: the pieces counted once.
    answer.composition = vec![100, 0, 10, 0, 0, 0, 30];
    let (rows, ..) = request_rows(&[answer], &details);
    // The row now holds the sum of the three rounds: tool traffic included,
    // and each round's own answer in the output slot, so the categories
    // add up to the bill instead of stopping at the prompts.
    assert_eq!(rows[0].composition, vec![300, 0, 30, 0, 0, 0, 40, 18, 0]);
    // The list shows both halves of the bill: what was sent each round and
    // what came back, which is what makes 计费 = Σ 每轮 + 回答 readable.
    assert_eq!(rows[0].prompt_tokens, 750);
    assert_eq!(rows[0].answer_tokens, 18);
    assert_eq!(rows[0].round_composition.len(), 3);
    assert_eq!(
        rows[0].round_composition[2][ContextKind::ToolResults as usize],
        30
    );
    // The categories carry each round's answer too, which is the 18 the older
    // version of this sum dropped.
    assert_eq!(rows[0].composition.iter().sum::<u32>(), 388);
    assert_eq!(rows[0].composition[ContextKind::Assistant as usize], 0);
    assert_eq!(rows[0].composition[ContextKind::Output as usize], 18);
}

#[test]
fn the_totals_use_the_same_closed_split_as_the_request_card() {
    use super::{ContextKind, request_rows, timeline_points};
    use crate::chats::{StoredRequest, StoredRequestItem};
    // A server-side search ran inside the round, but only its summary is
    // kept locally. The request card hands the missing full pages to
    // 工具结果; the totals and the timeline must do the same instead of
    // keeping the raw 10-token estimate and showing 1%.
    let stored = StoredRequest {
        turn: 1,
        at_ms: 0,
        model: "m".to_string(),
        tokens: 1_020,
        context: 1_020,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 1_000,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 1,
        web_fetches: 0,
        reasoning: String::new(),
        reasoning_tokens: 0,
        items: vec![
            StoredRequestItem {
                kind: "system".to_string(),
                label: "system".to_string(),
                text: "system".to_string(),
                tokens: 100,
            },
            StoredRequestItem {
                kind: "user".to_string(),
                label: "question".to_string(),
                text: "question".to_string(),
                tokens: 10,
            },
            StoredRequestItem {
                kind: "toolResults".to_string(),
                label: "web_search".to_string(),
                text: "summary only".to_string(),
                tokens: 10,
            },
        ],
    };
    let details = std::collections::HashMap::from([(1u64, stored)]);
    let mut search = ChatMessage::tool("web_search · ...", false);
    search.call = "round-1-1".to_string();
    let mut answer = ChatMessage::assistant("a");
    answer.tokens = 1_020;
    answer.context = 1_020;
    answer.rounds = 1;
    answer.round_tokens = vec![(1_000, 20, 0)];
    answer.composition = vec![100, 0, 10, 0, 0, 0, 10, 20, 0];
    let messages = vec![ChatMessage::user("q"), search, answer];
    let (rows, sums) = request_rows(&messages, &details);
    let tool = ContextKind::ToolResults as usize;
    // 120 estimated prompt tokens become the endpoint's 1 000: the 880 the
    // estimate could not see are the search pages.
    assert_eq!(rows[0].closed_composition[tool], 890);
    assert_eq!(rows[0].closed_composition.iter().sum::<usize>(), 1_020);
    // The whole-conversation table reads the row, not a second estimate.
    assert_eq!(sums.composition, rows[0].closed_composition);
    // The raw estimate stays available for the 「分类估算」 chip.
    assert_eq!(sums.estimate, 140);
    // The timeline draws the closed share too, so its tool-result segment
    // matches the table instead of falling back to 10/140.
    let points = timeline_points(&rows, &std::collections::HashSet::new());
    assert!(!points[0].flat);
    assert!((points[0].segments[2].1 - 890.0 / 1_020.0).abs() < 1e-6);
}

#[test]
fn without_a_server_call_the_gap_goes_to_injected() {
    use super::{ContextKind, request_rows};
    use crate::chats::{StoredRequest, StoredRequestItem};
    let stored = StoredRequest {
        turn: 1,
        at_ms: 0,
        model: "m".to_string(),
        tokens: 1_020,
        context: 1_020,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 1_000,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: String::new(),
        reasoning_tokens: 0,
        items: vec![StoredRequestItem {
            kind: "system".to_string(),
            label: "system".to_string(),
            text: "system".to_string(),
            tokens: 100,
        }],
    };
    let details = std::collections::HashMap::from([(1u64, stored)]);
    let mut answer = ChatMessage::assistant("a");
    answer.tokens = 1_020;
    answer.context = 1_020;
    answer.rounds = 1;
    answer.round_tokens = vec![(1_000, 20, 0)];
    answer.composition = vec![100, 0, 0, 0, 0, 0, 0, 20, 0];
    let (rows, sums) = request_rows(&[answer], &details);
    let injected = ContextKind::Injected as usize;
    // No web tool ran, so the engine's unrecorded replay is the gap: the
    // 100 estimated prompt tokens become the endpoint's 1 000.
    assert_eq!(rows[0].closed_composition[injected], 900);
    assert_eq!(
        rows[0].closed_composition[ContextKind::ToolResults as usize],
        0
    );
    assert_eq!(sums.composition, rows[0].closed_composition);
    assert_eq!(sums.composition.iter().sum::<usize>(), 1_020);
}

#[test]
fn the_tool_schema_fingerprint_follows_the_content() {
    use super::schemas_fingerprint;
    let one = vec![("read".to_string(), "{}".to_string())];
    let same = vec![("read".to_string(), "{}".to_string())];
    // Same number of tools, different names or schemas: the panel must notice.
    let renamed = vec![("grep".to_string(), "{}".to_string())];
    let changed = vec![("read".to_string(), "{\"a\":1}".to_string())];
    assert_eq!(schemas_fingerprint(&one), schemas_fingerprint(&same));
    assert_ne!(schemas_fingerprint(&one), schemas_fingerprint(&renamed));
    assert_ne!(schemas_fingerprint(&one), schemas_fingerprint(&changed));
    assert_eq!(schemas_fingerprint(&[]), 0xcbf2_9ce4_8422_2325);
}

#[test]
fn the_window_bar_counts_the_prompt_not_the_answer() {
    use super::{ContextKind, window_parts};
    let mut sized = vec![0usize; ContextKind::ALL.len()];
    sized[ContextKind::User as usize] = 600;
    sized[ContextKind::Output as usize] = 400;
    // The request billed 1000, but the prompt that filled the window was 600.
    let parts = window_parts(&sized, 600);
    assert_eq!(parts[ContextKind::Output as usize], 0);
    assert_eq!(parts[ContextKind::User as usize], 600);
    assert_eq!(parts.iter().sum::<usize>(), 600);
    // A save with nothing to scale leaves the answer out too.
    assert_eq!(window_parts(&sized, 0)[ContextKind::Output as usize], 0);
}

#[test]
fn a_server_tool_round_keeps_its_calls_and_one_round() {
    use super::{request_rows, server_round};
    assert_eq!(server_round("round-1-2"), Some(1));
    assert_eq!(server_round("call_00_abc"), None);
    assert_eq!(server_round("round-x-1"), None);
    let mut answer = ChatMessage::assistant("done");
    answer.rounds = 1;
    answer.round_tokens = vec![(1000, 50, 0)];
    answer.tokens = 1050;
    let mut first = ChatMessage::tool("web_search · q", false);
    first.call = "round-1-1".to_string();
    let mut second = ChatMessage::tool("read_url · u", false);
    second.call = "round-1-2".to_string();
    let messages = vec![
        ChatMessage::user("search"),
        ChatMessage::thinking("plan", 1),
        first,
        second,
        answer,
    ];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    // One round trip; the two server calls happened inside it.
    assert_eq!(rows[0].rounds, 1);
    assert_eq!(rows[0].round_server_calls, vec![2]);
    assert_eq!(rows[0].tools, 2);
    // The round's own figures still add up to the request's bill.
    assert_eq!(
        rows[0].prompt_tokens + rows[0].answer_tokens,
        rows[0].tokens
    );
}

#[test]
fn the_replay_gap_joins_the_injected_row() {
    use super::{ContextKind, close_request_parts};
    let mut raw = vec![0usize; ContextKind::ALL.len()];
    raw[ContextKind::User as usize] = 100;
    raw[ContextKind::Injected as usize] = 200;
    raw[ContextKind::Output as usize] = 40;
    raw[ContextKind::Reasoning as usize] = 10;
    // 1000 prompt measured, 40 completion with 10 of it thinking.
    let (parts, gap) = close_request_parts(&raw, 1000, 40, 10, 0, ContextKind::Injected);
    assert_eq!(gap, 700);
    assert_eq!(parts[ContextKind::Injected as usize], 900);
    assert_eq!(parts[ContextKind::Output as usize], 30);
    assert_eq!(parts[ContextKind::Reasoning as usize], 10);
    // The categories add up to the request: prompt + answer.
    assert_eq!(parts.iter().sum::<usize>(), 1040);
    // An estimate that already covers the prompt is scaled, not padded.
    let (parts, gap) = close_request_parts(&raw, 150, 40, 10, 0, ContextKind::Injected);
    assert_eq!(gap, 0);
    assert_eq!(parts[ContextKind::Injected as usize], 100);
    assert_eq!(parts[ContextKind::User as usize], 50);
}

#[test]
fn every_request_adds_up_to_the_conversation_bill() {
    use super::request_rows;
    let mut first = ChatMessage::assistant("a");
    first.tokens = 100;
    let mut second = ChatMessage::assistant("b");
    second.tokens = 250;
    let messages = vec![
        ChatMessage::user("q1"),
        first,
        ChatMessage::user("q2"),
        second,
    ];
    let (rows, ..) = request_rows(&messages, &std::collections::HashMap::new());
    let bills: u64 = rows.iter().map(|row| row.tokens).sum();
    // The conversation's own total is the same sum, one row per answer.
    assert_eq!(bills, 350);
}

#[test]
fn a_category_spreads_over_its_items_exactly() {
    use super::spread_tokens;
    // The shares follow the entries' own estimates.
    let shares = spread_tokens(&[3, 7], 1000);
    assert_eq!(shares, vec![300, 700]);
    // Rounding never loses or invents a token: the shares add up to the row.
    let shares = spread_tokens(&[3, 7], 10);
    assert_eq!(shares.iter().sum::<u64>(), 10);
    // A category with no text has nothing to spread.
    assert!(spread_tokens(&[], 10).is_empty());
}

#[test]
fn a_server_search_result_counts_in_its_own_round() {
    use super::{ContextKind, RequestItem, round_estimates};
    let items = vec![
        RequestItem {
            kind: ContextKind::User,
            label: "q".to_string(),
            text: "q".to_string(),
            tokens: 10,
        },
        RequestItem {
            kind: ContextKind::ToolResults,
            label: "read_url · https://example.com".to_string(),
            text: "page".to_string(),
            tokens: 500,
        },
        RequestItem {
            kind: ContextKind::ToolResults,
            label: "shell · ls".to_string(),
            text: "local".to_string(),
            tokens: 100,
        },
    ];
    let rounds = round_estimates(&items, &[(1000, 50, 1)], 1);
    // The server page is in this round's prompt; the local result reaches the
    // next one, and there is none.
    assert_eq!(rounds[0][ContextKind::ToolResults as usize], 500);
}

#[test]
fn a_server_gap_lands_in_the_tool_results_row() {
    use super::{ContextKind, close_request_parts};
    let mut raw = vec![0usize; ContextKind::ALL.len()];
    raw[ContextKind::User as usize] = 100;
    raw[ContextKind::Output as usize] = 40;
    let (parts, gap) = close_request_parts(&raw, 1000, 40, 0, 0, ContextKind::ToolResults);
    assert!(gap > 0);
    assert_eq!(
        parts[ContextKind::ToolResults as usize],
        usize::try_from(gap).unwrap_or(usize::MAX)
    );
    assert_eq!(parts[ContextKind::User as usize], 100);
}

#[test]
fn a_round_the_endpoint_did_not_split_does_not_inflate_the_bill() {
    use super::request_rows;
    use crate::chats::{StoredRequest, StoredRequestItem};

    // The endpoint answered in two round trips but only counted the first.
    // The second round's local estimate may shape the split, but its tokens
    // are already inside the bill (`tokens`, and hence `unreported`), so
    // adding them to the prompt target as well used to make the categories
    // add up to more than the endpoint charged.
    let stored = StoredRequest {
        turn: 1,
        at_ms: 0,
        model: "m".to_string(),
        tokens: 900,
        context: 500,
        calls: 0,
        cache_hits: 0,
        prompt_tokens: 0,
        model_ms: 0,
        tool_ms: 0,
        web_searches: 0,
        web_fetches: 0,
        reasoning: String::new(),
        reasoning_tokens: 0,
        items: vec![
            StoredRequestItem {
                kind: "system".to_string(),
                label: "system".to_string(),
                text: "system".to_string(),
                tokens: 100,
            },
            StoredRequestItem {
                kind: "user".to_string(),
                label: "question".to_string(),
                text: "question".to_string(),
                tokens: 10,
            },
        ],
    };
    let details = std::collections::HashMap::from([(1u64, stored)]);
    let mut answer = ChatMessage::assistant("a");
    answer.rounds = 2;
    answer.round_tokens = vec![(400, 10, 0)];
    answer.tokens = 900;
    answer.context = 500;
    let (rows, _) = request_rows(&[answer], &details);
    // 400 reported prompt + 10 reported answer + 490 never split = 900.
    assert_eq!(rows[0].prompt_tokens, 400);
    assert_eq!(rows[0].answer_tokens, 10);
    assert_eq!(rows[0].closed_composition.iter().sum::<usize>(), 900);
}
