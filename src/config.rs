use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Action {
    Quit,
    PreviousFile,
    NextFile,
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Top,
    Bottom,
    Search,
    NextMatch,
    PreviousMatch,
    NextHunk,
    PreviousHunk,
    ToggleReviewed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KeyBinding {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyBinding {
    fn matches(&self, key: KeyEvent) -> bool {
        if self.code != key.code {
            return false;
        }

        let key_required = key
            .modifiers
            .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let binding_required = self
            .modifiers
            .intersection(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if key_required != binding_required {
            return false;
        }

        if matches!(self.code, KeyCode::Char(ch) if ch.is_ascii_uppercase()) {
            return true;
        }

        key.modifiers.contains(KeyModifiers::SHIFT) == self.modifiers.contains(KeyModifiers::SHIFT)
    }

    fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("alt".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT)
            && !matches!(self.code, KeyCode::Char(ch) if ch.is_ascii_uppercase())
        {
            parts.push("shift".to_string());
        }

        parts.push(match self.code {
            KeyCode::Backspace => "backspace".to_string(),
            KeyCode::Enter => "enter".to_string(),
            KeyCode::Left => "left".to_string(),
            KeyCode::Right => "right".to_string(),
            KeyCode::Up => "up".to_string(),
            KeyCode::Down => "down".to_string(),
            KeyCode::Home => "home".to_string(),
            KeyCode::End => "end".to_string(),
            KeyCode::PageUp => "pageup".to_string(),
            KeyCode::PageDown => "pagedown".to_string(),
            KeyCode::Esc => "esc".to_string(),
            KeyCode::Char(ch) => ch.to_string(),
            _ => "?".to_string(),
        });

        parts.join("-")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct KeyBindings {
    bindings_by_action: HashMap<Action, Vec<KeyBinding>>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings_by_action = HashMap::new();

        bindings_by_action.insert(Action::Quit, parse_default_bindings(&["q", "Q"]));
        bindings_by_action.insert(Action::PreviousFile, parse_default_bindings(&["h", "left"]));
        bindings_by_action.insert(Action::NextFile, parse_default_bindings(&["l", "right"]));
        bindings_by_action.insert(Action::ScrollUp, parse_default_bindings(&["k", "up"]));
        bindings_by_action.insert(Action::ScrollDown, parse_default_bindings(&["j", "down"]));
        bindings_by_action.insert(
            Action::PageUp,
            parse_default_bindings(&["ctrl-u", "pageup"]),
        );
        bindings_by_action.insert(
            Action::PageDown,
            parse_default_bindings(&["ctrl-d", "pagedown"]),
        );
        bindings_by_action.insert(Action::Top, parse_default_bindings(&["g", "home"]));
        bindings_by_action.insert(Action::Bottom, parse_default_bindings(&["G", "end"]));
        bindings_by_action.insert(Action::Search, parse_default_bindings(&["/"]));
        bindings_by_action.insert(Action::NextMatch, parse_default_bindings(&["n"]));
        bindings_by_action.insert(Action::PreviousMatch, parse_default_bindings(&["N"]));
        bindings_by_action.insert(Action::NextHunk, parse_default_bindings(&["}"]));
        bindings_by_action.insert(Action::PreviousHunk, parse_default_bindings(&["{"]));
        bindings_by_action.insert(Action::ToggleReviewed, parse_default_bindings(&["r"]));

        Self { bindings_by_action }
    }
}

impl KeyBindings {
    pub(crate) fn load(config_path: Option<&Path>, skip_config: bool) -> Result<Self> {
        let mut keybindings = Self::default();
        if skip_config {
            return Ok(keybindings);
        }

        let Some(path) = config_path
            .map(Path::to_path_buf)
            .or_else(default_config_path)
        else {
            return Ok(keybindings);
        };

        if !path.exists() {
            if config_path.is_some() {
                bail!("config file does not exist: {}", path.display());
            }
            return Ok(keybindings);
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        apply_config(&mut keybindings, &contents)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        Ok(keybindings)
    }

    pub(crate) fn action_for_key(&self, key: KeyEvent) -> Option<Action> {
        ACTION_ORDER.iter().copied().find(|action| {
            self.bindings_by_action
                .get(action)
                .is_some_and(|bindings| bindings.iter().any(|binding| binding.matches(key)))
        })
    }

    pub(crate) fn primary_label_for_action(&self, action: Action) -> String {
        self.bindings_by_action
            .get(&action)
            .and_then(|bindings| bindings.first())
            .map(KeyBinding::label)
            .unwrap_or_else(|| "?".to_string())
    }
}

const ACTION_ORDER: &[Action] = &[
    Action::Quit,
    Action::PreviousFile,
    Action::NextFile,
    Action::ScrollUp,
    Action::ScrollDown,
    Action::PageUp,
    Action::PageDown,
    Action::Top,
    Action::Bottom,
    Action::Search,
    Action::NextMatch,
    Action::PreviousMatch,
    Action::NextHunk,
    Action::PreviousHunk,
    Action::ToggleReviewed,
];

fn default_config_path() -> Option<PathBuf> {
    if let Some(config_home) = env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config_home).join("deff/config.toml"));
    }

    env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/deff/config.toml"))
}

fn apply_config(keybindings: &mut KeyBindings, contents: &str) -> Result<()> {
    let mut in_keybindings_section = false;

    for (line_index, raw_line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            in_keybindings_section = line == "[keybindings]";
            continue;
        }

        if !in_keybindings_section {
            continue;
        }

        let Some((raw_name, raw_value)) = line.split_once('=') else {
            bail!("line {line_number}: expected key = value");
        };

        let action = parse_action(raw_name.trim())
            .with_context(|| format!("line {line_number}: unknown keybinding action"))?;
        let binding = parse_quoted_binding(raw_value.trim())
            .with_context(|| format!("line {line_number}: invalid binding value"))?;
        keybindings.set_action_binding(action, binding);
    }

    Ok(())
}

impl KeyBindings {
    fn set_action_binding(&mut self, action: Action, binding: KeyBinding) {
        for (existing_action, existing_bindings) in &mut self.bindings_by_action {
            if *existing_action != action {
                existing_bindings.retain(|existing_binding| existing_binding != &binding);
            }
        }

        self.bindings_by_action.insert(action, vec![binding]);
    }
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

fn parse_action(name: &str) -> Result<Action> {
    match name {
        "quit" => Ok(Action::Quit),
        "previous_file" => Ok(Action::PreviousFile),
        "next_file" => Ok(Action::NextFile),
        "scroll_up" => Ok(Action::ScrollUp),
        "scroll_down" => Ok(Action::ScrollDown),
        "page_up" => Ok(Action::PageUp),
        "page_down" => Ok(Action::PageDown),
        "top" => Ok(Action::Top),
        "bottom" => Ok(Action::Bottom),
        "search" => Ok(Action::Search),
        "next_match" => Ok(Action::NextMatch),
        "previous_match" => Ok(Action::PreviousMatch),
        "next_hunk" => Ok(Action::NextHunk),
        "previous_hunk" => Ok(Action::PreviousHunk),
        "toggle_reviewed" => Ok(Action::ToggleReviewed),
        _ => bail!("{name}"),
    }
}

fn parse_quoted_binding(value: &str) -> Result<KeyBinding> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        bail!("binding must be a quoted string");
    }

    parse_key_binding(&value[1..value.len() - 1])
}

fn parse_default_bindings(values: &[&str]) -> Vec<KeyBinding> {
    values
        .iter()
        .map(|value| parse_key_binding(value).expect("default keybinding should parse"))
        .collect()
}

fn parse_key_binding(value: &str) -> Result<KeyBinding> {
    let mut modifiers = KeyModifiers::empty();
    let mut key_part = None;

    for part in value.split('-') {
        let normalized = part.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "ctrl" | "control" => modifiers.insert(KeyModifiers::CONTROL),
            "alt" | "option" => modifiers.insert(KeyModifiers::ALT),
            "shift" => modifiers.insert(KeyModifiers::SHIFT),
            "" => bail!("empty keybinding segment"),
            _ if key_part.is_none() => key_part = Some(part.trim()),
            _ => bail!("only one non-modifier key is supported"),
        }
    }

    let Some(key) = key_part else {
        bail!("missing key");
    };

    let code = match key.to_ascii_lowercase().as_str() {
        "left" | "left-arrow" => KeyCode::Left,
        "right" | "right-arrow" => KeyCode::Right,
        "up" | "up-arrow" => KeyCode::Up,
        "down" | "down-arrow" => KeyCode::Down,
        "pageup" | "page-up" => KeyCode::PageUp,
        "pagedown" | "page-down" => KeyCode::PageDown,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "enter" | "return" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        _ => {
            let mut chars = key.chars();
            let Some(ch) = chars.next() else {
                bail!("missing key");
            };
            if chars.next().is_some() {
                bail!("unknown key {key}");
            }
            let ch = if modifiers.contains(KeyModifiers::SHIFT) {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            KeyCode::Char(ch)
        }
    };

    if matches!(code, KeyCode::Char(ch) if ch.is_ascii_uppercase()) {
        modifiers.insert(KeyModifiers::SHIFT);
    }

    Ok(KeyBinding { code, modifiers })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn default_keybindings_include_ctrl_page_navigation() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Some(Action::PageUp)
        );
        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            Some(Action::PageDown)
        );
    }

    #[test]
    fn config_can_replace_page_bindings_with_shifted_letters() {
        let mut bindings = KeyBindings::default();
        apply_config(
            &mut bindings,
            r#"
            [keybindings]
            page_up = "K"
            page_down = "J"
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('K'), KeyModifiers::SHIFT)),
            Some(Action::PageUp)
        );
        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('J'), KeyModifiers::SHIFT)),
            Some(Action::PageDown)
        );
        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn config_keeps_unspecified_defaults() {
        let mut bindings = KeyBindings::default();
        apply_config(
            &mut bindings,
            r#"
            [keybindings]
            page_down = "J"
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('k'), KeyModifiers::empty())),
            Some(Action::ScrollUp)
        );
    }

    #[test]
    fn configured_binding_takes_key_from_default_action() {
        let mut bindings = KeyBindings::default();
        apply_config(
            &mut bindings,
            r#"
            [keybindings]
            page_down = "j"
            "#,
        )
        .expect("config should parse");

        assert_eq!(
            bindings.action_for_key(key(KeyCode::Char('j'), KeyModifiers::empty())),
            Some(Action::PageDown)
        );
    }

    #[test]
    fn primary_label_uses_configured_binding() {
        let mut bindings = KeyBindings::default();
        apply_config(
            &mut bindings,
            r#"
            [keybindings]
            page_down = "J"
            previous_file = "alt-left"
            "#,
        )
        .expect("config should parse");

        assert_eq!(bindings.primary_label_for_action(Action::PageDown), "J");
        assert_eq!(
            bindings.primary_label_for_action(Action::PreviousFile),
            "alt-left"
        );
    }
}
