//! `:smile:` short codes, the few an answer is likely to write.

/// A `:name:` short code starting at `index` → the character it stands for and
/// where the text after it starts.
///
/// Only names on the list are replaced, and every name is a word an answer would
/// not otherwise put between two colons: a time (`12:30`), a path (`a::b`) and
/// an address (`https://`) all stay as written. The colon after a word is what
/// makes a code: `https://x` has nothing between its two colons, so it is text.
pub(crate) fn emoji_at(characters: &[char], index: usize) -> Option<(&'static str, usize)> {
    if characters.get(index) != Some(&':') {
        return None;
    }
    let mut end = index + 1;
    while characters
        .get(end)
        .is_some_and(|found| found.is_ascii_alphanumeric() || matches!(found, '_' | '-' | '+'))
    {
        end += 1;
    }
    if characters.get(end) != Some(&':') || end == index + 1 {
        return None;
    }
    let name: String = characters[index + 1..end].iter().collect();
    let emoji = emoji_of(&name)?;
    Some((emoji, end + 1))
}

/// The character a short code names, `None` for anything else.
///
/// The list is deliberately short: it holds the codes answers actually use, and
/// it leaves out anything that is also an ordinary identifier (`new`, `ok`,
/// `free`, `top`), so `Vec::new()` in prose is not turned into a picture.
pub(crate) fn emoji_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "smile" => "😄",
        "grin" => "😁",
        "joy" => "😂",
        "laughing" | "satisfied" => "😆",
        "wink" => "😉",
        "blush" => "😊",
        "heart_eyes" => "😍",
        "thinking" | "thinking_face" => "🤔",
        "neutral_face" => "😐",
        "worried" => "😟",
        "cry" => "😢",
        "sob" => "😭",
        "angry" => "😠",
        "sunglasses" => "😎",
        "sweat_smile" => "😅",
        "scream" => "😱",
        "sleeping" => "😴",
        "slightly_smiling_face" => "🙂",
        "upside_down_face" => "🙃",
        "heart" => "❤️",
        "broken_heart" => "💔",
        "sparkling_heart" => "💖",
        "thumbsup" | "+1" => "👍",
        "thumbsdown" | "-1" => "👎",
        "ok_hand" => "👌",
        "clap" => "👏",
        "pray" => "🙏",
        "muscle" => "💪",
        "wave" => "👋",
        "point_right" => "👉",
        "point_left" => "👈",
        "point_up" => "👆",
        "point_down" => "👇",
        "eyes" => "👀",
        "fire" => "🔥",
        "rocket" => "🚀",
        "tada" => "🎉",
        "sparkles" => "✨",
        "star" => "⭐",
        "star2" => "🌟",
        "zap" => "⚡",
        "boom" => "💥",
        "warning" => "⚠️",
        "no_entry" => "⛔",
        "question" => "❓",
        "exclamation" => "❗",
        "information_source" => "ℹ️",
        "white_check_mark" => "✅",
        "heavy_check_mark" => "✔️",
        "x" | "heavy_multiplication_x" => "❌",
        "bulb" => "💡",
        "book" => "📖",
        "books" => "📚",
        "memo" | "pencil" => "📝",
        "pushpin" => "📌",
        "paperclip" => "📎",
        "bug" => "🐛",
        "lock" => "🔒",
        "unlock" => "🔓",
        "key" => "🔑",
        "gear" | "gears" => "⚙️",
        "wrench" => "🔧",
        "hammer" => "🔨",
        "package" => "📦",
        "file_folder" => "📁",
        "computer" => "💻",
        "iphone" => "📱",
        "email" | "envelope" => "✉️",
        "calendar" => "📅",
        "clock1" | "hourglass" => "⏳",
        "coffee" => "☕",
        "beer" => "🍺",
        "cake" => "🍰",
        "apple" => "🍎",
        "sunny" => "☀️",
        "cloud" => "☁️",
        "umbrella" => "☂️",
        "snowflake" => "❄️",
        "rainbow" => "🌈",
        "moon" => "🌙",
        "earth_asia" | "globe_with_meridians" => "🌏",
        "sunrise" => "🌅",
        "checkered_flag" => "🏁",
        "construction" => "🚧",
        "recycle" => "♻️",
        "test_tube" => "🧪",
        "dna" => "🧬",
        "robot" => "🤖",
        "brain" => "🧠",
        _ => return None,
    })
}
