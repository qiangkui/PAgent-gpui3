//! Block-level parsing: fences, headings, lists, quotes, tables, details, footnotes and rules.

use super::*;

#[test]
fn keeps_a_fenced_block_together() {
    let blocks = parse("看这个：\n```ts\nconst a = 1\n  if (a) {}\n```\n好了");
    assert_eq!(
        blocks,
        vec![
            Block::Paragraph("看这个：".to_string()),
            Block::Code("ts".to_string(), "const a = 1\n  if (a) {}".to_string()),
            Block::Paragraph("好了".to_string()),
        ]
    );
}

#[test]
fn reads_headings_lists_and_quotes() {
    let blocks = parse("# 标题\n- 一\n* 二\n1. 甲\n2) 乙\n> 引用\n正文");
    assert_eq!(
        blocks,
        vec![
            Block::Heading(1, "标题".to_string()),
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(0, "二".to_string()),
            Block::Numbered(0, "1.".to_string(), "甲".to_string()),
            Block::Numbered(0, "2)".to_string(), "乙".to_string()),
            Block::Quote(vec![Rich {
                block: Block::Paragraph("引用".to_string()),
                spans: vec![span("引用".len(), false, false, false)],
                links: Vec::new(),
            }]),
            Block::Paragraph("正文".to_string()),
        ]
    );
}

#[test]
fn a_decimal_number_is_not_a_list_item() {
    // `3.14` and `1.5` are numbers; only a marker followed by a space (or
    // nothing at all) starts a list.
    assert_eq!(
        parse("3.14 是圆周率\n1.5 倍"),
        vec![Block::Paragraph("3.14 是圆周率 1.5 倍".to_string())]
    );
    assert_eq!(
        parse("2024. 计划\n1)"),
        vec![
            Block::Numbered(0, "2024.".to_string(), "计划".to_string()),
            // `1)` is a list of its own: the delimiter changed, so the count
            // starts over at what was written.
            Block::Numbered(0, "1)".to_string(), String::new()),
        ]
    );
}

#[test]
fn a_heading_keeps_a_sharp_that_is_part_of_the_text() {
    assert_eq!(parse("# C#"), vec![Block::Heading(1, "C#".to_string())]);
    // A closing run is decoration only when a space comes before it.
    assert_eq!(
        parse("## 标题 ##"),
        vec![Block::Heading(2, "标题".to_string())]
    );
    assert_eq!(parse("## ###"), vec![Block::Heading(2, String::new())]);
    assert_eq!(
        parse("### 带#号的标题"),
        vec![Block::Heading(3, "带#号的标题".to_string())]
    );
}

#[test]
fn indent_nests_a_list_item() {
    let blocks = parse("- 一\n  - 一甲\n    - 一甲子\n       - 三层");
    assert_eq!(
        blocks,
        vec![
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(1, "一甲".to_string()),
            Block::Bullet(2, "一甲子".to_string()),
            Block::Bullet(3, "三层".to_string()),
        ]
    );
    // Six levels are drawn; anything deeper stops at the deepest level
    // instead of pushing the text out of the column.
    let blocks = parse(
        "- 0\n  - 1\n    - 2\n      - 3\n        - 4\n          - 5\n            - 6\n              - 太深了",
    );
    let levels: Vec<usize> = blocks
        .iter()
        .map(|block| match block {
            Block::Bullet(level, _) => *level,
            other => panic!("expected a bullet, got {other:?}"),
        })
        .collect();
    assert_eq!(
        levels,
        vec![0, 1, 2, 3, 4, 5, MAX_LIST_LEVEL, MAX_LIST_LEVEL]
    );
    // A deeper indent nests one level, whether it is two, three or four
    // spaces: the item above is the only thing it is deeper than.
    let blocks = parse("- 一\n    - 一甲\n        - 一甲子");
    assert_eq!(
        blocks,
        vec![
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(1, "一甲".to_string()),
            Block::Bullet(2, "一甲子".to_string()),
        ]
    );
    let blocks = parse("1. 一\n   1. 甲\n      1. 子\n2. 二");
    assert_eq!(
        blocks,
        vec![
            Block::Numbered(0, "1.".to_string(), "一".to_string()),
            Block::Numbered(1, "1.".to_string(), "甲".to_string()),
            Block::Numbered(2, "1.".to_string(), "子".to_string()),
            Block::Numbered(0, "2.".to_string(), "二".to_string()),
        ]
    );
    // An item with nothing above it is top level, however it is indented.
    let blocks = parse("   - 一");
    assert_eq!(blocks, vec![Block::Bullet(0, "一".to_string())]);
}

#[test]
fn a_line_of_equals_or_dashes_underlines_the_line_above_it() {
    assert_eq!(
        parse("大标题\n==="),
        vec![Block::Heading(1, "大标题".to_string())]
    );
    assert_eq!(
        parse("小标题\n---"),
        vec![Block::Heading(2, "小标题".to_string())]
    );
    // Only the line touching the underline becomes the heading.
    assert_eq!(
        parse("一段正文\n标题行\n---"),
        vec![
            Block::Paragraph("一段正文".to_string()),
            Block::Heading(2, "标题行".to_string()),
        ]
    );
    // Emphasis inside the underlined line is kept.
    assert_eq!(
        parse("看 `代码`\n==="),
        vec![Block::Heading(1, "看 代码".to_string())]
    );
    // A line of dashes with a blank line above it is still a rule.
    assert_eq!(
        parse("正文\n\n---"),
        vec![Block::Paragraph("正文".to_string()), Block::Rule]
    );
}

#[test]
fn a_rule_line_is_a_block_of_its_own() {
    let blocks = parse("上\n\n---\n\n下");
    assert_eq!(
        blocks,
        vec![
            Block::Paragraph("上".to_string()),
            Block::Rule,
            Block::Paragraph("下".to_string()),
        ]
    );
    // With a blank line before it, dashes separate and never draw.
    assert_eq!(
        parse("标题\n\n- - -\n\n正文"),
        vec![
            Block::Paragraph("标题".to_string()),
            Block::Rule,
            Block::Paragraph("正文".to_string()),
        ]
    );
    // A table's rule is not a section rule.
    assert_eq!(
        parse("| a | b |\n|---|---|\n| 1 | 2 |")[0],
        Block::Table(Table {
            header: vec![Cell::from("a"), Cell::from("b")],
            rows: vec![vec![Cell::from("1"), Cell::from("2")]],
            aligns: vec![Align::Left, Align::Left],
        })
    );
}

#[test]
fn joins_wrapped_paragraph_lines() {
    // A soft break does not show: the text wraps to the column, not to the
    // width the model typed at. Between two Chinese lines it joins with
    // nothing, between two English words with a space.
    assert_eq!(
        parse("第一行\n第二行\n\n另一段"),
        vec![
            Block::Paragraph("第一行第二行".to_string()),
            Block::Paragraph("另一段".to_string()),
        ]
    );
    assert_eq!(
        parse("first line\nsecond line"),
        vec![Block::Paragraph("first line second line".to_string())]
    );
    // A line ending in two spaces (or a backslash) keeps its break.
    assert_eq!(
        parse("第一行  \n第二行\n第三行\\\n第四行"),
        vec![Block::Paragraph("第一行\n第二行第三行\n第四行".to_string())]
    );
    // A space is only dropped next to Chinese, never before its punctuation.
    assert_eq!(
        parse("这是 abc\n然后"),
        vec![Block::Paragraph("这是 abc 然后".to_string())]
    );
}

#[test]
fn joins_the_lines_of_one_quote() {
    assert_eq!(
        parse("> 第一行\n> 第二行\n\n正文"),
        vec![quoted("第一行第二行"), Block::Paragraph("正文".to_string())]
    );
}

#[test]
fn keeps_a_blank_quote_line_inside_the_quote() {
    let blocks = parse("> 一\n>\n> 二\n正文");
    assert_eq!(
        blocks,
        vec![
            Block::Quote(vec![para("一"), para("二")]),
            Block::Paragraph("正文".to_string()),
        ]
    );
}

#[test]
fn a_quote_keeps_the_shape_of_what_it_quotes() {
    // A list inside a quote is a list in the box, not its markers as text.
    let blocks = parse("> 说明：\n> - 一\n> - 二");
    let Block::Quote(inner) = &blocks[0] else {
        panic!("expected a quote");
    };
    assert_eq!(
        inner
            .iter()
            .map(|rich| rich.block.clone())
            .collect::<Vec<_>>(),
        vec![
            Block::Paragraph("说明：".to_string()),
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(0, "二".to_string()),
        ]
    );
    // A heading and a fence too.
    let blocks = parse("> # 标题\n> ```ts\n> let a = 1;\n> ```");
    let Block::Quote(inner) = &blocks[0] else {
        panic!("expected a quote");
    };
    assert_eq!(
        inner
            .iter()
            .map(|rich| rich.block.clone())
            .collect::<Vec<_>>(),
        vec![
            Block::Heading(1, "标题".to_string()),
            Block::Code("ts".to_string(), "let a = 1;".to_string()),
        ]
    );
}

#[test]
fn an_italic_cell_copies_back_with_its_stars() {
    let cell = styled_cell("看 *重点* 与 `码`");
    assert_eq!(cell.text, "看 重点 与 码");
    assert_eq!(cell.markdown(), "看 *重点* 与 `码`");
}

#[test]
fn a_struck_cell_copies_back_with_its_tildes() {
    let cell = styled_cell("旧的 ~~方案~~");
    assert_eq!(cell.text, "旧的 方案");
    assert_eq!(cell.markdown(), "旧的 ~~方案~~");
}

#[test]
fn a_task_item_carries_its_box() {
    assert_eq!(
        parse("- [ ] 还没做\n- [x] 做好了\n- [X] 也一样\n- 普通项"),
        vec![
            Block::Task(0, false, "还没做".to_string()),
            Block::Task(0, true, "做好了".to_string()),
            Block::Task(0, true, "也一样".to_string()),
            Block::Bullet(0, "普通项".to_string()),
        ]
    );
    // Nested items are indented like any other list item.
    assert_eq!(
        parse("- 父\n  - [ ] 子项"),
        vec![
            Block::Bullet(0, "父".to_string()),
            Block::Task(1, false, "子项".to_string()),
        ]
    );
    // A bracket that is not a box is the item's text.
    assert_eq!(
        parse("- [链接] 说明"),
        vec![Block::Bullet(0, "[链接] 说明".to_string())]
    );
}

#[test]
fn an_unclosed_fence_runs_to_the_end() {
    let blocks = parse("```\nlet a = 1");
    assert_eq!(
        blocks,
        vec![Block::Code(String::new(), "let a = 1".to_string())]
    );
}

#[test]
fn plain_text_is_one_paragraph() {
    assert_eq!(
        parse("就是一句话"),
        vec![Block::Paragraph("就是一句话".to_string())]
    );
    assert!(parse("   \n\n ").is_empty());
}

#[test]
fn a_table_is_read_as_header_and_rows() {
    let blocks = parse("| 名称 | 数量 |\n|---|---|\n| 文件 | 12 |\n| `目录` | 3 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("名称", "数量"),
            rows: vec![
                row!("文件", "12"),
                vec![
                    Cell {
                        text: "目录".to_string(),
                        spans: vec![span(6, false, false, true)],
                        links: Vec::new(),
                    },
                    Cell::from("3"),
                ],
            ],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn text_around_a_table_keeps_its_own_blocks() {
    let blocks = parse("看这张表：\n| a | b |\n|---|---|\n| 1 | 2 |\n表就到这里");
    assert_eq!(
        blocks,
        vec![
            Block::Paragraph("看这张表：".to_string()),
            Block::Table(Table {
                header: row!("a", "b"),
                rows: vec![row!("1", "2")],
                aligns: vec![Align::Left, Align::Left],
            }),
            Block::Paragraph("表就到这里".to_string()),
        ]
    );
}

#[test]
fn a_short_row_is_padded_to_the_header() {
    let blocks = parse("| a | b |\n|---|---|\n| 1 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("a", "b"),
            rows: vec![vec![Cell::from("1"), Cell::default()]],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn a_pipe_in_prose_is_text() {
    let blocks = parse("用 A | B 表示或");
    assert_eq!(
        blocks,
        vec![Block::Paragraph("用 A | B 表示或".to_string())]
    );
}

#[test]
fn a_table_rule_without_leading_pipes_is_still_a_rule() {
    let blocks = parse("名称 | 数量\n--- | ---\n文件 | 12");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("名称", "数量"),
            rows: vec![row!("文件", "12")],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn a_table_rule_without_a_trailing_pipe_is_still_a_rule() {
    let blocks = parse("| a | b |\n|---|---\n| 1 | 2 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("a", "b"),
            rows: vec![row!("1", "2")],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn a_rule_with_nothing_above_it_is_dropped() {
    assert_eq!(parse("|---|---|"), Vec::new());
}

#[test]
fn colons_in_the_rule_align_the_columns() {
    let blocks = parse("| 名称 | 数量 | 备注 |\n| :-- | --: | :-: |\n| 文件 | 12 | 一 |");
    let Block::Table(table) = &blocks[0] else {
        panic!("expected a table");
    };
    assert_eq!(table.aligns, vec![Align::Left, Align::Right, Align::Center]);
}

#[test]
fn a_lone_pipe_adds_no_row() {
    let blocks = parse("| a | b |\n|---|---|\n|\n| 1 | 2 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("a", "b"),
            rows: vec![row!("1", "2")],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn an_escaped_pipe_stays_inside_its_cell() {
    let blocks = parse("| a | b |\n|---|---|\n| x \\| y | 2 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: row!("a", "b"),
            rows: vec![row!("x | y", "2")],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn a_column_added_by_a_longer_row_aligns_left() {
    let blocks = parse("| a |\n|---|\n| 1 | 2 |");
    assert_eq!(
        blocks,
        vec![Block::Table(Table {
            header: vec![Cell::from("a"), Cell::default()],
            rows: vec![row!("1", "2")],
            aligns: vec![Align::Left, Align::Left],
        })]
    );
}

#[test]
fn a_table_reads_back_as_pipe_text() {
    let table = Table {
        header: row!("a", "b"),
        rows: vec![row!("1", "2")],
        aligns: vec![Align::Left, Align::Right],
    };
    assert_eq!(table.text(), "a | b\n1 | 2");
}

#[test]
fn a_bold_cell_copies_back_with_its_markers() {
    let table = Table {
        header: row!("a", "b"),
        rows: vec![vec![styled_cell("**总计**"), Cell::from("12")]],
        aligns: vec![Align::Left, Align::Right],
    };
    assert_eq!(
        table.markdown(),
        "| a | b |\n| --- | ---: |\n| **总计** | 12 |"
    );
}

#[test]
fn numbered_items_count_up_from_the_first_one() {
    // Markdown ignores the numbers after the first, so `1.` on every line still
    // reads as 1, 2, 3 instead of repeating the marker.
    assert_eq!(
        parse("1. 一\n1. 二\n1. 三"),
        vec![
            Block::Numbered(0, "1.".to_string(), "一".to_string()),
            Block::Numbered(0, "2.".to_string(), "二".to_string()),
            Block::Numbered(0, "3.".to_string(), "三".to_string()),
        ]
    );
    // The first item keeps the number it was written with.
    assert_eq!(
        parse("3. 一\n1. 二"),
        vec![
            Block::Numbered(0, "3.".to_string(), "一".to_string()),
            Block::Numbered(0, "4.".to_string(), "二".to_string()),
        ]
    );
}

#[test]
fn a_numbered_list_continues_across_its_own_nested_items() {
    assert_eq!(
        parse("1. 一\n   1. 甲\n   1. 乙\n1. 二"),
        vec![
            Block::Numbered(0, "1.".to_string(), "一".to_string()),
            Block::Numbered(1, "1.".to_string(), "甲".to_string()),
            Block::Numbered(1, "2.".to_string(), "乙".to_string()),
            Block::Numbered(0, "2.".to_string(), "二".to_string()),
        ]
    );
}

#[test]
fn a_numbered_list_after_another_block_starts_over() {
    let blocks = parse("1. 一\n\n段落\n\n1. 二");
    assert_eq!(
        blocks,
        vec![
            Block::Numbered(0, "1.".to_string(), "一".to_string()),
            Block::Paragraph("段落".to_string()),
            Block::Numbered(0, "1.".to_string(), "二".to_string()),
        ]
    );
    // A bullet list between them ends the count too.
    let blocks = parse("1. 一\n- 记\n1. 二");
    assert!(matches!(&blocks[2], Block::Numbered(_, marker, _) if marker == "1."));
}

#[test]
fn a_hash_without_a_space_is_text() {
    assert_eq!(parse("#标签"), vec![Block::Paragraph("#标签".to_string())]);
}

#[test]
fn a_list_item_runs_on_to_its_next_line() {
    let blocks = parse("- 第一行\n  接着写\n- 第二项");
    assert_eq!(
        blocks,
        vec![
            Block::Bullet(0, "第一行接着写".to_string()),
            Block::Bullet(0, "第二项".to_string()),
        ]
    );
}

#[test]
fn a_wrapped_task_keeps_its_box() {
    let blocks = parse("- [x] 做完\n  的一半");
    assert_eq!(blocks, vec![Block::Task(0, true, "做完的一半".to_string())]);
}

#[test]
fn a_paragraph_under_a_list_stays_a_paragraph() {
    // Only an indented line continues an item, so a model that forgets the
    // blank line does not glue the next paragraph into the list.
    let blocks = parse("- 一项\n下一段");
    assert_eq!(
        blocks,
        vec![
            Block::Bullet(0, "一项".to_string()),
            Block::Paragraph("下一段".to_string()),
        ]
    );
}

#[test]
fn a_four_space_continuation_is_text_not_code() {
    let blocks = parse("- 一项\n    接着");
    assert_eq!(blocks, vec![Block::Bullet(0, "一项接着".to_string())]);
}

#[test]
fn a_fence_under_a_list_item_loses_its_own_indent() {
    // The indentation that puts the fence inside the item is layout, not code.
    // It also keeps the block under the item, one level in.
    let blocks = parse("- 一项\n\n  ```rs\n  let a = 1;\n  ```");
    assert_eq!(blocks[0], Block::Bullet(0, "一项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the fence under the item");
    };
    assert_eq!(
        inner.block,
        Block::Code("rs".to_string(), "let a = 1;".to_string())
    );
    // A tab is four columns, like the opening line's own indent.
    let blocks = parse("- 一项\n\n\t```\n\tx\n\t```");
    assert_eq!(blocks[0], Block::Bullet(0, "一项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the fence under the item");
    };
    assert_eq!(inner.block, Block::Code(String::new(), "x".to_string()));
}

#[test]
fn a_www_cell_copies_back_as_the_address_it_was() {
    let table = Table {
        header: row!("地址"),
        rows: vec![vec![styled_cell("www.example.com")]],
        aligns: vec![Align::Left],
    };
    assert_eq!(table.markdown(), "| 地址 |\n| --- |\n| www.example.com |");
}

#[test]
fn footnotes_are_numbered_by_their_reference_and_drawn_at_the_end() {
    let rich = parse_rich("术语[^1] 与另一个[^note]。\n\n[^note]: 第二条。\n[^1]: 第一条。");
    check_runs(&rich);
    // The reference draws its number, not its label; the notes themselves
    // come last, numbered the same way.
    assert_eq!(body_of(&rich[0].block), Some("术语¹ 与另一个²。"));
    assert_eq!(rich[1].block, Block::Footnote(1, vec![para("第一条。")]));
    assert_eq!(rich[2].block, Block::Footnote(2, vec![para("第二条。")]));
}

#[test]
fn an_indented_line_carries_a_footnote_on() {
    let blocks = parse("看这里[^a]。\n\n[^a]: 第一行\n  接着写。\n\n正文继续。");
    assert_eq!(
        blocks,
        vec![
            Block::Paragraph("看这里¹。".to_string()),
            Block::Paragraph("正文继续。".to_string()),
            // The notes are all drawn at the end, however far down the
            // definition was written.
            Block::Footnote(1, vec![para("第一行接着写。")]),
        ]
    );
}

#[test]
fn an_inline_note_is_drawn_with_the_other_notes() {
    let rich = parse_rich("正文^[就地说明] 后面");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("正文¹ 后面"));
    assert_eq!(rich[1].block, Block::Footnote(1, vec![para("就地说明")]));
    // The two kinds share one numbering, in the order they are referred to.
    let rich = parse_rich("甲[^a] 乙^[就地] 丙[^b]\n\n[^a]: 一\n[^b]: 二");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("甲¹ 乙² 丙³"));
    assert_eq!(rich[1].block, Block::Footnote(1, vec![para("一")]));
    assert_eq!(rich[2].block, Block::Footnote(2, vec![para("就地")]));
    assert_eq!(rich[3].block, Block::Footnote(3, vec![para("二")]));
    // Emphasis in the note is read like any other text.
    let rich = parse_rich("看^[**重点**]");
    let Block::Footnote(_, inner) = &rich[1].block else {
        panic!("expected a note");
    };
    assert!(inner[0].spans.iter().any(|span| span.bold));
}

#[test]
fn a_bracket_that_is_not_an_inline_note_stays_text() {
    // Inside code the marks are characters.
    assert_eq!(
        parse("`^[x]` 与 ^[] 与 \\^[y]"),
        vec![Block::Paragraph("^[x] 与 ^[] 与 ^[y]".to_string())]
    );
    // A fence is code, not a note.
    assert_eq!(
        parse("```\n^[x]\n```"),
        vec![Block::Code(String::new(), "^[x]".to_string())]
    );
}

#[test]
fn an_unknown_footnote_and_an_unreferred_definition_still_read() {
    // A reference with no definition stays as it was written.
    let rich = parse_rich("没有定义的[^x]");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("没有定义的[^x]"));
    // A note nobody refers to is still drawn, in definition order.
    assert_eq!(
        parse("正文。\n\n[^d]: 没被引用。"),
        vec![
            Block::Paragraph("正文。".to_string()),
            Block::Footnote(1, vec![para("没被引用。")]),
        ]
    );
}

#[test]
fn a_details_section_holds_blocks_of_its_own() {
    let blocks =
        parse("<details> <summary>点我</summary>\n内容：\n- 一\n```\ncode\n```\n</details>");
    let Block::Details(summary, open, inner) = &blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(summary.text, "点我");
    assert!(!open, "without the attribute it starts collapsed");
    assert_eq!(
        inner
            .iter()
            .map(|rich| rich.block.clone())
            .collect::<Vec<_>>(),
        vec![
            Block::Paragraph("内容：".to_string()),
            Block::Bullet(0, "一".to_string()),
            Block::Code(String::new(), "code".to_string()),
        ]
    );
    // `<details open>` starts expanded.
    let blocks = parse("<details open>\n正文\n</details>");
    assert!(matches!(&blocks[0], Block::Details(_, true, _)));
}

#[test]
fn nested_details_sections_close_one_at_a_time() {
    let blocks = parse(
        "<details>\n<summary>外</summary>\n<details><summary>内</summary>文字</details>\n</details>",
    );
    assert_eq!(blocks.len(), 1, "the outer close is not left as text");
    let Block::Details(summary, _, inner) = &blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(summary.text, "外");
    assert_eq!(inner.len(), 1);
    let Block::Details(inner_summary, _, inner_blocks) = &inner[0].block else {
        panic!("expected a nested details");
    };
    assert_eq!(inner_summary.text, "内");
    assert_eq!(inner_blocks[0].block, Block::Paragraph("文字".to_string()));
}

#[test]
fn an_unclosed_details_section_still_draws() {
    let blocks = parse("<details>\n正文");
    let Block::Details(summary, _, inner) = &blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(summary.text, "详情");
    assert_eq!(inner[0].block, Block::Paragraph("正文".to_string()));
}

#[test]
fn a_details_summary_keeps_its_emphasis() {
    let blocks = parse("<details><summary>**粗** 与 `code`</summary>\n正文\n</details>");
    let Block::Details(summary, _, _) = &blocks[0] else {
        panic!("expected details");
    };
    assert_eq!(summary.text, "粗 与 code");
    assert!(summary.spans.iter().any(|span| span.bold));
    assert!(summary.spans.iter().any(|span| span.code));
}

#[test]
fn a_footnote_definition_may_use_a_fullwidth_colon() {
    let rich = parse_rich("正文[^1]\n\n[^1]：说明");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("正文¹"));
    let Block::Footnote(1, inner) = &rich[1].block else {
        panic!("expected the note");
    };
    assert_eq!(inner[0].block, Block::Paragraph("说明".to_string()));
    // A space before the colon is still a definition.
    let rich = parse_rich("正文[^1]\n\n[^1] : 说明");
    assert_eq!(body_of(&rich[0].block), Some("正文¹"));
    assert!(matches!(&rich[1].block, Block::Footnote(1, _)));
    // The same for a definition list.
    assert_eq!(
        parse("术语\n：说明"),
        vec![Block::Definition("术语\n说明".to_string())]
    );
}

#[test]
fn an_image_title_is_the_caption_when_the_alt_is_empty() {
    let blocks = parse("![](pic.png \"示意图\")");
    let Block::Image(caption, url) = &blocks[0] else {
        panic!("expected an image");
    };
    assert_eq!(caption, "示意图");
    assert_eq!(url, "pic.png");
    // With alt text the alt is the caption and the title stays a tooltip.
    let blocks = parse("![图](pic.png \"示意图\")");
    let Block::Image(caption, url) = &blocks[0] else {
        panic!("expected an image");
    };
    assert_eq!(caption, "图");
    assert_eq!(url, "pic.png");
}

#[test]
fn an_image_path_may_hold_a_space() {
    // Only a quoted run after the address is a title, so a local file name with
    // spaces stays one path instead of being cut at the first space.
    let blocks = parse("![图](my file.png)");
    let Block::Image(caption, url) = &blocks[0] else {
        panic!("expected an image");
    };
    assert_eq!(caption, "图");
    assert_eq!(url, "my file.png");
    // An address in angle brackets loses them.
    let blocks = parse("![图](<my file.png>)");
    let Block::Image(_, url) = &blocks[0] else {
        panic!("expected an image");
    };
    assert_eq!(url, "my file.png");
}

#[test]
fn html_comments_draw_nothing() {
    assert_eq!(
        parse("前<!-- 备注 -->后"),
        vec![Block::Paragraph("前后".to_string())]
    );
    // On its own line, even when it spans several.
    assert_eq!(
        parse("上\n\n<!-- 多\n行 -->\n\n下"),
        vec![
            Block::Paragraph("上".to_string()),
            Block::Paragraph("下".to_string()),
        ]
    );
    // Inside a fence it is code.
    assert_eq!(
        parse("```\n<!-- x -->\n```"),
        vec![Block::Code(String::new(), "<!-- x -->".to_string())]
    );
}

#[test]
fn an_html_hr_is_a_rule() {
    assert_eq!(
        parse("上\n\n<hr>\n\n下"),
        vec![
            Block::Paragraph("上".to_string()),
            Block::Rule,
            Block::Paragraph("下".to_string()),
        ]
    );
    assert_eq!(parse("<hr class=\"x\">"), vec![Block::Rule]);
}

#[test]
fn an_html_img_line_is_a_picture() {
    let blocks = parse("<img src=\"pic.png\" alt=\"示意图\">");
    let Block::Image(caption, url) = &blocks[0] else {
        panic!("expected an image");
    };
    assert_eq!(caption, "示意图");
    assert_eq!(url, "pic.png");
    // Inline, the alt text stands in and a remote address is a link.
    let rich = parse_rich("看图 <img src=\"https://x/a.png\" alt=\"图\"> 吧");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("看图 图 吧"));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://x/a.png");
}

#[test]
fn a_code_cell_with_a_backtick_in_it_copies_back_whole() {
    let table = Table {
        header: row!("写法"),
        rows: vec![vec![styled_cell("``a`b``")], vec![styled_cell("`` `x` ``")]],
        aligns: vec![Align::Left],
    };
    assert_eq!(
        table.markdown(),
        "| 写法 |\n| --- |\n| ``a`b`` |\n| `` `x` `` |"
    );
}

#[test]
fn a_shortcut_reference_uses_its_own_label() {
    assert_eq!(
        parse("见 [文档]\n\n[文档]: https://example.com/d"),
        vec![Block::Paragraph("见 文档".to_string())]
    );
}

#[test]
fn a_reference_works_inside_a_table_cell() {
    let rich = parse_rich("| a |\n|---|\n| [文档][1] |\n\n[1]: https://example.com/t");
    check_runs(&rich);
    let Block::Table(table) = &rich[0].block else {
        panic!("expected a table");
    };
    assert_eq!(table.rows[0][0].text, "文档");
    assert_eq!(table.rows[0][0].links[0].url, "https://example.com/t");
}

#[test]
fn a_definition_draws_nothing() {
    assert_eq!(parse("[a]: https://example.com"), Vec::new());
    // A label is matched without its case, and the first definition wins.
    let rich = parse_rich("[A]: https://example.com/1\n[a]: https://example.com/2\n用 [a]");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("用 a"));
    assert_eq!(rich[0].links[0].url, "https://example.com/1");
}

#[test]
fn a_definition_that_cannot_be_opened_stays_text() {
    assert_eq!(
        parse("[a]: javascript:alert(1)"),
        vec![Block::Paragraph("[a]: javascript:alert(1)".to_string())]
    );
}

#[test]
fn a_definition_inside_code_is_code() {
    assert_eq!(
        parse("```\n[a]: https://example.com\n```"),
        vec![Block::Code(
            String::new(),
            "[a]: https://example.com".to_string()
        )]
    );
    // An unknown label stays as it was written.
    assert_eq!(
        parse("看 [文档][nope]\n\n[别的]: https://example.com"),
        vec![Block::Paragraph("看 [文档][nope]".to_string())]
    );
}

#[test]
fn a_tab_after_the_marker_still_makes_an_item() {
    assert_eq!(parse("-\t一"), vec![Block::Bullet(0, "一".to_string())]);
    assert_eq!(parse("*\t一"), vec![Block::Bullet(0, "一".to_string())]);
    assert_eq!(
        parse("-\t[ ] 一"),
        vec![Block::Task(0, false, "一".to_string())]
    );
}

#[test]
fn a_second_paragraph_after_a_blank_line_stays_in_the_item() {
    // A loose list item is one block of text: the gap becomes a blank line
    // inside the item instead of a paragraph outside the list.
    assert_eq!(
        parse("- 第一段\n\n  第二段"),
        vec![Block::Bullet(0, "第一段\n\n第二段".to_string())]
    );
    // The item may run on over several such paragraphs.
    assert_eq!(
        parse("1. 一\n\n   续\n\n   再续\n2. 二"),
        vec![
            Block::Numbered(0, "1.".to_string(), "一\n\n续\n\n再续".to_string()),
            Block::Numbered(0, "2.".to_string(), "二".to_string()),
        ]
    );
}

#[test]
fn a_blank_line_after_an_item_does_not_swallow_later_blocks() {
    // Four-space indentation is still a code block, kept under the item.
    let blocks = parse("- 项\n\n    let a = 1;");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the code under the item");
    };
    assert_eq!(
        inner.block,
        Block::Code(String::new(), "let a = 1;".to_string())
    );
    // Unindented text is still a paragraph of its own.
    assert_eq!(
        parse("- 项\n\n下一段"),
        vec![
            Block::Bullet(0, "项".to_string()),
            Block::Paragraph("下一段".to_string()),
        ]
    );
    // A nested item stays nested.
    assert_eq!(
        parse("- 一\n\n  - 二"),
        vec![
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(1, "二".to_string()),
        ]
    );
    // A table and a setext heading keep the shape their next line gives them,
    // and stay under the item they were written in.
    let blocks = parse("- 项\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the table under the item");
    };
    assert_eq!(
        inner.block,
        Block::Table(Table {
            header: vec![Cell::from("a"), Cell::from("b")],
            rows: vec![vec![Cell::from("1"), Cell::from("2")]],
            aligns: vec![Align::Left, Align::Left],
        })
    );
    let blocks = parse("- 项\n\n  标题\n  ===");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the heading under the item");
    };
    assert_eq!(inner.block, Block::Heading(1, "标题".to_string()));
}

#[test]
fn an_indented_block_after_an_item_is_not_swallowed_as_its_text() {
    // `<details>` and an image are blocks of their own, so an indented one under
    // a list item closes the item instead of continuing it — and stays under it.
    let blocks = parse("- 项\n  <details><summary>看</summary>内</details>");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the details under the item");
    };
    assert!(matches!(inner.block, Block::Details(_, _, _)));

    let blocks = parse("- 项\n  ![说明](pic.png)");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the image under the item");
    };
    assert_eq!(
        inner.block,
        Block::Image("说明".to_string(), "pic.png".to_string())
    );
}

#[test]
fn a_trailing_backslash_at_the_end_of_a_paragraph_is_a_character() {
    // There is no next line for the marker to break, so it stays.
    assert_eq!(parse("一\\"), vec![Block::Paragraph("一\\".to_string())]);
    // With a line after it, it is still a hard break.
    assert_eq!(
        parse("一\\\n二"),
        vec![Block::Paragraph("一\n二".to_string())]
    );
}

#[test]
fn text_that_only_held_markers_leaves_no_empty_paragraph() {
    assert!(parse("[](https://example.com)").is_empty());
    assert!(parse("<!-- 只是注释 -->").is_empty());
}

#[test]
fn an_html_pre_is_a_code_block() {
    assert_eq!(
        parse("<pre><code class=\"language-js\">\nlet a = 1;\n</code></pre>"),
        vec![Block::Code("js".to_string(), "let a = 1;".to_string())]
    );
    // Without a language or a `<code>` wrapper it is still code.
    assert_eq!(
        parse("<pre>\nlet a = 1;\n</pre>"),
        vec![Block::Code(String::new(), "let a = 1;".to_string())]
    );
    // Entities stand for the characters they name.
    assert_eq!(
        parse("<pre>&lt;div&gt;</pre>"),
        vec![Block::Code(String::new(), "<div>".to_string())]
    );
}

#[test]
fn html_inside_a_code_block_is_code() {
    assert_eq!(
        parse("```html\n<table><tr><td>x</td></tr></table>\n<details>y</details>\n```"),
        vec![Block::Code(
            "html".to_string(),
            "<table><tr><td>x</td></tr></table>\n<details>y</details>".to_string()
        )]
    );
    // A four-space block keeps every indented line, even marker-like ones.
    assert_eq!(
        parse("    code\n    - 一\n    <details>"),
        vec![Block::Code(
            String::new(),
            "code\n- 一\n<details>".to_string()
        )]
    );
}

#[test]
fn html_lists_headings_and_paragraphs_are_blocks() {
    assert_eq!(
        parse("<ul><li>一</li><li>二</li></ul>"),
        vec![
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(0, "二".to_string()),
        ]
    );
    assert_eq!(
        parse("<ol><li>一</li><li>二</li></ol>"),
        vec![
            Block::Numbered(0, "1.".to_string(), "一".to_string()),
            Block::Numbered(0, "2.".to_string(), "二".to_string()),
        ]
    );
    assert_eq!(
        parse("<h2>标题</h2>"),
        vec![Block::Heading(2, "标题".to_string())]
    );
    // Inline tags inside a paragraph keep their emphasis.
    let rich = parse_rich("<p>一段 <b>粗</b> 话</p>");
    check_runs(&rich);
    assert_eq!(rich[0].block, Block::Paragraph("一段 粗 话".to_string()));
    assert!(rich[0].spans.iter().any(|span| span.bold));
    // A `<div>` is only a wrapper: what it holds is drawn as blocks.
    assert_eq!(
        parse("<div>\n- 一\n</div>"),
        vec![Block::Bullet(0, "一".to_string())]
    );
}

#[test]
fn a_tag_mentioned_in_prose_is_still_text() {
    for line in ["<h1> 是标题标签", "<ul> 是列表标签", "<div> 是块元素"] {
        assert_eq!(
            parse(line),
            vec![Block::Paragraph(line.to_string())],
            "{line}"
        );
    }
    // A closed element is a block.
    assert_eq!(
        parse("<h1>标题</h1>"),
        vec![Block::Heading(1, "标题".to_string())]
    );
}

#[test]
fn an_html_block_under_a_list_item_stays_under_it() {
    let blocks = parse("- 项\n\n  <pre>x</pre>");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the code under the item");
    };
    assert_eq!(inner.block, Block::Code(String::new(), "x".to_string()));

    // An HTML list makes several blocks; every one of them stays under the item.
    let blocks = parse("- 项\n\n  <ul><li>一</li><li>二</li></ul>");
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    for block in &blocks[1..] {
        let Block::Indented(0, Marker::Bullet, inner) = block else {
            panic!("expected the list items under the item");
        };
        assert!(matches!(inner.block, Block::Bullet(0, _)));
    }

    // A `<div>` wrapper is drawn as the blocks it holds, under the item too.
    let blocks = parse("- 项\n\n  <div>\n  - 一\n  - 二\n  </div>");
    assert_eq!(blocks.len(), 3);
    assert!(
        matches!(&blocks[1], Block::Indented(0, Marker::Bullet, inner) if matches!(inner.block, Block::Bullet(0, _)))
    );
}

#[test]
fn a_pre_keeps_the_indentation_of_its_code() {
    assert_eq!(
        parse("<pre>\n  let a = 1;\n</pre>"),
        vec![Block::Code(String::new(), "  let a = 1;".to_string())]
    );
    // The first class names the language, even with others after it.
    assert_eq!(
        parse("<pre><code class=\"language-js hljs\">x</code></pre>"),
        vec![Block::Code("js".to_string(), "x".to_string())]
    );
}

#[test]
fn an_html_table_is_drawn_as_a_table() {
    let blocks = parse(
        "<table>\n<tr><th>名称</th><th>值</th></tr>\n\
         <tr><td><b>甲</b></td><td>1</td></tr>\n\
         <tr><td>乙<br>丙</td><td>2</td></tr>\n</table>",
    );
    let Block::Table(table) = &blocks[0] else {
        panic!("expected a table");
    };
    assert_eq!(table.header[0].text, "名称");
    assert_eq!(table.header[1].text, "值");
    assert_eq!(table.rows[0][0].text, "甲");
    // `<b>` is the same emphasis Markdown writes.
    assert!(table.rows[0][0].spans[0].bold);
    // `<br>` keeps the words apart instead of running them together.
    assert_eq!(table.rows[1][0].text, "乙 丙");
    assert_eq!(table.rows[1][1].text, "2");

    // A link and a picture keep their link and their text.
    let blocks = parse(
        "<table><tr><th>a</th></tr><tr><td>\
         <a href=\"https://x.y\">点</a> <img src=\"p.png\" alt=\"图\"></td></tr></table>",
    );
    let Block::Table(table) = &blocks[0] else {
        panic!("expected a table");
    };
    assert_eq!(table.rows[0][0].text, "点 图");
    assert_eq!(table.rows[0][0].links[0].url, "https://x.y");
}

#[test]
fn an_html_blockquote_holds_blocks_of_its_own() {
    let blocks = parse("<blockquote>\n- 一\n- 二\n</blockquote>");
    let Block::Quote(inner) = &blocks[0] else {
        panic!("expected a quote");
    };
    assert_eq!(inner[0].block, Block::Bullet(0, "一".to_string()));
    assert_eq!(inner[1].block, Block::Bullet(0, "二".to_string()));
}
