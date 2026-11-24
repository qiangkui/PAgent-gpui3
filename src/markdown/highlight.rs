//! The tokeniser behind coloured code blocks: languages, comments, strings, numbers and keywords.

use super::*;

/// The languages a code block is tokenised for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Rust,
    TypeScript,
    Python,
    Shell,
    Go,
    Sql,
    Json,
    C,
    Cpp,
    Java,
    CSharp,
    Kotlin,
    Swift,
    Ruby,
    Php,
    Lua,
    Yaml,
    Toml,
    Html,
    Css,
    Dart,
    Scala,
    R,
    Haskell,
    Elixir,
    PowerShell,
    Dockerfile,
    Nix,
    Julia,
    Makefile,
    GraphQL,
    Protobuf,
    Zig,
    Solidity,
    Hcl,
    OCaml,
    FSharp,
    Clojure,
    Vim,
    Diff,
    Ini,
    Batch,
    Perl,
    CMake,
    Matlab,
}

impl Language {
    /// The language a fence's info string names, `None` for anything else.
    pub(crate) fn of(name: &str) -> Option<Self> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "rust" | "rs" => Self::Rust,
            "ts" | "tsx" | "js" | "jsx" | "javascript" | "typescript" => Self::TypeScript,
            "py" | "python" => Self::Python,
            "sh" | "bash" | "zsh" | "shell" | "console" => Self::Shell,
            "go" | "golang" => Self::Go,
            "sql" => Self::Sql,
            "json" | "jsonc" => Self::Json,
            "c" | "h" => Self::C,
            "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hxx" => Self::Cpp,
            "java" => Self::Java,
            "cs" | "csharp" | "c#" => Self::CSharp,
            "kt" | "kts" | "kotlin" => Self::Kotlin,
            "swift" => Self::Swift,
            "rb" | "ruby" => Self::Ruby,
            "php" => Self::Php,
            "lua" => Self::Lua,
            "yml" | "yaml" => Self::Yaml,
            "toml" => Self::Toml,
            "html" | "htm" | "xml" | "svg" | "vue" => Self::Html,
            "css" | "scss" | "less" => Self::Css,
            "dart" => Self::Dart,
            "scala" | "sc" => Self::Scala,
            "r" => Self::R,
            "hs" | "haskell" => Self::Haskell,
            "ex" | "exs" | "elixir" => Self::Elixir,
            "ps1" | "pwsh" | "powershell" => Self::PowerShell,
            "dockerfile" | "docker" => Self::Dockerfile,
            "nix" => Self::Nix,
            "jl" | "julia" => Self::Julia,
            "makefile" | "make" | "mk" => Self::Makefile,
            "graphql" | "gql" => Self::GraphQL,
            "proto" | "protobuf" => Self::Protobuf,
            "zig" => Self::Zig,
            "sol" | "solidity" => Self::Solidity,
            "hcl" | "tf" | "terraform" => Self::Hcl,
            "ml" | "ocaml" => Self::OCaml,
            "fs" | "fsharp" | "f#" => Self::FSharp,
            "clj" | "cljs" | "clojure" | "edn" => Self::Clojure,
            "vim" | "viml" | "vimscript" => Self::Vim,
            "diff" | "patch" | "udiff" => Self::Diff,
            "ini" | "cfg" | "conf" | "properties" | "env" | "editorconfig" => Self::Ini,
            "bat" | "cmd" | "batch" => Self::Batch,
            "pl" | "pm" | "perl" => Self::Perl,
            "cmake" => Self::CMake,
            "matlab" | "octave" => Self::Matlab,
            _ => return None,
        })
    }

    /// Whether the language writes `#` comments.
    fn hash_comments(self) -> bool {
        matches!(
            self,
            Self::Python
                | Self::Shell
                | Self::Ruby
                | Self::R
                | Self::Yaml
                | Self::Toml
                | Self::Elixir
                | Self::Php
                | Self::PowerShell
                | Self::Dockerfile
                | Self::Nix
                | Self::Julia
                | Self::Makefile
                | Self::GraphQL
                | Self::Hcl
                | Self::Ini
                | Self::Perl
                | Self::CMake
        )
    }

    /// Whether the language writes `//` comments.
    ///
    /// Zig has `//` but no `/* */`, which is why this is not the same set as
    /// [`Self::slash_comments`].
    fn line_slash_comments(self) -> bool {
        self.slash_comments() || matches!(self, Self::Zig | Self::FSharp)
    }

    /// Whether the language writes `//` and `/* */` comments.
    fn slash_comments(self) -> bool {
        matches!(
            self,
            Self::Rust
                | Self::TypeScript
                | Self::Go
                | Self::C
                | Self::Cpp
                | Self::Java
                | Self::CSharp
                | Self::Kotlin
                | Self::Swift
                | Self::Dart
                | Self::Scala
                | Self::Php
                | Self::Css
                | Self::Protobuf
                | Self::Solidity
                | Self::Hcl
        )
    }

    /// Whether the language writes `--` comments.
    fn dash_comments(self) -> bool {
        matches!(self, Self::Sql | Self::Lua | Self::Haskell)
    }

    /// Whether the language writes `<!-- -->` comments.
    fn html_comments(self) -> bool {
        matches!(self, Self::Html)
    }

    /// Whether `/* */` comments nest (`/* /* */ */`).
    fn nested_comments(self) -> bool {
        matches!(
            self,
            Self::Rust | Self::Kotlin | Self::Swift | Self::Dart | Self::Scala
        )
    }

    /// Whether the language writes `/* */` block comments.
    ///
    /// Nix has them without `//` line comments, so this is not just
    /// [`Self::slash_comments`].
    fn block_comments(self) -> bool {
        self.slash_comments() || matches!(self, Self::Nix)
    }

    /// Whether the language writes `#= … =#` comments (Julia); they nest.
    fn hash_block_comments(self) -> bool {
        matches!(self, Self::Julia)
    }

    /// Whether the language writes `(* … *)` comments (OCaml, F#); they nest.
    fn paren_comments(self) -> bool {
        matches!(self, Self::OCaml | Self::FSharp)
    }

    /// Whether the language writes `;` comments (Clojure).
    fn semicolon_comments(self) -> bool {
        matches!(self, Self::Clojure)
    }

    /// Whether `"` starts a comment rather than a string (Vim script).
    fn quote_comments(self) -> bool {
        matches!(self, Self::Vim)
    }

    /// Whether the language writes `<# … #>` comments (PowerShell).
    fn angle_block_comments(self) -> bool {
        matches!(self, Self::PowerShell)
    }

    /// Whether `%` starts a comment (MATLAB / Octave).
    fn percent_comments(self) -> bool {
        matches!(self, Self::Matlab)
    }

    /// Whether `::` and `rem` start a comment (Windows batch files).
    ///
    /// `::` is a label nobody jumps to, which is how a batch file writes a
    /// comment; `rem` is the documented one.
    fn batch_comments(self) -> bool {
        matches!(self, Self::Batch)
    }

    /// Whether keywords are matched regardless of case.
    fn case_insensitive(self) -> bool {
        matches!(self, Self::Sql | Self::PowerShell | Self::Dockerfile)
    }

    /// Whether a backtick quotes a string (and not, say, a template expression).
    fn backtick_strings(self) -> bool {
        matches!(self, Self::TypeScript | Self::Shell | Self::Go)
    }
}

/// The spans of a code block, with the tokens the view colours.
///
/// Only the languages answers are written in are read. Anything else — and an
/// indented block, which has no language — comes back empty, which draws the
/// body as one plain run exactly as before there were colours.
pub fn highlight(language: &str, body: &str) -> Vec<Span> {
    let Some(language) = Language::of(language) else {
        return Vec::new();
    };
    // A diff is read line by line: what makes a line `+` or `-` is its first
    // character, and every word on it reads the same way.
    if language == Language::Diff {
        return diff_spans(body);
    }
    let characters: Vec<char> = body.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while let Some(character) = characters.get(index).copied() {
        let found = if let Some(next) = comment_end(&characters, index, language) {
            Some((Token::Comment, next))
        } else if let Some(next) = string_end(&characters, index, language) {
            Some((Token::String, next))
        } else if let Some(next) = regex_end(&characters, index, language) {
            // A regular expression reads like a string, so it borrows the
            // string colour rather than getting one of its own.
            Some((Token::String, next))
        } else if character.is_ascii_digit() {
            Some((Token::Number, number_end(&characters, index)))
        } else if is_word_start(character) {
            let end = word_end(&characters, index);
            let word: String = characters[index..end].iter().collect();
            if is_keyword(language, &word) {
                Some((Token::Keyword, end))
            } else if is_call(&characters, end) {
                Some((Token::Function, end))
            } else if is_type_name(&word) {
                Some((Token::Type, end))
            } else {
                None
            }
        } else {
            None
        };
        match found {
            Some((token, next)) => {
                push_span(&mut spans, &plain, None);
                plain.clear();
                let text: String = characters[index..next].iter().collect();
                push_span(&mut spans, &text, Some(token));
                index = next;
            }
            None => {
                plain.push(character);
                index += 1;
            }
        }
    }
    push_span(&mut spans, &plain, None);
    spans
}

/// The spans of a unified diff: a whole line takes the colour of what it does.
///
/// The header lines (`+++ b/x`, `--- a/x`, `@@ -1 +1 @@`) are the diff's own
/// syntax rather than content, so they are keywords; the lines the file keeps
/// are plain, which is what makes the two colours stand out.
fn diff_spans(body: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    for line in body.split_inclusive('\n') {
        push_span(&mut spans, line, diff_token(line));
    }
    spans
}

/// What kind of line a diff line is, `None` for context.
fn diff_token(line: &str) -> Option<Token> {
    let text = line.trim_end_matches(['\n', '\r']);
    if text.starts_with("+++") || text.starts_with("---") || text.starts_with("@@") {
        return Some(Token::Keyword);
    }
    if text.starts_with("diff ") || text.starts_with("index ") || text.starts_with("\\ ") {
        return Some(Token::Comment);
    }
    match text.chars().next() {
        Some('+') => Some(Token::Added),
        Some('-') => Some(Token::Removed),
        _ => None,
    }
}

/// Append a run to the spans, merging it into the previous one when the token
/// matches.
pub(crate) fn push_span(spans: &mut Vec<Span>, text: &str, token: Option<Token>) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.token == token
    {
        last.len += text.len();
        return;
    }
    spans.push(Span {
        len: text.len(),
        bold: false,
        italic: false,
        code: true,
        strike: false,
        underline: false,
        marked: false,
        token,
    });
}

/// Where a comment starting at `index` ends, if one starts there.
pub(crate) fn comment_end(characters: &[char], index: usize, language: Language) -> Option<usize> {
    let line_end = |from: usize| {
        let mut end = from;
        while characters.get(end).is_some_and(|found| *found != '\n') {
            end += 1;
        }
        end
    };
    // Block comments come first: `#=` and `<#` start with a character that a
    // line-comment rule would otherwise swallow whole.
    if language.hash_block_comments()
        && characters.get(index) == Some(&'#')
        && characters.get(index + 1) == Some(&'=')
    {
        let mut depth = 1usize;
        let mut end = index + 2;
        while end + 1 < characters.len() {
            if characters[end] == '#' && characters[end + 1] == '=' {
                depth += 1;
                end += 2;
                continue;
            }
            if characters[end] == '=' && characters[end + 1] == '#' {
                depth -= 1;
                if depth == 0 {
                    return Some(end + 2);
                }
                end += 2;
                continue;
            }
            end += 1;
        }
        return Some(characters.len());
    }
    if language.angle_block_comments()
        && characters.get(index) == Some(&'<')
        && characters.get(index + 1) == Some(&'#')
    {
        let mut end = index + 2;
        while end + 1 < characters.len() {
            if characters[end] == '#' && characters[end + 1] == '>' {
                return Some(end + 2);
            }
            end += 1;
        }
        return Some(characters.len());
    }
    if language.paren_comments()
        && characters.get(index) == Some(&'(')
        && characters.get(index + 1) == Some(&'*')
    {
        let mut depth = 1usize;
        let mut end = index + 2;
        while end + 1 < characters.len() {
            if characters[end] == '(' && characters[end + 1] == '*' {
                depth += 1;
                end += 2;
                continue;
            }
            if characters[end] == '*' && characters[end + 1] == ')' {
                depth -= 1;
                if depth == 0 {
                    return Some(end + 2);
                }
                end += 2;
                continue;
            }
            end += 1;
        }
        return Some(characters.len());
    }
    if language.semicolon_comments() && characters.get(index) == Some(&';') {
        return Some(line_end(index));
    }
    if language.quote_comments() && characters.get(index) == Some(&'"') {
        return Some(line_end(index));
    }
    if language.hash_comments() && characters.get(index) == Some(&'#') {
        return Some(line_end(index));
    }
    if characters.get(index) == Some(&'/') {
        match characters.get(index + 1) {
            Some('/') if language.line_slash_comments() => return Some(line_end(index)),
            Some('*') if language.block_comments() => {
                let nested = language.nested_comments();
                let mut depth = 1usize;
                let mut end = index + 2;
                while end + 1 < characters.len() {
                    if nested && characters[end] == '/' && characters[end + 1] == '*' {
                        depth += 1;
                        end += 2;
                        continue;
                    }
                    if characters[end] == '*' && characters[end + 1] == '/' {
                        depth -= 1;
                        if depth == 0 {
                            return Some(end + 2);
                        }
                        end += 2;
                        continue;
                    }
                    end += 1;
                }
                return Some(characters.len());
            }
            _ => {}
        }
    }
    if language.dash_comments()
        && characters.get(index) == Some(&'-')
        && characters.get(index + 1) == Some(&'-')
    {
        return Some(line_end(index));
    }
    // A batch file's `::` is a label nobody jumps to, and `rem` is the
    // documented comment; both run to the end of the line.
    if language.batch_comments() {
        if characters.get(index) == Some(&':') && characters.get(index + 1) == Some(&':') {
            return Some(line_end(index));
        }
        // A batch file's `rem` is a comment, but only as a word of its own: a
        // name like `remote` is not one.
        let rem = ['r', 'e', 'm'];
        if characters.get(index..index + 3).is_some_and(|word| {
            word.iter()
                .zip(rem)
                .all(|(found, want)| found.eq_ignore_ascii_case(&want))
        }) && characters
            .get(index + 3)
            .is_none_or(|found| found.is_whitespace())
            && (index == 0 || characters.get(index - 1) == Some(&'\n'))
        {
            return Some(line_end(index));
        }
    }
    if language.percent_comments() && characters.get(index) == Some(&'%') {
        return Some(line_end(index));
    }
    if language.html_comments()
        && characters.get(index) == Some(&'<')
        && characters.get(index + 1) == Some(&'!')
        && characters.get(index + 2) == Some(&'-')
        && characters.get(index + 3) == Some(&'-')
    {
        let mut end = index + 4;
        while end + 2 < characters.len() {
            if characters[end] == '-' && characters[end + 1] == '-' && characters[end + 2] == '>' {
                return Some(end + 3);
            }
            end += 1;
        }
        return Some(characters.len());
    }
    None
}

/// Where a string starting at `index` ends, if one starts there.
pub(crate) fn string_end(characters: &[char], index: usize, language: Language) -> Option<usize> {
    let quote = characters.get(index).copied()?;
    // `"""…"""` and `'''…'''` are one long string in Python and Julia; reading
    // them as an empty string would paint the body as code and treat a `#`
    // inside it as a comment.
    if matches!(language, Language::Python | Language::Julia)
        && matches!(quote, '"' | '\'')
        && characters.get(index + 1) == Some(&quote)
        && characters.get(index + 2) == Some(&quote)
    {
        let mut end = index + 3;
        while end + 2 < characters.len() {
            if characters[end] == '\\' {
                end += 2;
                continue;
            }
            if characters[end] == quote
                && characters[end + 1] == quote
                && characters[end + 2] == quote
            {
                return Some(end + 3);
            }
            end += 1;
        }
        return Some(characters.len());
    }
    let allowed = match quote {
        '"' | '\'' => true,
        '`' => language.backtick_strings(),
        _ => false,
    };
    if !allowed {
        return None;
    }
    let mut end = index + 1;
    while let Some(character) = characters.get(end).copied() {
        if character == '\\' {
            end += 2;
            continue;
        }
        if character == quote {
            return Some(end + 1);
        }
        // An unterminated string ends at the line, so one odd line cannot paint
        // the rest of the block as a string.
        if character == '\n' {
            return Some(end);
        }
        end += 1;
    }
    Some(characters.len())
}

/// Where the number starting at `index` ends.
pub(crate) fn number_end(characters: &[char], index: usize) -> usize {
    let mut end = index;
    while characters
        .get(end)
        .is_some_and(|found| found.is_ascii_alphanumeric() || matches!(found, '.' | '_'))
    {
        end += 1;
    }
    end
}

/// Whether `characters[index]` starts a word (an identifier or a keyword).
pub(crate) fn is_word_start(character: char) -> bool {
    character.is_alphabetic() || character == '_'
}

/// Whether a word ending at `after` is used as a call: a `(` follows it.
pub(crate) fn is_call(characters: &[char], after: usize) -> bool {
    let mut at = after;
    while characters
        .get(at)
        .is_some_and(|character| matches!(character, ' ' | '\t'))
    {
        at += 1;
    }
    characters.get(at) == Some(&'(')
}

/// Whether a word reads as a type name: it starts with a capital and also has a
/// small letter. `String` and `Vec` are types; `MAX_SIZE` is a constant and stays
/// plain.
pub(crate) fn is_type_name(word: &str) -> bool {
    match word.chars().next() {
        Some(first) if first.is_uppercase() => {}
        _ => return false,
    }
    word.chars().any(|character| character.is_lowercase())
}

/// A `/…/flags` regular expression starting at `index`, in a language that has
/// regex literals.
///
/// JavaScript gives `/` two jobs, so this only reads one where an expression can
/// begin — after `=`, `(`, `,`, a keyword like `return`, and so on. After a name
/// or a closing bracket the `/` is division, and the tokeniser leaves it alone.
pub(crate) fn regex_end(characters: &[char], index: usize, language: Language) -> Option<usize> {
    if language != Language::TypeScript || characters.get(index) != Some(&'/') {
        return None;
    }
    match characters.get(index + 1) {
        Some('/' | '*') | None => return None,
        _ => {}
    }
    let mut before = index.checked_sub(1);
    while let Some(at) = before {
        match characters.get(at) {
            Some(character) if character.is_whitespace() => {
                before = at.checked_sub(1);
            }
            _ => break,
        }
    }
    if let Some(at) = before {
        let allowed = matches!(
            characters[at],
            '=' | '('
                | '['
                | '{'
                | ','
                | ':'
                | ';'
                | '!'
                | '&'
                | '|'
                | '?'
                | '+'
                | '-'
                | '*'
                | '%'
                | '<'
                | '>'
                | '~'
                | '^'
        );
        if !allowed {
            // `return /re/` and `typeof /re/`: an operator keyword may precede
            // an expression just as a punctuator does.
            let word = word_before(characters, at);
            if !matches!(
                word.as_str(),
                "return"
                    | "typeof"
                    | "instanceof"
                    | "case"
                    | "in"
                    | "of"
                    | "yield"
                    | "await"
                    | "void"
                    | "delete"
                    | "do"
                    | "else"
                    | "new"
                    | "throw"
            ) {
                return None;
            }
        }
    }
    let mut end = index + 1;
    while let Some(character) = characters.get(end).copied() {
        match character {
            '\\' => end += 2,
            // A literal cannot run past the line; an unclosed one is not a
            // regex at all.
            '\n' => return None,
            '/' => {
                let mut after = end + 1;
                while characters
                    .get(after)
                    .is_some_and(|found| found.is_ascii_alphabetic())
                {
                    after += 1;
                }
                return Some(after);
            }
            _ => end += 1,
        }
    }
    None
}

/// The identifier ending at `at`, or the character there.
pub(crate) fn word_before(characters: &[char], at: usize) -> String {
    let mut start = at;
    while start > 0
        && characters
            .get(start - 1)
            .is_some_and(|character| character.is_alphanumeric() || matches!(character, '_' | '$'))
    {
        start -= 1;
    }
    characters[start..=at].iter().collect()
}

/// Where the word starting at `index` ends.
pub(crate) fn word_end(characters: &[char], index: usize) -> usize {
    let mut end = index;
    while characters
        .get(end)
        .is_some_and(|found| found.is_alphanumeric() || *found == '_')
    {
        end += 1;
    }
    end
}

/// Whether `word` is a keyword of `language`.
pub(crate) fn is_keyword(language: Language, word: &str) -> bool {
    let lowered = word.to_ascii_lowercase();
    let word = if language.case_insensitive() {
        lowered.as_str()
    } else {
        word
    };
    let words: &[&str] = match language {
        Language::Rust => &[
            "fn", "let", "mut", "pub", "use", "struct", "enum", "impl", "match", "if", "else",
            "for", "while", "loop", "return", "self", "Self", "where", "trait", "mod", "crate",
            "as", "in", "const", "static", "ref", "move", "async", "await", "dyn", "unsafe",
            "type", "break", "continue", "extern", "true", "false",
        ],
        Language::TypeScript => &[
            "function",
            "const",
            "let",
            "var",
            "class",
            "interface",
            "type",
            "import",
            "export",
            "from",
            "default",
            "return",
            "if",
            "else",
            "for",
            "while",
            "new",
            "this",
            "async",
            "await",
            "try",
            "catch",
            "throw",
            "typeof",
            "instanceof",
            "extends",
            "implements",
            "public",
            "private",
            "protected",
            "readonly",
            "enum",
            "true",
            "false",
            "null",
            "undefined",
            "static",
            "yield",
        ],
        Language::Python => &[
            "def", "class", "import", "from", "return", "if", "elif", "else", "for", "while", "in",
            "not", "and", "or", "is", "None", "True", "False", "with", "as", "try", "except",
            "finally", "lambda", "pass", "break", "continue", "global", "nonlocal", "yield",
            "async", "await", "raise", "del", "assert",
        ],
        Language::Shell => &[
            "if", "then", "else", "elif", "fi", "for", "do", "done", "while", "until", "case",
            "esac", "function", "echo", "export", "local", "return", "exit", "source", "readonly",
            "declare",
        ],
        Language::Go => &[
            "func",
            "package",
            "import",
            "var",
            "const",
            "type",
            "struct",
            "interface",
            "map",
            "chan",
            "go",
            "defer",
            "return",
            "if",
            "else",
            "for",
            "range",
            "switch",
            "case",
            "default",
            "select",
            "break",
            "continue",
            "nil",
            "true",
            "false",
        ],
        Language::Sql => &[
            "select", "from", "where", "insert", "into", "values", "update", "set", "delete",
            "create", "table", "drop", "alter", "join", "left", "right", "inner", "outer", "on",
            "group", "by", "order", "having", "limit", "offset", "distinct", "as", "and", "or",
            "not", "null", "is", "in", "like", "between", "union", "all", "desc", "asc",
        ],
        Language::Json => &["true", "false", "null"],
        Language::C => &[
            "auto", "break", "case", "char", "const", "continue", "default", "do", "double",
            "else", "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long",
            "register", "restrict", "return", "short", "signed", "sizeof", "static", "struct",
            "switch", "typedef", "union", "unsigned", "void", "volatile", "while", "true", "false",
            "NULL",
        ],
        Language::Cpp => &[
            "auto",
            "bool",
            "break",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "constexpr",
            "continue",
            "default",
            "delete",
            "do",
            "double",
            "else",
            "enum",
            "explicit",
            "extern",
            "false",
            "float",
            "for",
            "friend",
            "if",
            "inline",
            "int",
            "long",
            "namespace",
            "new",
            "noexcept",
            "nullptr",
            "operator",
            "override",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "signed",
            "sizeof",
            "static",
            "struct",
            "switch",
            "template",
            "this",
            "throw",
            "true",
            "try",
            "typedef",
            "typename",
            "union",
            "unsigned",
            "using",
            "virtual",
            "void",
            "volatile",
            "while",
        ],
        Language::Java => &[
            "abstract",
            "assert",
            "boolean",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "class",
            "const",
            "continue",
            "default",
            "do",
            "double",
            "else",
            "enum",
            "extends",
            "final",
            "finally",
            "float",
            "for",
            "if",
            "implements",
            "import",
            "instanceof",
            "int",
            "interface",
            "long",
            "native",
            "new",
            "null",
            "package",
            "private",
            "protected",
            "public",
            "return",
            "short",
            "static",
            "strictfp",
            "super",
            "switch",
            "synchronized",
            "this",
            "throw",
            "throws",
            "transient",
            "true",
            "false",
            "try",
            "void",
            "volatile",
            "while",
            "var",
            "record",
        ],
        Language::CSharp => &[
            "abstract",
            "as",
            "async",
            "await",
            "base",
            "bool",
            "break",
            "byte",
            "case",
            "catch",
            "char",
            "checked",
            "class",
            "const",
            "continue",
            "decimal",
            "default",
            "delegate",
            "do",
            "double",
            "else",
            "enum",
            "event",
            "explicit",
            "extern",
            "false",
            "finally",
            "fixed",
            "float",
            "for",
            "foreach",
            "get",
            "goto",
            "if",
            "implicit",
            "in",
            "int",
            "interface",
            "internal",
            "is",
            "lock",
            "long",
            "namespace",
            "new",
            "null",
            "object",
            "operator",
            "out",
            "override",
            "params",
            "private",
            "protected",
            "public",
            "readonly",
            "ref",
            "return",
            "sbyte",
            "sealed",
            "set",
            "short",
            "sizeof",
            "stackalloc",
            "static",
            "string",
            "struct",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "uint",
            "ulong",
            "unchecked",
            "unsafe",
            "ushort",
            "using",
            "var",
            "virtual",
            "void",
            "volatile",
            "while",
            "yield",
        ],
        Language::Kotlin => &[
            "as",
            "break",
            "class",
            "continue",
            "do",
            "else",
            "false",
            "for",
            "fun",
            "if",
            "in",
            "interface",
            "is",
            "null",
            "object",
            "package",
            "return",
            "super",
            "this",
            "throw",
            "true",
            "try",
            "typealias",
            "typeof",
            "val",
            "var",
            "when",
            "while",
            "by",
            "catch",
            "constructor",
            "dynamic",
            "finally",
            "get",
            "import",
            "init",
            "set",
            "where",
            "abstract",
            "annotation",
            "companion",
            "const",
            "data",
            "enum",
            "external",
            "final",
            "infix",
            "inline",
            "inner",
            "internal",
            "lateinit",
            "open",
            "operator",
            "out",
            "override",
            "private",
            "protected",
            "public",
            "reified",
            "sealed",
            "suspend",
            "tailrec",
            "vararg",
        ],
        Language::Swift => &[
            "associatedtype",
            "class",
            "deinit",
            "enum",
            "extension",
            "fileprivate",
            "func",
            "import",
            "init",
            "inout",
            "internal",
            "let",
            "open",
            "operator",
            "private",
            "protocol",
            "public",
            "rethrows",
            "static",
            "struct",
            "subscript",
            "typealias",
            "var",
            "break",
            "case",
            "continue",
            "default",
            "defer",
            "do",
            "else",
            "fallthrough",
            "for",
            "guard",
            "if",
            "in",
            "repeat",
            "return",
            "switch",
            "where",
            "while",
            "as",
            "catch",
            "false",
            "is",
            "nil",
            "super",
            "self",
            "Self",
            "throw",
            "throws",
            "true",
            "try",
            "async",
            "await",
            "some",
            "any",
        ],
        Language::Ruby => &[
            "alias",
            "and",
            "begin",
            "break",
            "case",
            "class",
            "def",
            "do",
            "else",
            "elsif",
            "end",
            "ensure",
            "false",
            "for",
            "if",
            "in",
            "module",
            "next",
            "nil",
            "not",
            "or",
            "redo",
            "rescue",
            "retry",
            "return",
            "self",
            "super",
            "then",
            "true",
            "undef",
            "unless",
            "until",
            "when",
            "while",
            "yield",
            "require",
            "require_relative",
            "attr_accessor",
            "attr_reader",
            "attr_writer",
            "puts",
            "lambda",
            "proc",
            "new",
        ],
        Language::Php => &[
            "abstract",
            "and",
            "array",
            "as",
            "break",
            "callable",
            "case",
            "catch",
            "class",
            "clone",
            "const",
            "continue",
            "declare",
            "default",
            "do",
            "echo",
            "else",
            "elseif",
            "empty",
            "enddeclare",
            "endfor",
            "endforeach",
            "endif",
            "endswitch",
            "endwhile",
            "enum",
            "extends",
            "final",
            "finally",
            "fn",
            "for",
            "foreach",
            "function",
            "global",
            "goto",
            "if",
            "implements",
            "include",
            "include_once",
            "instanceof",
            "insteadof",
            "interface",
            "isset",
            "list",
            "match",
            "namespace",
            "new",
            "or",
            "print",
            "private",
            "protected",
            "public",
            "readonly",
            "require",
            "require_once",
            "return",
            "static",
            "switch",
            "throw",
            "trait",
            "true",
            "false",
            "null",
            "try",
            "unset",
            "use",
            "var",
            "while",
            "xor",
            "yield",
            "int",
            "float",
            "string",
            "bool",
        ],
        Language::Lua => &[
            "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto",
            "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until",
            "while",
        ],
        Language::Yaml => &["true", "false", "null", "yes", "no", "on", "off"],
        Language::Toml => &["true", "false"],
        // HTML has tags, not keywords; the comment rule is what it adds.
        Language::Html => &[],
        Language::Css => &[
            "important",
            "media",
            "import",
            "keyframes",
            "supports",
            "font-face",
            "from",
            "to",
            "inherit",
            "initial",
            "unset",
            "none",
            "auto",
            "block",
            "inline",
            "flex",
            "grid",
            "absolute",
            "relative",
            "fixed",
            "sticky",
            "bold",
            "italic",
            "solid",
            "dashed",
            "transparent",
            "currentColor",
        ],
        Language::Dart => &[
            "abstract",
            "as",
            "assert",
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "covariant",
            "default",
            "deferred",
            "do",
            "dynamic",
            "else",
            "enum",
            "export",
            "extends",
            "extension",
            "external",
            "factory",
            "false",
            "final",
            "finally",
            "for",
            "get",
            "hide",
            "if",
            "implements",
            "import",
            "in",
            "interface",
            "is",
            "late",
            "library",
            "mixin",
            "new",
            "null",
            "on",
            "operator",
            "part",
            "required",
            "rethrow",
            "return",
            "set",
            "show",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typedef",
            "var",
            "void",
            "while",
            "with",
            "yield",
        ],
        Language::Scala => &[
            "abstract",
            "case",
            "catch",
            "class",
            "def",
            "do",
            "else",
            "enum",
            "extends",
            "false",
            "final",
            "finally",
            "for",
            "if",
            "implicit",
            "import",
            "lazy",
            "match",
            "new",
            "null",
            "object",
            "override",
            "package",
            "private",
            "protected",
            "return",
            "sealed",
            "super",
            "then",
            "this",
            "throw",
            "trait",
            "true",
            "try",
            "type",
            "val",
            "var",
            "while",
            "with",
            "yield",
            "given",
            "using",
            "export",
            "extension",
            "inline",
            "opaque",
            "open",
            "infix",
        ],
        Language::R => &[
            "if", "else", "repeat", "while", "function", "for", "in", "next", "break", "TRUE",
            "FALSE", "NULL", "Inf", "NaN", "NA", "library", "require",
        ],
        Language::Haskell => &[
            "case", "class", "data", "default", "deriving", "do", "else", "if", "import", "in",
            "infix", "infixl", "infixr", "instance", "let", "module", "newtype", "of", "then",
            "type", "where", "forall", "foreign", "family",
        ],
        Language::Elixir => &[
            "def",
            "defp",
            "defmodule",
            "defmacro",
            "defstruct",
            "defimpl",
            "defprotocol",
            "defdelegate",
            "do",
            "end",
            "fn",
            "case",
            "cond",
            "if",
            "unless",
            "else",
            "try",
            "rescue",
            "catch",
            "after",
            "raise",
            "reraise",
            "throw",
            "receive",
            "for",
            "with",
            "when",
            "and",
            "or",
            "not",
            "in",
            "true",
            "false",
            "nil",
            "use",
            "import",
            "require",
            "alias",
            "quote",
            "unquote",
            "super",
        ],
        Language::PowerShell => &[
            "function", "param", "begin", "process", "end", "if", "elseif", "else", "switch",
            "foreach", "for", "while", "do", "until", "try", "catch", "finally", "throw", "return",
            "break", "continue", "class", "enum", "using", "in", "not", "and", "or", "true",
            "false", "null", "filter", "trap", "data",
        ],
        Language::Dockerfile => &[
            "from",
            "run",
            "cmd",
            "label",
            "maintainer",
            "expose",
            "env",
            "add",
            "copy",
            "entrypoint",
            "volume",
            "user",
            "workdir",
            "arg",
            "onbuild",
            "stopsignal",
            "healthcheck",
            "shell",
        ],
        Language::Nix => &[
            "let", "in", "rec", "with", "inherit", "assert", "if", "then", "else", "or",
        ],
        Language::Julia => &[
            "function",
            "end",
            "if",
            "else",
            "elseif",
            "for",
            "while",
            "begin",
            "let",
            "do",
            "try",
            "catch",
            "finally",
            "struct",
            "mutable",
            "module",
            "baremodule",
            "using",
            "import",
            "export",
            "return",
            "break",
            "continue",
            "const",
            "global",
            "local",
            "macro",
            "quote",
            "abstract",
            "primitive",
            "type",
            "where",
            "in",
            "isa",
            "true",
            "false",
            "nothing",
            "missing",
        ],
        Language::Makefile => &[
            "ifeq", "ifneq", "ifdef", "ifndef", "else", "endif", "include", "sinclude", "define",
            "endef", "export", "unexport", "override", "private", "vpath", "shell",
        ],
        Language::GraphQL => &[
            "query",
            "mutation",
            "subscription",
            "fragment",
            "on",
            "type",
            "interface",
            "input",
            "enum",
            "union",
            "scalar",
            "schema",
            "extend",
            "directive",
            "implements",
            "true",
            "false",
            "null",
        ],
        Language::Protobuf => &[
            "syntax",
            "package",
            "import",
            "option",
            "message",
            "enum",
            "service",
            "rpc",
            "returns",
            "repeated",
            "optional",
            "required",
            "oneof",
            "map",
            "reserved",
            "extend",
            "extensions",
            "true",
            "false",
        ],
        Language::Zig => &[
            "fn",
            "const",
            "var",
            "pub",
            "return",
            "if",
            "else",
            "while",
            "for",
            "switch",
            "struct",
            "enum",
            "union",
            "error",
            "try",
            "catch",
            "defer",
            "errdefer",
            "unreachable",
            "comptime",
            "inline",
            "export",
            "extern",
            "asm",
            "test",
            "and",
            "or",
            "orelse",
            "null",
            "undefined",
            "true",
            "false",
        ],
        Language::Solidity => &[
            "pragma",
            "contract",
            "interface",
            "library",
            "function",
            "modifier",
            "event",
            "struct",
            "enum",
            "mapping",
            "address",
            "uint",
            "int",
            "bool",
            "string",
            "bytes",
            "public",
            "private",
            "internal",
            "external",
            "pure",
            "view",
            "payable",
            "returns",
            "return",
            "if",
            "else",
            "for",
            "while",
            "do",
            "break",
            "continue",
            "new",
            "delete",
            "emit",
            "require",
            "assert",
            "revert",
            "import",
            "is",
            "using",
            "constructor",
            "memory",
            "storage",
            "calldata",
            "true",
            "false",
        ],
        Language::Hcl => &[
            "resource",
            "data",
            "variable",
            "output",
            "module",
            "provider",
            "terraform",
            "locals",
            "provisioner",
            "connection",
            "backend",
            "required_providers",
            "for",
            "in",
            "if",
            "true",
            "false",
            "null",
        ],
        Language::OCaml => &[
            "let",
            "in",
            "rec",
            "and",
            "fun",
            "function",
            "match",
            "with",
            "if",
            "then",
            "else",
            "begin",
            "end",
            "module",
            "open",
            "type",
            "val",
            "exception",
            "try",
            "raise",
            "for",
            "to",
            "downto",
            "while",
            "do",
            "done",
            "struct",
            "sig",
            "functor",
            "object",
            "method",
            "mutable",
            "private",
            "virtual",
            "class",
            "inherit",
            "initializer",
            "as",
            "of",
            "when",
            "true",
            "false",
            "not",
            "land",
            "lor",
            "lxor",
            "mod",
            "lazy",
        ],
        Language::FSharp => &[
            "let",
            "in",
            "rec",
            "and",
            "fun",
            "function",
            "match",
            "with",
            "if",
            "then",
            "elif",
            "else",
            "begin",
            "end",
            "module",
            "namespace",
            "open",
            "type",
            "member",
            "static",
            "override",
            "interface",
            "inherit",
            "new",
            "do",
            "for",
            "to",
            "downto",
            "while",
            "yield",
            "return",
            "try",
            "finally",
            "use",
            "mutable",
            "abstract",
            "default",
            "val",
            "true",
            "false",
            "null",
            "not",
            "async",
            "task",
        ],
        Language::Clojure => &[
            "def",
            "defn",
            "defmacro",
            "defstruct",
            "defprotocol",
            "defrecord",
            "deftype",
            "defmulti",
            "defmethod",
            "fn",
            "let",
            "if",
            "if-let",
            "when",
            "cond",
            "case",
            "do",
            "loop",
            "recur",
            "try",
            "catch",
            "finally",
            "throw",
            "ns",
            "in-ns",
            "require",
            "import",
            "use",
            "quote",
            "var",
            "new",
            "true",
            "false",
            "nil",
            "and",
            "or",
            "not",
            "map",
            "reduce",
            "filter",
        ],
        Language::Vim => &[
            "let",
            "unlet",
            "set",
            "setlocal",
            "if",
            "elseif",
            "else",
            "endif",
            "for",
            "endfor",
            "in",
            "while",
            "endwhile",
            "function",
            "endfunction",
            "return",
            "call",
            "execute",
            "echo",
            "echom",
            "autocmd",
            "augroup",
            "map",
            "nnoremap",
            "inoremap",
            "vnoremap",
            "command",
            "silent",
            "try",
            "catch",
            "finally",
            "endtry",
            "throw",
            "and",
            "or",
            "not",
        ],
        // A diff has no words of its own: `diff_spans` colours it by line.
        Language::Diff => &[],
        // A settings file has no keywords either: every key is a name, and
        // colouring them all would be noise.
        Language::Ini => &[],
        Language::Batch => &[
            "if",
            "else",
            "for",
            "in",
            "do",
            "goto",
            "call",
            "set",
            "setlocal",
            "endlocal",
            "echo",
            "shift",
            "exit",
            "not",
            "exist",
            "defined",
            "errorlevel",
        ],
        Language::Perl => &[
            "my", "our", "local", "sub", "use", "require", "package", "if", "elsif", "else",
            "unless", "while", "until", "for", "foreach", "do", "return", "last", "next", "redo",
            "die", "warn", "print", "say", "push", "pop", "shift", "unshift", "keys", "values",
            "exists", "defined", "ref", "bless", "eval", "and", "or", "not", "eq", "ne", "lt",
            "gt", "le", "ge", "cmp",
        ],
        Language::CMake => &[
            "cmake_minimum_required",
            "project",
            "add_executable",
            "add_library",
            "target_link_libraries",
            "target_include_directories",
            "target_compile_options",
            "set",
            "unset",
            "if",
            "elseif",
            "else",
            "endif",
            "foreach",
            "endforeach",
            "function",
            "endfunction",
            "macro",
            "endmacro",
            "include",
            "find_package",
            "option",
            "message",
            "install",
            "enable_testing",
            "add_test",
            "list",
            "string",
            "file",
            "get_filename_component",
            "configure_file",
        ],
        Language::Matlab => &[
            "function",
            "end",
            "if",
            "elseif",
            "else",
            "for",
            "while",
            "switch",
            "case",
            "otherwise",
            "try",
            "catch",
            "break",
            "continue",
            "return",
            "global",
            "persistent",
            "classdef",
            "properties",
            "methods",
            "events",
            "true",
            "false",
            "inf",
            "nan",
            "pi",
        ],
    };
    words.contains(&word)
}
