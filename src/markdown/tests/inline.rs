//! Inline runs: emphasis, code spans, links, HTML tags, entities and strike-through.

use super::*;

#[test]
fn a_pair_of_stars_needs_a_pair_to_close_it() {
    // `2**10` is arithmetic: without a closing pair the stars stay stars.
    assert_eq!(
        parse("算 2**10 次方"),
        vec![Block::Paragraph("算 2**10 次方".to_string())]
    );
    assert_eq!(
        parse("**粗体** 后面"),
        vec![Block::Paragraph("粗体 后面".to_string())]
    );
    assert!(parse_rich("**粗体** 后面")[0].spans[0].bold);
}

#[test]
fn bold_can_end_with_italic() {
    // `**粗 *斜***`: the closing three close the italic and the bold.
    let rich = parse_rich("**粗 *斜***");
    assert_eq!(rich[0].block, Block::Paragraph("粗 斜".to_string()));
    assert_eq!(
        rich[0].spans,
        vec![
            span("粗 ".len(), true, false, false),
            span("斜".len(), true, true, false),
        ]
    );
}

#[test]
fn inline_html_tags_are_dropped_and_br_breaks_the_line() {
    assert_eq!(
        parse("一<br>二<br/>三"),
        vec![Block::Paragraph("一\n二\n三".to_string())]
    );
    // The text of an inline tag is kept, the tag itself goes.
    assert_eq!(
        parse("<kbd>Ctrl</kbd>+C 与 <b>粗</b>"),
        vec![Block::Paragraph("Ctrl+C 与 粗".to_string())]
    );
    // A tag nobody knows is text, and so is a stray `<`.
    assert_eq!(
        parse("看 <div> 与 a < b"),
        vec![Block::Paragraph("看 <div> 与 a < b".to_string())]
    );
    // Inside code the brackets are code.
    assert_eq!(parse("`<br>`"), vec![Block::Paragraph("<br>".to_string())]);
}

#[test]
fn a_link_title_is_dropped() {
    let rich = parse_rich("看 [文档](https://example.com/docs \"官方文档\") 一下");
    assert_eq!(rich[0].block, Block::Paragraph("看 文档 一下".to_string()));
    assert_eq!(rich[0].links[0].url, "https://example.com/docs");
    // A `)` inside the title does not end the address early.
    let rich = parse_rich("[a](https://example.com/x 'a)b')");
    assert_eq!(rich[0].links[0].url, "https://example.com/x");
}

#[test]
fn an_address_in_a_link_keeps_a_balanced_pair_of_brackets() {
    let rich = parse_rich("[维基](https://en.wikipedia.org/wiki/A_(b)) 后面");
    assert_eq!(rich[0].block, Block::Paragraph("维基 后面".to_string()));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://en.wikipedia.org/wiki/A_(b)");
    // The bracket that closes the link is not part of the address.
    assert_eq!(rich[0].links[0].end, "维基".len());
}

#[test]
fn a_link_can_wrap_its_address_in_angle_brackets() {
    // The brackets let the address hold a space, and they are not part of it.
    let rich = parse_rich("[文档](<https://example.com/a b>) 后面");
    assert_eq!(rich[0].block, Block::Paragraph("文档 后面".to_string()));
    assert_eq!(rich[0].links[0].url, "https://example.com/a b");
    let rich = parse_rich("[文档](<https://example.com/x> \"说明\")");
    assert_eq!(rich[0].links[0].url, "https://example.com/x");
}

#[test]
fn bold_around_an_address_leaves_the_stars_out_of_the_link() {
    let rich = parse_rich("**https://example.com/a**");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("https://example.com/a".to_string())
    );
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://example.com/a");
    // The address is bold, like the text it sits in.
    assert!(rich[0].spans.iter().all(|span| span.bold));
}

#[test]
fn a_quote_inside_a_quote_is_its_own_box() {
    let blocks = parse("> > 里面\n> > 还是里面\n> 外面");
    assert_eq!(
        blocks,
        vec![Block::Quote(vec![
            Rich {
                block: quoted("里面还是里面"),
                spans: Vec::new(),
                links: Vec::new(),
            },
            para("外面"),
        ])]
    );
    // A `>` inside the text is not a marker.
    assert_eq!(parse("> a > b"), vec![quoted("a > b")]);
}

#[test]
fn emphasis_survives_a_joined_quote() {
    let rich = parse_rich("> **重点**\n> 普通");
    assert_eq!(rich.len(), 1);
    let Block::Quote(inner) = &rich[0].block else {
        panic!("expected a quote");
    };
    assert_eq!(inner.len(), 1);
    assert_eq!(inner[0].block, Block::Paragraph("重点普通".to_string()));
    assert_eq!(
        inner[0].spans,
        vec![
            span("重点".len(), true, false, false),
            span("普通".len(), false, false, false),
        ]
    );
}

#[test]
fn strips_inline_markers() {
    let blocks = parse("用 `read` **工具** 读文件");
    assert_eq!(
        blocks,
        vec![Block::Paragraph("用 read 工具 读文件".to_string())]
    );
}

#[test]
fn bold_and_code_keep_their_runs() {
    let rich = parse_rich("用 `read` **工具** 读文件");
    assert_eq!(rich.len(), 1);
    assert_eq!(
        rich[0].block,
        Block::Paragraph("用 read 工具 读文件".to_string())
    );
    assert_eq!(
        rich[0].spans,
        vec![
            span("用 ".len(), false, false, false),
            span(4, false, false, true),
            span(1, false, false, false),
            span("工具".len(), true, false, false),
            span(" 读文件".len(), false, false, false),
        ]
    );
}

#[test]
fn half_written_markup_never_breaks_the_runs() {
    // Text arrives a token at a time and models write odd markup; every one
    // of these has to come out as blocks whose runs still cover the text.
    let nasty = [
        "",
        "*",
        "**",
        "***",
        "****",
        "**a",
        "a**",
        "a**b**c",
        "~",
        "~~",
        "~~~",
        "~~a",
        "~~a~~b~~",
        "~~`a~~`",
        "``a`b``",
        "``a`b`` **c**",
        "**a `` ` `` b**",
        "`` ` ``",
        "&amp;",
        "&#x4e2d;",
        "&",
        "&amp",
        "www.example.com",
        "xwww.example.com",
        "https://a/(b)c)",
        "[^1]",
        "[^note] 与 [^1]",
        "[^a]: 一条注释",
        "<user@example.com>",
        "<不是地址>",
        "![图](https://example.com/a.png)",
        "![](x)",
        "$",
        "$$",
        "$a^2$",
        "$ 空的 $",
        "$没有结尾",
        "$$ \\frac{1}{2} $$",
        "<details>",
        "<details><summary>x</summary>\n正文",
        "<details></details>",
        "\\frac{}{}",
        "\\",
        "\\*",
        "\\`a`",
        "\\\\*a*",
        "_",
        "__",
        "___a__",
        "a_b_c",
        "_a_b_",
        "[",
        "]",
        "[a]",
        "[a](",
        "[a](https://x",
        "[a](https://x \"t",
        "![a](https://x)",
        "!",
        "<",
        "<https://x",
        "`",
        "``a",
        "`**a**`",
        "a`b",
        "*a*_b_",
        "**~~a~~**",
        "**a *b* c**",
        "> ",
        "> **a**",
        "- ",
        "- [",
        "- [x",
        "- [x]",
        "- [x] *a*",
        "1.",
        "1. ",
        "  - x",
        "\t- y",
        "|",
        "| a |",
        "| a |\n|---|",
        "| a | b |\n|---|---|\n| `x",
        "#",
        "# ",
        "---",
        "===",
        "text\n---",
        "text\n===",
        "* 项目",
        "*emphasis*",
        "a ~ b ~~c~~ d",
        "**看 https://example.com**",
        "2**10 次方",
        "**粗 *斜***",
        "***a***b",
        "__init__ 与 obj.__dict__",
        "一<br>二<br/>三",
        "<kbd>Ctrl</kbd>+C",
        "<b>x</b> 与 <div>",
        "<",
        "<<b>>",
    ];
    for text in nasty {
        let rich = parse_rich(text);
        check_runs(&rich);
        // Parsing twice gives the same answer: nothing depends on state
        // left over from an earlier call.
        assert_eq!(rich, parse_rich(text), "{text:?}");
    }
}

#[test]
fn an_asterisk_inside_code_is_text() {
    let rich = parse_rich("用 `a**b` 连接");
    assert_eq!(rich[0].block, Block::Paragraph("用 a**b 连接".to_string()));
    assert_eq!(
        rich[0].spans,
        vec![
            span("用 ".len(), false, false, false),
            span("a**b".len(), false, false, true),
            span(" 连接".len(), false, false, false),
        ]
    );
}

#[test]
fn underscores_emphasise_too_but_not_inside_a_word() {
    assert_eq!(
        parse("看 _重点_ 吧"),
        vec![Block::Paragraph("看 重点 吧".to_string())]
    );
    let rich = parse_rich("看 _重点_ 吧");
    assert!(
        rich[0]
            .spans
            .iter()
            .any(|span| span.italic && span.len == "重点".len())
    );
    // Two underscores are bold, three are both.
    assert_eq!(
        parse("__加粗__ 与 ___都要___"),
        vec![Block::Paragraph("加粗 与 都要".to_string())]
    );
    let rich = parse_rich("__加粗__");
    assert!(rich[0].spans.iter().any(|span| span.bold));
    let rich = parse_rich("___都要___");
    assert!(rich[0].spans.iter().any(|span| span.bold && span.italic));
    // A name is a name: underscores between letters and digits stay.
    assert_eq!(
        parse("文件 foo_bar.ts 与 snake_case_2"),
        vec![Block::Paragraph(
            "文件 foo_bar.ts 与 snake_case_2".to_string()
        )]
    );
    assert!(
        parse_rich("文件 foo_bar.ts")
            .iter()
            .flat_map(|rich| rich.spans.iter())
            .all(|span| !span.italic && !span.bold)
    );
}

#[test]
fn a_star_between_spaces_stays_a_star() {
    let rich = parse_rich("5 * 3 等于 15");
    assert_eq!(rich[0].block, Block::Paragraph("5 * 3 等于 15".to_string()));
    assert_eq!(rich[0].spans.len(), 1);
    assert!(!rich[0].spans[0].italic);
}

#[test]
fn an_unclosed_star_stays_a_star() {
    let rich = parse_rich("*没有结束");
    assert_eq!(rich[0].block, Block::Paragraph("*没有结束".to_string()));
    assert!(rich[0].spans.iter().all(|span| !span.italic));
}

#[test]
fn bold_inside_italic_keeps_both() {
    let rich = parse_rich("*斜 **粗** 斜*");
    assert_eq!(rich[0].block, Block::Paragraph("斜 粗 斜".to_string()));
    let bold = rich[0]
        .spans
        .iter()
        .find(|span| span.bold)
        .expect("a bold run");
    assert!(bold.italic);
    assert!(
        rich[0]
            .spans
            .first()
            .is_some_and(|span| span.italic && !span.bold)
    );
}

#[test]
fn tilde_fences_open_a_code_block_too() {
    assert_eq!(
        parse("看：\n~~~ts\nconst a = 1;\n~~~\n好了"),
        vec![
            Block::Paragraph("看：".to_string()),
            Block::Code("ts".to_string(), "const a = 1;".to_string()),
            Block::Paragraph("好了".to_string()),
        ]
    );
    // The marker that opened the block is the one that closes it: a line of
    // backticks inside a tilde block is code.
    assert_eq!(
        parse("~~~\n```\n~~~"),
        vec![Block::Code(String::new(), "```".to_string())]
    );
    // An unclosed fence runs to the end of the answer.
    assert_eq!(
        parse("~~~\nabc"),
        vec![Block::Code(String::new(), "abc".to_string())]
    );
    // Two tildes are a strikethrough, not a fence.
    assert_eq!(
        parse("~~删除~~"),
        vec![Block::Paragraph("删除".to_string())]
    );
}

#[test]
fn an_inline_image_draws_as_a_link_to_it() {
    let rich = parse_rich("看图 ![截图](https://example.com/a.png) 好");
    assert_eq!(rich[0].block, Block::Paragraph("看图 截图 好".to_string()));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://example.com/a.png");
}

#[test]
fn an_inline_image_shows_its_alt_text_even_when_the_path_cannot_be_opened() {
    // A local picture is not a link, but its alt text still belongs in the
    // sentence: the raw `![]()` syntax is not what the answer meant to show.
    let rich = parse_rich("看图 ![截图](pic.png) 好");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("看图 截图 好"));
    assert!(rich[0].links.is_empty());
    // The same in a table cell.
    let rich = parse_rich("| a |\n|---|\n| ![图](pic.png) |");
    check_runs(&rich);
    let Block::Table(table) = &rich[0].block else {
        panic!("expected a table");
    };
    assert_eq!(table.rows[0][0].text, "图");
    assert!(table.rows[0][0].links.is_empty());
}

#[test]
fn an_empty_picture_or_link_takes_one_of_the_spaces_around_it() {
    // Nothing is drawn, and the sentence should not be left with two spaces.
    let rich = parse_rich("看 ![](https://a.b/x.png) 好");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("看 好"));
    let rich = parse_rich("a [](https://a.b) b");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("a b"));
    // Without a space on the other side nothing is taken away.
    let rich = parse_rich("看 ![](https://a.b/x.png)，好");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("看 ，好"));
}

#[test]
fn a_picture_wrapped_in_a_link_draws_its_alt_text() {
    // `[![alt](img)](url)` is a picture that opens a page: the label keeps the
    // alt text and only the outer address is clickable.
    let rich = parse_rich("[![图](https://x/a.png)](https://example.com)");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("图"));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://example.com");
}

#[test]
fn blocks_under_a_list_item_stay_under_it() {
    // A quote written under an item is drawn under it, one level in.
    let blocks = parse("- 项\n\n  > 引用");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    let Block::Indented(0, Marker::Bullet, inner) = &blocks[1] else {
        panic!("expected the quote under the item");
    };
    assert!(matches!(inner.block, Block::Quote(_)));

    // Several blocks may follow each other under the same item.
    let blocks = parse("- 项\n\n  > 引用\n\n  ```js\n  x\n  ```");
    assert_eq!(blocks[0], Block::Bullet(0, "项".to_string()));
    assert_eq!(blocks.len(), 3, "both blocks stay under the item");
    let Block::Indented(0, Marker::Bullet, quote) = &blocks[1] else {
        panic!("expected the quote under the item");
    };
    assert!(matches!(quote.block, Block::Quote(_)));
    let Block::Indented(0, Marker::Bullet, code) = &blocks[2] else {
        panic!("expected the fence under the item");
    };
    assert_eq!(code.block, Block::Code("js".to_string(), "x".to_string()));

    // A nested item's blocks sit at the nested level.
    let blocks = parse("- 一\n  - 二\n\n    > 三");
    let Block::Indented(1, Marker::Bullet, inner) = &blocks[2] else {
        panic!("expected the quote under the nested item");
    };
    assert!(matches!(inner.block, Block::Quote(_)));

    // A numbered item's number column is wider, and so is its child's inset.
    let blocks = parse("1. 一\n\n   > 二");
    let Block::Indented(0, Marker::Number, _) = &blocks[1] else {
        panic!("expected the quote under the numbered item");
    };
}

#[test]
fn a_footnote_may_hold_blocks_of_its_own() {
    let rich = parse_rich("正文[^a]\n\n[^a]: 说明\n\n    - 一\n    - 二");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("正文¹"));
    let Block::Footnote(1, inner) = &rich[1].block else {
        panic!("expected the note");
    };
    assert_eq!(inner[0].block, Block::Paragraph("说明".to_string()));
    assert_eq!(inner[1].block, Block::Bullet(0, "一".to_string()));
    assert_eq!(inner[2].block, Block::Bullet(0, "二".to_string()));

    // A fence keeps its own shape too, and its indentation is layout.
    let rich = parse_rich("看[^b]\n\n[^b]: 示例\n\n    ```js\n    let a = 1;\n    ```");
    check_runs(&rich);
    let Block::Footnote(1, inner) = &rich[1].block else {
        panic!("expected the note");
    };
    assert_eq!(inner[0].block, Block::Paragraph("示例".to_string()));
    assert_eq!(
        inner[1].block,
        Block::Code("js".to_string(), "let a = 1;".to_string())
    );
}

#[test]
fn several_terms_share_one_description() {
    let rich = parse_rich("术语一\n术语二\n: 说明");
    check_runs(&rich);
    assert_eq!(rich.len(), 1);
    assert_eq!(
        rich[0].block,
        Block::Definition("术语一\n术语二\n说明".to_string())
    );
    // Both terms are drawn bold, and the description is not.
    let spans = &rich[0].spans;
    assert!(spans.first().is_some_and(|span| span.bold));
    assert!(spans.last().is_some_and(|span| !span.bold));
    assert_eq!(
        spans.iter().map(|span| span.len).sum::<usize>(),
        "术语一\n术语二\n说明".len()
    );
}

#[test]
fn an_image_on_its_own_line_is_a_placeholder() {
    let rich = parse_rich("![图片占位](https://example.com/a.png)");
    check_runs(&rich);
    assert_eq!(
        rich[0].block,
        Block::Image(
            "图片占位".to_string(),
            "https://example.com/a.png".to_string()
        )
    );
    // The alt text is the link, so the card can be clicked; with no alt text
    // there is nothing to click, and an address that cannot be opened stays
    // text.
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://example.com/a.png");
    assert!(
        parse_rich("![](https://example.com/a.png)")[0]
            .links
            .is_empty()
    );
    assert!(parse_rich("![图](javascript:x)")[0].links.is_empty());
}

#[test]
fn carets_raise_and_single_tildes_lower() {
    assert_eq!(
        parse("x^2^ 与 H~2~O"),
        vec![Block::Paragraph("x² 与 H₂O".to_string())]
    );
    assert_eq!(parse("a^b^"), vec![Block::Paragraph("aᵇ".to_string())]);
    // Without a closing mark, or with a space inside, the marks are characters.
    assert_eq!(
        parse("2^10 倍"),
        vec![Block::Paragraph("2^10 倍".to_string())]
    );
    assert_eq!(
        parse("5~10 个"),
        vec![Block::Paragraph("5~10 个".to_string())]
    );
    // Nothing with no script form is converted, so the marks stay put.
    assert_eq!(
        parse("^中文^"),
        vec![Block::Paragraph("^中文^".to_string())]
    );
    assert_eq!(
        parse("^_^ 笑脸"),
        vec![Block::Paragraph("^_^ 笑脸".to_string())]
    );
    // Inside code they are characters.
    assert_eq!(parse("`x^2^`"), vec![Block::Paragraph("x^2^".to_string())]);
    // A strikethrough still wins over the subscript reading.
    let rich = parse_rich("~~删除~~");
    assert!(rich[0].spans.iter().any(|span| span.strike));
    assert_eq!(body_of(&rich[0].block), Some("删除"));
}

#[test]
fn a_short_code_becomes_its_picture() {
    assert_eq!(
        parse("好耶 :tada: 与 :rocket:"),
        vec![Block::Paragraph("好耶 🎉 与 🚀".to_string())]
    );
    // A name not on the list, a time and a path keep their colons.
    assert_eq!(
        parse(":unknown:"),
        vec![Block::Paragraph(":unknown:".to_string())]
    );
    assert_eq!(
        parse("12:30 开会"),
        vec![Block::Paragraph("12:30 开会".to_string())]
    );
    assert_eq!(parse("a::b"), vec![Block::Paragraph("a::b".to_string())]);
    // An address is not a short code: the colon at the end is the sentence's,
    // and the name inside it is left to the link.
    let rich = parse_rich("https://example.com/:smile:");
    assert_eq!(rich[0].links[0].url, "https://example.com/:smile");
    // A name that is also an identifier (`Vec::new()`) is left alone.
    assert_eq!(
        parse("Vec::new()"),
        vec![Block::Paragraph("Vec::new()".to_string())]
    );
    // Inside code the colons are characters.
    assert_eq!(
        parse("`:tada:`"),
        vec![Block::Paragraph(":tada:".to_string())]
    );
}

#[test]
fn double_equals_highlights_a_run() {
    let rich = parse_rich("看 ==重点== 与普通");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("看 重点 与普通".to_string())
    );
    let marked: Vec<&Span> = rich[0].spans.iter().filter(|span| span.marked).collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(marked[0].len, "重点".len());
    // Copying the run back gives the Markdown that made it.
    assert_eq!(styled_cell("看 ==重点==").markdown(), "看 ==重点==");
}

#[test]
fn a_lone_equals_pair_stays_text() {
    // A comparison is not a highlight: the marks need text right inside them.
    assert_eq!(
        parse("if a == b 则"),
        vec![Block::Paragraph("if a == b 则".to_string())]
    );
    assert_eq!(parse("x = 1"), vec![Block::Paragraph("x = 1".to_string())]);
    // `===` is a comparison too, and an unclosed pair stays as written.
    assert_eq!(
        parse("a === b"),
        vec![Block::Paragraph("a === b".to_string())]
    );
    assert_eq!(
        parse("==没有收尾"),
        vec![Block::Paragraph("==没有收尾".to_string())]
    );
    // Inside code the marks are characters.
    assert_eq!(
        parse("`a == b`"),
        vec![Block::Paragraph("a == b".to_string())]
    );
}

#[test]
fn double_tildes_strike_through_only_what_they_wrap() {
    let rich = parse_rich("旧的 ~~方案~~ 已经废弃");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("旧的 方案 已经废弃".to_string())
    );
    let struck: Vec<&Span> = rich[0].spans.iter().filter(|span| span.strike).collect();
    assert_eq!(struck.len(), 1);
    assert_eq!(struck[0].len, "方案".len());
    assert!(!struck[0].bold && !struck[0].italic);
}

#[test]
fn a_single_tilde_is_text() {
    // Ranges and approximations use one tilde; they are not emphasis.
    assert_eq!(
        parse("大概 5~10 个，约 ~~~ 这样"),
        vec![Block::Paragraph("大概 5~10 个，约 ~~~ 这样".to_string())]
    );
    // A pair with no closing pair stays as it was written.
    assert_eq!(
        parse("没有结束 ~~这里"),
        vec![Block::Paragraph("没有结束 ~~这里".to_string())]
    );
}

#[test]
fn an_angled_address_is_a_link_without_its_brackets() {
    let rich = parse_rich("看 <https://example.com/a> 与 <不是链接>");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("看 https://example.com/a 与 <不是链接>".to_string())
    );
    assert_eq!(
        rich[0].links,
        vec![Link {
            start: "看 ".len(),
            end: "看 https://example.com/a".len(),
            url: "https://example.com/a".to_string(),
        }]
    );
    // Inside code the brackets stay text.
    let rich = parse_rich("`<https://example.com>`");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("<https://example.com>".to_string())
    );
    assert!(rich[0].links.is_empty());
}

#[test]
fn a_backslash_takes_the_next_marker_literally() {
    assert_eq!(
        parse(r"用 \*星号\* 和 \`反引号\` 和 \[方括号\]"),
        vec![Block::Paragraph(
            "用 *星号* 和 `反引号` 和 [方括号]".to_string()
        )]
    );
    // A backslash before something ordinary is just a backslash.
    assert_eq!(
        parse(r"C:\Users\me"),
        vec![Block::Paragraph(r"C:\Users\me".to_string())]
    );
    // An escaped tilde is not a marker either.
    assert_eq!(
        parse(r"\~~不是删除线\~~"),
        vec![Block::Paragraph("~~不是删除线~~".to_string())]
    );
}

#[test]
fn a_link_is_drawn_as_its_label() {
    let rich = parse_rich("看 [文档](https://example.com/docs) 和 `[x](https://y)`");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("看 文档 和 [x](https://y)".to_string())
    );
    assert_eq!(
        rich[0].links,
        vec![Link {
            start: "看 ".len(),
            end: "看 文档".len(),
            url: "https://example.com/docs".to_string(),
        }]
    );
}

#[test]
fn only_http_links_are_links() {
    // Anything else would draw as a link that cannot be opened, so the
    // brackets stay as text.
    let rich = parse_rich("[本地](file:///C:/x) 与 [脚本](javascript:alert(1))");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("[本地](file:///C:/x) 与 [脚本](javascript:alert(1))".to_string())
    );
    assert!(rich[0].links.is_empty());
    assert!(is_openable_link("https://a.b"));
    assert!(is_openable_link("http://a.b"));
    assert!(!is_openable_link("http://"));
    assert!(!is_openable_link("file:///C:/x"));
    assert!(!is_openable_link("javascript:alert(1)"));
}

#[test]
fn a_link_keeps_the_emphasis_it_sits_in() {
    let rich = parse_rich("**看 [文档](https://example.com)**");
    assert_eq!(rich[0].block, Block::Paragraph("看 文档".to_string()));
    let label = &rich[0].links[0];
    assert_eq!(label.url, "https://example.com");
    // The label is its own run, drawn bold because the text around it is.
    let covered: usize = rich[0].spans.iter().map(|span| span.len).sum();
    assert_eq!(covered, "看 文档".len());
    assert!(rich[0].spans.iter().any(|span| span.bold));
}

#[test]
fn a_link_copies_back_as_markdown() {
    let cell = styled_cell("看 [文档](https://example.com) 一下");
    assert_eq!(cell.text, "看 文档 一下");
    assert_eq!(cell.markdown(), "看 [文档](https://example.com) 一下");
}

#[test]
fn a_bare_address_is_a_link_too() {
    let rich = parse_rich("见 https://example.com/a_b 好了");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("见 https://example.com/a_b 好了".to_string())
    );
    assert_eq!(rich[0].links.len(), 1);
    let link = &rich[0].links[0];
    assert_eq!(link.url, "https://example.com/a_b");
    let Block::Paragraph(text) = &rich[0].block else {
        panic!("expected a paragraph");
    };
    assert_eq!(&text[link.start..link.end], "https://example.com/a_b");
}

#[test]
fn trailing_punctuation_stays_out_of_a_bare_link() {
    let rich = parse_rich("见 https://example.com。还有 (https://example.com/x) 结束");
    let urls: Vec<&str> = rich[0].links.iter().map(|link| link.url.as_str()).collect();
    assert_eq!(urls, vec!["https://example.com", "https://example.com/x"]);
    // The sentence punctuation is still drawn.
    assert_eq!(
        rich[0].block,
        Block::Paragraph("见 https://example.com。还有 (https://example.com/x) 结束".to_string())
    );
}

#[test]
fn a_bare_address_needs_a_word_boundary_and_a_host() {
    assert!(parse_rich("xhttps://example.com")[0].links.is_empty());
    assert!(parse_rich("https://")[0].links.is_empty());
    // Inside code it is text.
    assert!(
        parse_rich("用 `https://example.com` 打开")[0]
            .links
            .is_empty()
    );
}

#[test]
fn a_bare_address_copies_back_as_itself() {
    let cell = styled_cell("看 https://example.com 一下");
    assert_eq!(cell.text, "看 https://example.com 一下");
    assert_eq!(cell.markdown(), "看 https://example.com 一下");
    // A written link still gets its brackets back.
    let cell = styled_cell("看 [文档](https://example.com) 一下");
    assert_eq!(cell.markdown(), "看 [文档](https://example.com) 一下");
}

#[test]
fn a_cell_copies_back_as_markdown_with_its_markers() {
    let cell = styled_cell("用 `read` **读**");
    assert_eq!(cell.markdown(), "用 `read` **读**");
}

#[test]
fn bold_in_a_cell_becomes_a_bold_run() {
    let blocks = parse("| 项目 | 数量 |\n|---|---|\n| **总计** | 12 |");
    let Block::Table(table) = &blocks[0] else {
        panic!("expected a table");
    };
    let cell = &table.rows[0][0];
    assert_eq!(cell.text, "总计");
    assert_eq!(cell.spans, vec![span(6, true, false, false)]);
}

#[test]
fn a_cell_mixes_bold_and_plain_runs() {
    let cell = styled_cell("共 **3** 个");
    assert_eq!(cell.text, "共 3 个");
    assert_eq!(
        cell.spans,
        vec![
            span("共 ".len(), false, false, false),
            span(1, true, false, false),
            span(" 个".len(), false, false, false),
        ]
    );
}

#[test]
fn a_cell_keeps_inline_code_in_the_code_run() {
    let cell = styled_cell("`read` 一行");
    assert_eq!(cell.text, "read 一行");
    assert_eq!(
        cell.spans,
        vec![
            span(4, false, false, true),
            span(" 一行".len(), false, false, false),
        ]
    );
}

#[test]
fn a_table_copies_back_as_markdown() {
    let table = Table {
        header: row!("a", "b"),
        rows: vec![row!("1", "2 | 3")],
        aligns: vec![Align::Center, Align::Right],
    };
    assert_eq!(
        table.markdown(),
        "| a | b |\n| :--: | ---: |\n| 1 | 2 \\| 3 |"
    );
}

#[test]
fn a_wrapped_item_keeps_its_emphasis() {
    let rich = parse_rich("- **粗**开头\n  接着**也粗**");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("粗开头接着也粗"));
    assert_eq!(
        rich[0].spans,
        vec![
            span("粗".len(), true, false, false),
            span("开头接着".len(), false, false, false),
            span("也粗".len(), true, false, false),
        ]
    );
}

#[test]
fn an_indented_marker_under_an_item_still_nests() {
    let blocks = parse("- 一项\n  - 嵌套");
    assert_eq!(
        blocks,
        vec![
            Block::Bullet(0, "一项".to_string()),
            Block::Bullet(1, "嵌套".to_string()),
        ]
    );
}

#[test]
fn entities_become_the_character_they_stand_for() {
    let rich = parse_rich("a &amp; b &#65; &#x4e2d; &nbsp;c");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("a & b A 中  c"));
}

#[test]
fn an_unknown_entity_and_a_bare_ampersand_stay_as_written() {
    let rich = parse_rich("&bogus; & &; 100% &amp");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("&bogus; & &; 100% &amp"));
}

#[test]
fn an_entity_inside_code_is_text() {
    let rich = parse_rich("`&amp;`");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("&amp;"));
}

#[test]
fn a_span_can_hold_a_backtick_of_its_own() {
    let rich = parse_rich("``a`b``");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("a`b"));
    assert_eq!(rich[0].spans, vec![span("a`b".len(), false, false, true)]);
}

#[test]
fn one_space_of_padding_comes_off_a_span() {
    let rich = parse_rich("`` `x` ``");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("`x`"));
}

#[test]
fn a_span_of_two_backticks_is_not_emphasis() {
    let rich = parse_rich("``*a*``");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("*a*"));
    assert_eq!(rich[0].spans, vec![span(3, false, false, true)]);
}

#[test]
fn a_run_of_backticks_with_no_partner_is_text() {
    let rich = parse_rich("a `` b");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("a `` b"));
}

#[test]
fn a_bare_www_address_is_a_link_with_a_scheme() {
    let rich = parse_rich("见 www.rust-lang.org 一节");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("见 www.rust-lang.org 一节"));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://www.rust-lang.org");
    // A word that merely starts with `w` is a word.
    assert_eq!(
        parse("was wir"),
        vec![Block::Paragraph("was wir".to_string())]
    );
}

#[test]
fn a_bracket_inside_an_address_stays_in_it() {
    let rich = parse_rich("https://en.wikipedia.org/wiki/A_(b)");
    check_runs(&rich);
    assert_eq!(rich[0].links[0].url, "https://en.wikipedia.org/wiki/A_(b)");
}

#[test]
fn a_bracket_around_an_address_belongs_to_the_sentence() {
    let rich = parse_rich("(见 https://example.com/a)");
    check_runs(&rich);
    assert_eq!(rich[0].links[0].url, "https://example.com/a");
}

#[test]
fn a_styling_tag_turns_its_run_on() {
    let rich = parse_rich("这是 <b>粗</b>、<i>斜</i> 和 <s>划掉</s>");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("这是 粗、斜 和 划掉"));
    assert!(rich[0].spans.iter().any(|run| run.bold));
    assert!(rich[0].spans.iter().any(|run| run.italic));
    assert!(rich[0].spans.iter().any(|run| run.strike));
    // `<kbd>` keeps its text and draws it in the code font.
    let rich = parse_rich("按 <kbd>Ctrl</kbd>+C");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("按 Ctrl+C"));
    assert!(rich[0].spans.iter().any(|run| run.code));
}

#[test]
fn more_inline_tags_style_their_run() {
    let rich = parse_rich("<u>下划线</u> 与 <code>mono</code>");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("下划线 与 mono"));
    assert!(rich[0].spans.iter().any(|span| span.underline));
    assert!(rich[0].spans.iter().any(|span| span.code));
    // The tags themselves never show, and a stray closer is harmless.
    let rich = parse_rich("<kbd>Ctrl</kbd></ins>");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("Ctrl"));
}

#[test]
fn html_sup_and_sub_become_script_characters() {
    assert_eq!(
        parse("x<sup>2</sup> + y"),
        vec![Block::Paragraph("x² + y".to_string())]
    );
    assert_eq!(
        parse("H<sub>2</sub>O"),
        vec![Block::Paragraph("H₂O".to_string())]
    );
    // Text with no Unicode script form stays as written, tags dropped.
    assert_eq!(
        parse("<sup>中文</sup>"),
        vec![Block::Paragraph("中文".to_string())]
    );
    // An unclosed tag is dropped and its text kept.
    assert_eq!(parse("<sup>2"), vec![Block::Paragraph("2".to_string())]);
}

#[test]
fn an_html_anchor_reads_as_a_link() {
    let rich = parse_rich("见 <a href=\"https://example.com/a\">这里</a>。");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("见 这里。"));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://example.com/a");
    // The label is its own run, so copying the answer puts the brackets back
    // around exactly it.
    let cell = styled(
        "见 <a href=\"https://example.com/a\">这里</a>。",
        &Definitions::default(),
    );
    assert!(cell.markdown().contains("[这里](https://example.com/a)"));
    // Single quotes and a stray closer are handled; only an address the
    // opener accepts becomes a link.
    let rich = parse_rich("<a href='https://a.b'>y</a></a>");
    check_runs(&rich);
    assert_eq!(rich[0].links.len(), 1);
    let rich = parse_rich("<a href=\"javascript:alert(1)\">x</a>");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("x"));
    assert!(rich[0].links.is_empty());
    // Never closed: no link is left behind.
    let rich = parse_rich("<a href=\"https://a.b\">y");
    check_runs(&rich);
    assert!(rich[0].links.is_empty());
}

#[test]
fn a_mark_tag_highlights_its_run() {
    let rich = parse_rich("重点是 <mark>这一句</mark>。");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("重点是 这一句。"));
    assert!(rich[0].spans.iter().any(|span| span.marked));
    // Only the marked run carries it.
    let marked: usize = rich[0]
        .spans
        .iter()
        .filter(|span| span.marked)
        .map(|span| span.len)
        .sum();
    assert_eq!(marked, "这一句".len());
}

#[test]
fn a_named_inline_image_takes_the_title_as_its_label() {
    let rich = parse_rich("见 ![](https://x/a.png \"示意图\")");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("见 示意图"));
    assert_eq!(rich[0].links.len(), 1);
    assert_eq!(rich[0].links[0].url, "https://x/a.png");
}

#[test]
fn a_mail_address_in_angle_brackets_opens_as_mail() {
    let rich = parse_rich("联系 <user@example.com> 即可");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("联系 user@example.com 即可"));
    assert_eq!(rich[0].links[0].url, "mailto:user@example.com");
    // A bare address is not a link: `@` is too easy to hit by accident.
    let rich = parse_rich("联系 user@example.com 即可");
    assert!(rich[0].links.is_empty());
    // Something that is not an address keeps its angle brackets.
    let rich = parse_rich("<不是地址>");
    assert!(rich[0].links.is_empty());
    assert_eq!(body_of(&rich[0].block), Some("<不是地址>"));
}

#[test]
fn a_reference_link_resolves_to_its_definition() {
    let rich = parse_rich(
        "看 [文档][1] 与 [另一份][]。\n\n[1]: https://example.com/a\n[另一份]: https://example.com/b \"标题\"",
    );
    check_runs(&rich);
    // The definitions themselves draw nothing.
    assert_eq!(rich.len(), 1);
    assert_eq!(body_of(&rich[0].block), Some("看 文档 与 另一份。"));
    assert_eq!(rich[0].links.len(), 2);
    assert_eq!(rich[0].links[0].url, "https://example.com/a");
    assert_eq!(rich[0].links[1].url, "https://example.com/b");
}
