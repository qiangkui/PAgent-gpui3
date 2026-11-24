//! Small end-to-end checks that do not belong to one part.

use super::*;

#[test]
fn a_leading_front_matter_block_draws_nothing() {
    assert_eq!(
        parse("---\ntitle: 报告\nauthor: me\n---\n正文"),
        vec![Block::Paragraph("正文".to_string())]
    );
    // A block of nothing but dashes is two rules, not front matter.
    assert_eq!(parse("---\n---"), vec![Block::Rule, Block::Rule]);
    // Text that is not `key: value` means the dashes are a rule.
    assert_eq!(
        parse("---\n第一段\n---"),
        vec![Block::Rule, Block::Heading(2, "第一段".to_string()),]
    );
    // Front matter is only front matter at the very top.
    assert_eq!(
        parse("前言\n\n---\ntitle: x\n---"),
        vec![
            Block::Paragraph("前言".to_string()),
            Block::Rule,
            Block::Heading(2, "title: x".to_string()),
        ]
    );
}

#[test]
fn a_definition_list_keeps_its_term_and_description() {
    let rich = parse_rich("术语\n: 说明一\n: 说明二");
    check_runs(&rich);
    // The term becomes a bold line, and each description is an entry of its own
    // under it.
    assert_eq!(rich[0].block, Block::Definition("术语\n说明一".to_string()));
    assert!(rich[0].spans[0].bold);
    assert_eq!(rich[1].block, Block::Definition("说明二".to_string()));
    // A term with emphasis and a description with a link both survive.
    let rich = parse_rich("**重点**\n: 见[文档](https://example.com)");
    check_runs(&rich);
    assert_eq!(rich[0].links.len(), 1);
    // A wrapped description runs on like a list item.
    assert_eq!(
        parse("术语\n: 第一行\n  接着写"),
        vec![Block::Definition("术语\n第一行接着写".to_string())]
    );
}

#[test]
fn a_colon_line_with_nothing_above_it_is_text() {
    // A CSS pseudo-class and a time are not definitions.
    assert_eq!(
        parse(":hover"),
        vec![Block::Paragraph(":hover".to_string())]
    );
    assert_eq!(
        parse("12:30 开会"),
        vec![Block::Paragraph("12:30 开会".to_string())]
    );
    // A definition ends when an unindented line follows it.
    assert_eq!(
        parse("术语\n: 说明\n\n正文"),
        vec![
            Block::Definition("术语\n说明".to_string()),
            Block::Paragraph("正文".to_string()),
        ]
    );
}

#[test]
fn a_definition_term_loses_its_indentation() {
    // The line above may be indented (it is still text, not a nested marker),
    // and the definition draws the term without that indentation.
    assert_eq!(
        parse("- 项\n\n  术语\n  : 说明"),
        vec![
            Block::Bullet(0, "项".to_string()),
            Block::Definition("术语\n说明".to_string()),
        ]
    );
}

#[test]
fn every_block_run_walks_its_own_text() {
    let text = "# `a` **b**\n- `x`\n1. **y**\n> 引用 `z`\n正文 **粗** 与 `码`\n\
                ```\ncode\n```\n| a | b |\n|---|---|\n| `c` | **d** |";
    let rich = parse_rich(text);
    check_runs(&rich);
    assert_eq!(rich.len(), 7);
}
