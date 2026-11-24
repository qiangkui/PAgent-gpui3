//! Formulas: LaTeX text, display blocks, matrices, binomials and roots.

use super::*;

#[test]
fn an_inline_formula_becomes_text_in_the_code_font() {
    let rich = parse_rich("勾股定理：$a^2 + b^2 = c^2$。");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("勾股定理：a² + b² = c²。"));
    assert!(rich[0].spans.iter().any(|span| span.code));
    // Prices are not formulas: the closing `$` has to touch the text.
    let rich = parse_rich("一共 $5 和 $10");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("一共 $5 和 $10"));
}

#[test]
fn a_double_dollar_formula_inside_a_sentence_is_still_maths() {
    // A display marker shares a line with the sentence: its `$$` still opens and
    // closes one formula, and no stray `$` is left behind.
    let rich = parse_rich("结果是 $$x^2$$ 就这样");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("结果是 x² 就这样"));
    assert!(rich[0].spans.iter().any(|span| span.code));
    // One that never closes stays as written.
    let rich = parse_rich("结果是 $$x^2 就这样");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("结果是 $$x^2 就这样"));
}

#[test]
fn a_display_formula_is_its_own_block() {
    let blocks = parse("前文。\n\n$$ \\sum_{i=1}^{n} i = \\frac{n(n+1)}{2} $$\n\n后文。");
    assert_eq!(
        blocks,
        vec![
            Block::Paragraph("前文。".to_string()),
            Block::Math(Formula::new(r"\sum_{i=1}^{n} i = \frac{n(n+1)}{2}")),
            Block::Paragraph("后文。".to_string()),
        ]
    );
}

#[test]
fn a_paren_delimited_formula_reads_as_maths() {
    // `\(…\)` is inline maths, the same as `$…$`.
    let rich = parse_rich("写成 \\(x^2\\) 也可以。");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("写成 x² 也可以。"));
    assert!(rich[0].spans.iter().any(|span| span.code));
    // An unclosed or empty one is not a formula: the bracket the escape
    // used to draw is kept.
    let rich = parse_rich("单独 \\( 一个");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("单独 ( 一个"));
    let rich = parse_rich("空 \\(\\) 呀");
    check_runs(&rich);
    assert_eq!(body_of(&rich[0].block), Some("空 () 呀"));
}

#[test]
fn a_bracket_display_formula_is_its_own_block() {
    assert_eq!(
        parse("前文。\n\n\\[ \\frac{a}{b} \\]\n\n后文。"),
        vec![
            Block::Paragraph("前文。".to_string()),
            Block::Math(Formula::new(r"\frac{a}{b}")),
            Block::Paragraph("后文。".to_string()),
        ]
    );
    // Inside a fence both markers are code, not a formula.
    assert_eq!(
        parse("```\n\\[ x \\]\n$$\n```"),
        vec![Block::Code(String::new(), "\\[ x \\]\n$$".to_string())]
    );
}

#[test]
fn more_commands_and_symbols_are_understood() {
    assert_eq!(latex(r"\overline{x}"), "x\u{0304}");
    assert_eq!(latex(r"\operatorname{sgn} x"), "sgn x");
    // Style switches and sizing commands draw nothing of their own, and take
    // the space that separates them from the formula.
    assert_eq!(latex(r"\displaystyle \sum_{i=1}^{n}"), "∑ᵢ₌₁ⁿ");
    assert_eq!(latex(r"\left( x \right)"), "( x )");
    assert_eq!(latex(r"\langle x, y \rangle"), "⟨ x, y ⟩");
    assert_eq!(latex(r"\forall \epsilon > 0"), "∀ ε > 0");
    assert_eq!(latex(r"\therefore x \implies y"), "∴ x ⟹ y");
    assert_eq!(latex(r"a \mapsto b"), "a ↦ b");
}

#[test]
fn braces_and_frames_are_drawn_nodes() {
    // A brace has no character of its own, so a rule stands in for it.
    let formula = Formula::new(r"\overbrace{a+b}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Overline(
            Box::new(MathNode::Text("a+b".to_string())),
            true
        )]
    );
    let formula = Formula::new(r"\underbrace{x}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Overline(
            Box::new(MathNode::Text("x".to_string())),
            false
        )]
    );
    // `\boxed` draws a frame round the piece.
    let formula = Formula::new(r"\boxed{x^2}");
    assert!(matches!(formula.nodes.as_slice(), [MathNode::Framed(_)]));
    assert!(formula.stacked());
    assert_eq!(latex(r"\boxed{x^2}"), "x²");
}

#[test]
fn equation_numbers_and_cancels_keep_only_their_content() {
    // `\tag` is a number for a typesetter: nothing of it is drawn.
    assert_eq!(latex(r"x = 1 \tag{7}").trim(), "x = 1");
    assert!(!latex(r"\tag{1} x").contains("tag"));
    // A cancel and a substack only group their content.
    assert_eq!(latex(r"\cancel{x}"), "x");
    assert_eq!(latex(r"\sout{x} + \bcancel{y}"), "x + y");
    assert!(!latex(r"\substack{a \\ b}").contains('\\'));
    // The node tree has no raw command either.
    let formula = Formula::new(r"\cancel{x^2}");
    assert!(matches!(formula.nodes.as_slice(), [MathNode::Sup(..)]));
}

#[test]
fn a_function_word_is_one_piece_of_the_formula() {
    // `\sin^2` raises the name, not its last letter.
    let formula = Formula::new(r"\sin^2 x");
    assert_eq!(
        formula.nodes,
        vec![
            MathNode::Sup(
                Box::new(MathNode::Text("sin".to_string())),
                Box::new(MathNode::Text("2".to_string())),
            ),
            MathNode::Text(" x".to_string()),
        ]
    );
    // `\lim_{x \to 0}` keeps the limit under the whole word.
    let formula = Formula::new(r"\lim_{x \to 0} f(x)");
    assert_eq!(
        formula.nodes[0],
        MathNode::Sub(
            Box::new(MathNode::Text("lim".to_string())),
            Box::new(MathNode::Text("x → 0".to_string())),
        )
    );
    assert_eq!(latex(r"\lim_{x \to 0} f(x)"), "lim_(x → 0) f(x)");
}

#[test]
fn more_function_names_carry_their_limits() {
    let formula = Formula::new(r"\argmax_{x} f(x)");
    assert_eq!(
        formula.nodes[0],
        MathNode::Sub(
            Box::new(MathNode::Text("argmax".to_string())),
            Box::new(MathNode::Text("x".to_string())),
        )
    );
    assert_eq!(latex(r"\limsup_{n} a_n"), "lim supₙ aₙ");
    assert_eq!(latex(r"\liminf_{n} a_n"), "lim infₙ aₙ");
    assert_eq!(latex(r"\Pr(A)"), "Pr(A)");
    assert_eq!(latex(r"\gcd(a, b)"), "gcd(a, b)");
}

#[test]
fn layout_commands_leave_no_backslashes_behind() {
    // `\left` / `\right` are the brackets' business, and `\,` is a space: the
    // command letter and the punctuation after it both belong to the command.
    assert_eq!(latex(r"\left( \frac{a}{b} \right)"), "( a/b )");
    assert_eq!(latex(r"e^{-x^2}\,dx"), "e^(-x²) dx");
    assert_eq!(latex(r"a \quad b"), "a   b");
    assert_eq!(latex(r"50\% \$5"), "50% $5");
    // The node tree must not keep `\left(` in its text either.
    let formula = Formula::new(r"\left( \frac{a}{b} \right)");
    assert!(
        !format!("{:?}", formula.nodes).contains("left"),
        "{:?}",
        formula.nodes
    );
}

#[test]
fn colours_braces_and_bold_keep_only_their_content() {
    assert_eq!(latex(r"\color{red}{x}"), "x");
    assert_eq!(latex(r"\textcolor{red}{y+1}"), "y+1");
    assert_eq!(latex(r"\overbrace{1+2}_{3}"), "1+2₃");
    assert_eq!(latex(r"\underbrace{1+2}_{3}"), "1+2₃");
    assert_eq!(latex(r"\boldsymbol{\alpha}"), "α");
    assert_eq!(latex(r"\boxed{x^2}"), "x²");
    // A command inside a fraction is converted on both sides.
    assert_eq!(latex(r"\frac{\partial f}{\partial x}"), "(∂ f)/(∂ x)");
    assert_eq!(latex(r"\sqrt{x^2 + y^2}"), "√(x² + y²)");
    // Number sets are blackboard letters in the node tree as well.
    let formula = Formula::new(r"\mathbb{R}^n");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Sup(
            Box::new(MathNode::Text("ℝ".to_string())),
            Box::new(MathNode::Text("n".to_string())),
        )]
    );
}

#[test]
fn latex_symbols_become_unicode() {
    assert_eq!(latex(r"\alpha \times \beta \le \infty"), "α × β ≤ ∞");
    assert_eq!(latex(r"\frac{1}{2}"), "1/2");
    assert_eq!(latex(r"\frac{a+b}{2}"), "(a+b)/2");
    assert_eq!(latex(r"\frac{n(n+1)}{2}"), "(n(n+1))/2");
    assert_eq!(latex(r"\sqrt{x}"), "√x");
    assert_eq!(latex(r"\sqrt[3]{8}"), "∛8");
    assert_eq!(latex("x^2 + y_1"), "x² + y₁");
    // A whole group of scripts when every character has a Unicode form.
    assert_eq!(latex(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ");
    // Words, accents and number sets.
    assert_eq!(latex(r"\lim_{x \to 0} \log x"), "lim_(x → 0) log x");
    assert_eq!(latex(r"\vec{v} \cdot \bar{x}"), "v⃗ · x̄");
    assert_eq!(latex(r"\mathbb{R}^n"), "ℝⁿ");
    // An unknown command keeps its backslash instead of vanishing.
    assert_eq!(latex(r"\unknown"), r"\unknown");
}

#[test]
fn a_math_fence_is_drawn_as_a_formula() {
    assert_eq!(
        parse("```math\n\\frac{1}{2}\n```"),
        vec![Block::Math(Formula::new(r"\frac{1}{2}"))]
    );
    assert_eq!(
        parse("```latex\na^2 + b^2\n```"),
        vec![Block::Math(Formula::new("a^2 + b^2"))]
    );
    // Other languages are code, as before.
    assert_eq!(
        parse("```rust\nlet a = 1;\n```"),
        vec![Block::Code("rust".to_string(), "let a = 1;".to_string())]
    );
    // A fence that never closes (a streaming answer) still draws.
    let blocks = parse("```math\nx^2");
    assert!(matches!(&blocks[0], Block::Math(_)));
}

#[test]
fn a_double_backslash_starts_a_new_line() {
    assert_eq!(latex(r"a\\b"), "a\nb");
    let formula = Formula::new(r"a \\ \frac{b}{c}");
    assert_eq!(
        formula
            .nodes
            .iter()
            .filter(|node| matches!(node, MathNode::Break))
            .count(),
        1
    );
    assert!(formula.stacked(), "the fraction still needs laying out");
}

#[test]
fn a_binomial_and_an_overline_are_nodes_of_their_own() {
    let formula = Formula::new(r"\binom{n}{k}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Binom(
            Box::new(MathNode::Text("n".to_string())),
            Box::new(MathNode::Text("k".to_string())),
        )]
    );
    assert!(formula.stacked());
    assert_eq!(latex(r"\binom{n}{k}"), "C(n, k)");

    // A rule over a whole expression: a combining mark would only sit on the
    // last character, so the piece is kept as a node of its own.
    let formula = Formula::new(r"\overline{AB}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Overline(
            Box::new(MathNode::Text("AB".to_string())),
            true
        )]
    );
    let formula = Formula::new(r"\underline{x+1}");
    let [MathNode::Overline(body, above)] = &formula.nodes[..] else {
        panic!("expected an overline node");
    };
    assert!(!*above);
    assert!(matches!(body.as_ref(), MathNode::Text(text) if text == "x+1"));
}

#[test]
fn a_matrix_environment_parses_into_rows_and_cells() {
    let formula = Formula::new(r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Matrix(
            vec![
                vec![
                    vec![MathNode::Text("1".to_string())],
                    vec![MathNode::Text("2".to_string())],
                ],
                vec![
                    vec![MathNode::Text("3".to_string())],
                    vec![MathNode::Text("4".to_string())],
                ],
            ],
            Some(Bracket::Paren),
        )]
    );
    // A grid is something a line of text cannot say, so it is stacked.
    assert!(formula.stacked());
    // An `array` draws no bracket, and its column spec is not a cell.
    let formula = Formula::new(r"\begin{array}{cc} a & b \\ c & d \end{array}");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Matrix(
            vec![
                vec![
                    vec![MathNode::Text("a".to_string())],
                    vec![MathNode::Text("b".to_string())],
                ],
                vec![
                    vec![MathNode::Text("c".to_string())],
                    vec![MathNode::Text("d".to_string())],
                ],
            ],
            None,
        )]
    );
    // A cell keeps a fraction of its own.
    let formula = Formula::new(r"\begin{pmatrix} \frac{1}{2} \\ 1 \end{pmatrix}");
    let [MathNode::Matrix(rows, _)] = &formula.nodes[..] else {
        panic!("expected one matrix, got {:?}", formula.nodes);
    };
    assert!(matches!(rows[0][0][0], MathNode::Frac(..)));
}

#[test]
fn a_matrix_environment_lays_its_rows_out_as_text() {
    assert_eq!(
        latex(r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}"),
        "(1 2; 3 4)"
    );
    assert_eq!(
        latex(r"\begin{cases} x & x > 0 \\ -x & x \le 0 \end{cases}"),
        "{x x > 0; -x x ≤ 0}"
    );
}

#[test]
fn a_fraction_is_stacked_and_a_plain_formula_is_not() {
    let formula = Formula::new(r"\frac{a}{b}");
    assert!(formula.stacked());
    assert_eq!(formula.text, "a/b");
    assert_eq!(
        formula.nodes,
        vec![MathNode::Frac(
            Box::new(MathNode::Text("a".to_string())),
            Box::new(MathNode::Text("b".to_string())),
        )]
    );
    // A script is worth laying out too; symbols alone are not.
    assert!(Formula::new("a^2 + b^2").stacked());
    let formula = Formula::new(r"\alpha + \beta");
    assert!(!formula.stacked());
    assert_eq!(formula.text, "α + β");
}

#[test]
fn scripts_hang_off_the_character_before_them() {
    assert_eq!(
        Formula::new("x^2").nodes,
        vec![MathNode::Sup(
            Box::new(MathNode::Text("x".to_string())),
            Box::new(MathNode::Text("2".to_string())),
        )]
    );
    // Only the character just before the marker is the base.
    assert_eq!(
        Formula::new("ab_1").nodes,
        vec![
            MathNode::Text("a".to_string()),
            MathNode::Sub(
                Box::new(MathNode::Text("b".to_string())),
                Box::new(MathNode::Text("1".to_string())),
            ),
        ]
    );
}

#[test]
fn a_root_keeps_its_index_and_its_body() {
    assert_eq!(
        Formula::new(r"\sqrt[3]{x+1}").nodes,
        vec![MathNode::Sqrt(
            Some(Box::new(MathNode::Text("3".to_string()))),
            Box::new(MathNode::Text("x+1".to_string())),
        )]
    );
}

#[test]
fn a_group_keeps_the_pieces_it_holds() {
    // The exponents inside the root survive instead of collapsing to text.
    let formula = Formula::new(r"\sqrt{x^2 + y^2}");
    let [MathNode::Sqrt(_, body)] = &formula.nodes[..] else {
        panic!("expected a root");
    };
    let MathNode::Row(pieces) = body.as_ref() else {
        panic!("expected the pieces to stay apart");
    };
    assert_eq!(pieces.len(), 3);
    assert!(matches!(pieces[0], MathNode::Sup(..)));
    assert!(matches!(pieces[2], MathNode::Sup(..)));
    // A fraction inside a fraction stays a fraction.
    let formula = Formula::new(r"\frac{\frac{a}{b}}{2}");
    let [MathNode::Frac(first, second)] = &formula.nodes[..] else {
        panic!("expected a fraction");
    };
    assert!(matches!(first.as_ref(), MathNode::Frac(..)));
    assert_eq!(second.as_ref(), &MathNode::Text("2".to_string()));
}

#[test]
fn a_sum_keeps_its_limits_together() {
    // Rendering puts these above and below the symbol; the parse keeps the
    // shape `Sup(Sub(∑, i=1), n)`.
    let formula = Formula::new(r"\sum_{i=1}^{n}");
    let [MathNode::Sup(base, power)] = &formula.nodes[..] else {
        panic!("expected a superscript");
    };
    assert!(matches!(base.as_ref(), MathNode::Sub(..)));
    assert_eq!(power.as_ref(), &MathNode::Text("n".to_string()));
}
