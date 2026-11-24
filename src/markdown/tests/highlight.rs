//! Code-block tokenising: languages, comments, strings, regexes and token kinds.

use super::*;

#[test]
fn four_spaces_make_a_code_block() {
    assert_eq!(
        parse("看：\n\n    const a = 1;\n    if (a) {}\n\n结束"),
        vec![
            Block::Paragraph("看：".to_string()),
            Block::Code(String::new(), "const a = 1;\nif (a) {}".to_string()),
            Block::Paragraph("结束".to_string()),
        ]
    );
    // A tab indents too.
    assert_eq!(
        parse("\n\techo hi\n"),
        vec![Block::Code(String::new(), "echo hi".to_string())]
    );
    // An indented block cannot interrupt a paragraph; the line is text.
    assert_eq!(
        parse("看：\n    const a = 1;"),
        vec![Block::Paragraph("看： const a = 1;".to_string())]
    );
    // An indented list item is still a list item; four spaces nest once, like
    // any other indent deeper than the item above.
    assert_eq!(
        parse("- 一\n    - 二"),
        vec![
            Block::Bullet(0, "一".to_string()),
            Block::Bullet(1, "二".to_string()),
        ]
    );
    // So is an indented heading.
    assert_eq!(
        parse("正文\n\n    ## 缩进的标题"),
        vec![
            Block::Paragraph("正文".to_string()),
            Block::Heading(2, "缩进的标题".to_string()),
        ]
    );
}

#[test]
fn three_stars_make_a_bold_italic_run() {
    let rich = parse_rich("看 ***重点*** 和 **粗体**");
    assert_eq!(
        rich[0].block,
        Block::Paragraph("看 重点 和 粗体".to_string())
    );
    assert_eq!(
        rich[0].spans,
        vec![
            span("看 ".len(), false, false, false),
            span("重点".len(), true, true, false),
            span(" 和 ".len(), false, false, false),
            span("粗体".len(), true, false, false),
        ]
    );
}

#[test]
fn single_stars_make_an_italic_run() {
    let rich = parse_rich("前 *重点* 后");
    assert_eq!(rich[0].block, Block::Paragraph("前 重点 后".to_string()));
    let slanted: Vec<&Span> = rich[0].spans.iter().filter(|span| span.italic).collect();
    assert_eq!(slanted.len(), 1);
    assert_eq!(slanted[0].len, "重点".len());
    assert!(!slanted[0].bold);
}

#[test]
fn a_fence_label_is_its_first_word() {
    assert_eq!(
        parse("```rust title=\"x\"\na\n```"),
        vec![Block::Code("rust".to_string(), "a".to_string())]
    );
    assert_eq!(
        parse("```{ts}\na\n```"),
        vec![Block::Code("ts".to_string(), "a".to_string())]
    );
}

#[test]
fn a_code_block_is_tokenised_for_its_language() {
    let body = "let a = 1; // 注释\nfn f() { \"文本\" }";
    let spans = highlight("rust", body);
    // The runs cover the block byte for byte, or the view falls back to one.
    assert_eq!(spans.iter().map(|span| span.len).sum::<usize>(), body.len());
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));
    assert!(spans.iter().any(|span| span.token == Some(Token::String)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Number)));
    // A fenced block carries them into the answer.
    let rich = parse_rich("```rust\nlet a = 1;\n```");
    assert!(
        rich[0]
            .spans
            .iter()
            .any(|span| span.token == Some(Token::Keyword))
    );
}

#[test]
fn python_comments_and_strings_are_told_apart() {
    let body = "def f(x):\n    return \"#不是注释\"  # 注释";
    let spans = highlight("python", body);
    assert_eq!(spans.iter().map(|span| span.len).sum::<usize>(), body.len());
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.token == Some(Token::Comment))
            .count(),
        1
    );
    // The `#` inside the string is not a comment.
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.token == Some(Token::String))
            .count(),
        1
    );
}

#[test]
fn more_languages_are_tokenised() {
    // C: `//` comment, keyword, number and string.
    let body = "int main() { return 0; } // 注释";
    let spans = highlight("c", body);
    assert_eq!(spans.iter().map(|span| span.len).sum::<usize>(), body.len());
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Number)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));

    // Java, C# and Kotlin share the `//` comment style.
    for language in ["java", "cs", "kotlin", "swift", "dart", "scala"] {
        let spans = highlight(language, "class A {} // x");
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Comment)),
            "{language} comments"
        );
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Keyword)),
            "{language} keywords"
        );
    }

    // Ruby and R use `#`; Lua and Haskell use `--`; HTML uses `<!-- -->`.
    for (language, body) in [
        ("ruby", "def f # x"),
        ("r", "function(x) 1 # x"),
        ("lua", "local x = 1 -- x"),
        ("haskell", "f x = let y = x in y -- x"),
    ] {
        let spans = highlight(language, body);
        assert_eq!(
            spans.iter().map(|span| span.len).sum::<usize>(),
            body.len(),
            "{language} covers its text"
        );
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Comment)),
            "{language} comments"
        );
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Keyword)),
            "{language} keywords"
        );
    }
    let spans = highlight("html", "<!-- 注释 --><p>hi</p>");
    assert_eq!(
        spans
            .iter()
            .filter(|span| span.token == Some(Token::Comment))
            .count(),
        1
    );

    // The aliases an answer is likely to use.
    assert!(Language::of("c++").is_some());
    assert!(Language::of("yml").is_some());
    assert!(Language::of("xml").is_some());
    assert!(Language::of("scss").is_some());
}

#[test]
fn nested_block_comments_close_at_their_own_depth() {
    let body = "/* a /* b */ c */ let x = 1;";
    let spans = highlight("rust", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "/* a /* b */ c */");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));

    // C does not nest: the first `*/` closes it.
    let body = "/* a /* b */ int x;";
    let spans = highlight("c", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "/* a /* b */");
}

#[test]
fn a_javascript_regex_reads_as_one_token() {
    let body = "const r = /ab+c/gi;";
    let spans = highlight("ts", body);
    let string = spans
        .iter()
        .find(|span| span.token == Some(Token::String))
        .expect("a regex token");
    assert_eq!(string.len, "/ab+c/gi".len());
    // Division is not a regex.
    let spans = highlight("ts", "const q = a / b;");
    assert!(!spans.iter().any(|span| span.token == Some(Token::String)));
    // `return /re/` still is.
    let spans = highlight("ts", "return /x/;");
    assert!(spans.iter().any(|span| span.token == Some(Token::String)));
    // Only JS/TS get this reading.
    let spans = highlight("ruby", "r = /x/");
    assert!(!spans.iter().any(|span| span.token == Some(Token::String)));
}

#[test]
fn powershell_docker_nix_and_julia_are_tokenised() {
    let body = "function Get-X { $n = 1 } # 注释";
    let spans = highlight("powershell", body);
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));

    // PowerShell's `<# … #>` block comment.
    let body = "<# 说明 #> if ($x) { }";
    let spans = highlight("pwsh", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "<# 说明 #>");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));

    // Dockerfile instructions are matched regardless of case.
    let spans = highlight("dockerfile", "FROM rust:1 AS build");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));

    // Nix: `#` and `/* */` are comments, but `//` is an operator.
    let spans = highlight("nix", "let a = 1; # c");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));
    assert!(
        !highlight("nix", "a // b")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );
    let body = "/* b */ in";
    let spans = highlight("nix", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "/* b */");

    // Julia's `#= … =#` comments nest.
    let body = "#= a #= b =# c =# function f() end";
    let spans = highlight("julia", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "#= a #= b =# c =#");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
}

#[test]
fn a_triple_quoted_string_holds_its_body() {
    let body = "x = \"\"\"a # not a comment\"\"\"\ny = 1 # 注释";
    let spans = highlight("python", body);
    let strings: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::String))
        .map(|span| span.len)
        .sum();
    assert_eq!(strings, "\"\"\"a # not a comment\"\"\"".len());
    let comments: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[body.len() - comments..], "# 注释");
}

#[test]
fn settings_batch_perl_cmake_and_matlab_are_tokenised() {
    // An `.ini` has `#` comments but no keywords: colouring every key would be
    // noise.
    let spans = highlight("ini", "[main]\nkey = value # 注释");
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));
    assert!(!spans.iter().any(|span| span.token == Some(Token::Keyword)));
    // Batch: `::` and `rem` comments, `if` a keyword.
    assert!(
        highlight("bat", ":: 注释\nrem 也注释\nif exist x echo 1")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );
    let body = ":: a\nrem b";
    let comments: usize = highlight("cmd", body)
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    // Both lines are comments; the line break between them is not.
    assert_eq!(comments, body.len() - 1);
    // A name that starts with `rem` is not a comment.
    assert!(
        !highlight("bat", "set remote=1")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );
    // Perl, CMake and MATLAB: `#` / `%` comments with keywords.
    for (language, body) in [
        ("perl", "my $x = 1; # c"),
        ("pl", "use strict; # c"),
        ("cmake", "add_executable(x a.c) # c"),
        ("matlab", "if x % c\nend"),
        ("octave", "function y = f(x) % c"),
    ] {
        let spans = highlight(language, body);
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Comment)),
            "{language} comment"
        );
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Keyword)),
            "{language} keyword"
        );
    }
}

#[test]
fn a_diff_colours_whole_lines_by_what_they_do() {
    let body = "@@ -1 +1 @@\n-旧行\n+新行\n 同一行\n+++ b/x\n--- a/x\n";
    let spans = highlight("diff", body);
    // Every byte of the body is covered by exactly one run.
    let covered: usize = spans.iter().map(|span| span.len).sum();
    assert_eq!(covered, body.len());
    let removed: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Removed))
        .map(|span| span.len)
        .sum();
    let added: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Added))
        .map(|span| span.len)
        .sum();
    let keywords: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Keyword))
        .map(|span| span.len)
        .sum();
    assert_eq!(removed, "-旧行\n".len());
    assert_eq!(added, "+新行\n".len());
    // The hunk line and the two file headers are the diff's own syntax.
    assert_eq!(
        keywords,
        "@@ -1 +1 @@\n".len() + "+++ b/x\n".len() + "--- a/x\n".len()
    );
    // The context line stays plain.
    assert!(
        spans
            .iter()
            .any(|span| span.token.is_none() && span.len >= " 同一行\n".len())
    );
}

#[test]
fn a_diff_header_is_not_a_removed_line() {
    // `---` starts a file header in a diff, but a lone `-` line removes one.
    let spans = highlight("patch", "--- a/x\n+only\n");
    assert!(!spans.iter().any(|span| span.token == Some(Token::Removed)));
    let spans = highlight("diff", "-only\n");
    assert!(spans.iter().any(|span| span.token == Some(Token::Removed)));
}

#[test]
fn calls_and_type_names_get_their_own_tokens() {
    let spans = highlight("rust", "let v: Vec<String> = make(1);");
    assert!(spans.iter().any(|span| span.token == Some(Token::Type)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Function)));
    // Constants and plain names stay plain.
    let spans = highlight("rust", "const MAX_SIZE = 10;");
    assert!(!spans.iter().any(|span| span.token == Some(Token::Type)));
    // Only a `(` after the name makes it a call.
    assert!(
        !highlight("rust", "let x = vec![];")
            .iter()
            .any(|span| span.token == Some(Token::Function))
    );
}

#[test]
fn make_graphql_proto_zig_and_solidity_are_tokenised() {
    for (language, body) in [
        ("makefile", "ifeq ($(A),1) # c"),
        ("graphql", "query Q { field } # c"),
        ("proto", "message M { // c"),
        ("solidity", "contract C { // c"),
        ("hcl", "resource \"x\" \"y\" { # c"),
    ] {
        let spans = highlight(language, body);
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Comment)),
            "{language} comment"
        );
        assert!(
            spans.iter().any(|span| span.token == Some(Token::Keyword)),
            "{language} keyword"
        );
    }
    // Zig has `//` but no `/* */`.
    let body = "// c";
    let spans = highlight("zig", body);
    let comments: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(comments, body.len());
    assert!(
        !highlight("zig", "/* not a comment */")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );
}

#[test]
fn ocaml_fsharp_clojure_and_vim_are_tokenised() {
    // OCaml / F#: `(* … *)` nests; F# also has `//`.
    let body = "(* a (* b *) c *) let x = 1";
    let spans = highlight("ocaml", body);
    let comment: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[..comment], "(* a (* b *) c *)");
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
    assert!(
        !highlight("ocaml", "/* not a comment */")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );
    assert!(
        highlight("fs", "// c\nlet x = 1")
            .iter()
            .any(|span| span.token == Some(Token::Comment))
    );

    // Clojure: `;` comments.
    let spans = highlight("clj", "(def x 1) ; c");
    assert!(spans.iter().any(|span| span.token == Some(Token::Comment)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));

    // Vim: `"` starts a comment, while a quote inside a string is text.
    let body = "echo 'a \" b' \" comment";
    let spans = highlight("vim", body);
    let comments: usize = spans
        .iter()
        .filter(|span| span.token == Some(Token::Comment))
        .map(|span| span.len)
        .sum();
    assert_eq!(&body[body.len() - comments..], "\" comment");
    assert!(spans.iter().any(|span| span.token == Some(Token::String)));
    assert!(spans.iter().any(|span| span.token == Some(Token::Keyword)));
}

#[test]
fn an_unknown_language_is_left_plain() {
    assert!(highlight("brainfuck", "+++ [ > - < ]").is_empty());
    assert!(parse_rich("```brainfuck\n+++\n```")[0].spans.is_empty());
}
