//! The spotlight's input grammar (folio page Spotlight, "What each input does"): nothing, free text (with an optional
//! leading `@bot`), `@` while picking a bot, `>` for Go to only, `/` for a command, or a Launchpad link or short ID.

#[derive(Debug, PartialEq, Eq)]
pub enum Input<'a> {
    Empty,
    /// `@hub`: picking a bot (no space yet).
    Mention(&'a str),
    /// Free text, asked to the current target or to the bot named first (`@hubert-bot why is CI red`).
    Text { bot: Option<&'a str>, text: &'a str },
    /// `>folio`: Go to matches only.
    GoTo(&'a str),
    /// `/launch fix the tests`.
    Command { name: &'a str, rest: &'a str },
    /// `lp:session/…` or an address of a Launchpad page: resolved by Home.
    Link(&'a str),
}

/// The spotlight's commands, in the order the list shows them.
pub const COMMANDS: &[(&str, &str)] = &[
    ("launch", "Quick launch a new agent (with the text after the command)"),
    ("workspace", "Next workspace for the companion"),
    ("today", "Open Today in the browser"),
    ("dnd", "Pause the companion's notifications for 1 hour"),
    ("settings", "Open the companion's settings"),
];

pub fn parse(raw: &str) -> Input<'_> {
    let s = raw.trim_start();
    if s.trim().is_empty() {
        return Input::Empty;
    }
    let one_word = !s.trim_end().contains(char::is_whitespace);
    if one_word && (s.starts_with("lp:") || s.starts_with("https://") || s.starts_with("http://")) {
        return Input::Link(s.trim_end());
    }
    if let Some(rest) = s.strip_prefix('>') {
        return Input::GoTo(rest.trim());
    }
    if let Some(rest) = s.strip_prefix('/') {
        let (name, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        return Input::Command { name, rest: rest.trim() };
    }
    if let Some(rest) = s.strip_prefix('@') {
        return match rest.split_once(char::is_whitespace) {
            None => Input::Mention(rest),
            Some((bot, text)) => Input::Text { bot: Some(bot), text: text.trim() },
        };
    }
    Input::Text { bot: None, text: s.trim_end() }
}

/// The commands whose name starts with `prefix`.
pub fn commands(prefix: &str) -> impl Iterator<Item = &'static (&'static str, &'static str)> + '_ {
    COMMANDS.iter().filter(move |(name, _)| name.starts_with(&prefix.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar() {
        assert_eq!(parse("  "), Input::Empty);
        assert_eq!(parse("why is CI red"), Input::Text { bot: None, text: "why is CI red" });
        assert_eq!(parse("@hub"), Input::Mention("hub"));
        assert_eq!(parse("@"), Input::Mention(""));
        assert_eq!(parse("@hubert-bot  why is CI red "), Input::Text { bot: Some("hubert-bot"), text: "why is CI red" });
        assert_eq!(parse("@hubert-bot "), Input::Text { bot: Some("hubert-bot"), text: "" });
        assert_eq!(parse(">folio"), Input::GoTo("folio"));
        assert_eq!(parse("> "), Input::GoTo(""));
        assert_eq!(parse("/launch fix the tests"), Input::Command { name: "launch", rest: "fix the tests" });
        assert_eq!(parse("/wor"), Input::Command { name: "wor", rest: "" });
        assert_eq!(parse("lp:session/abc/12"), Input::Link("lp:session/abc/12"));
        assert_eq!(parse("https://me-home.hf.space/agents?session=abc "), Input::Link("https://me-home.hf.space/agents?session=abc"));
        // A link inside a sentence is text (the bot gets it as a reference).
        assert_eq!(parse("look at lp:session/abc"), Input::Text { bot: None, text: "look at lp:session/abc" });
        assert_eq!(commands("w").map(|c| c.0).collect::<Vec<_>>(), ["workspace"]);
        assert_eq!(commands("").count(), COMMANDS.len());
    }
}
