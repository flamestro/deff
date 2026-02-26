use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use crate::{
    model::{DiffFileView, PaneOffsets, PaneSide},
    render::{create_frame_layout, get_body_line_count, get_max_pane_offsets, get_pane_for_column},
};

const MOUSE_WHEEL_SCROLL_LINES: usize = 3;
const MOUSE_WHEEL_HORIZONTAL_COLUMNS: usize = 8;

#[derive(Clone, Debug, Default)]
pub(crate) struct KeypressOutcome {
    pub(crate) should_quit: bool,
    pub(crate) review_toggled: Option<(usize, bool)>,
}

#[derive(Clone, Debug)]
pub(crate) struct AppState {
    pub(crate) file_index: usize,
    pub(crate) scroll_offset: usize,
    pane_offsets_by_file: Vec<PaneOffsets>,
    reviewed_by_file: Vec<bool>,
    reviewed_count: usize,
    search_input_mode: bool,
    search_query: String,
    search_input: String,
    search_match_line_indexes: Vec<usize>,
    search_match_index: Option<usize>,
}

impl AppState {
    pub(crate) fn new(file_count: usize, reviewed_by_file: Vec<bool>) -> Self {
        let reviewed_by_file = if reviewed_by_file.len() == file_count {
            reviewed_by_file
        } else {
            vec![false; file_count]
        };
        let reviewed_count = reviewed_by_file
            .iter()
            .filter(|reviewed| **reviewed)
            .count();

        Self {
            file_index: 0,
            scroll_offset: 0,
            pane_offsets_by_file: vec![PaneOffsets::default(); file_count],
            reviewed_by_file,
            reviewed_count,
            search_input_mode: false,
            search_query: String::new(),
            search_input: String::new(),
            search_match_line_indexes: Vec::new(),
            search_match_index: None,
        }
    }

    pub(crate) fn current_offsets(&self) -> PaneOffsets {
        self.pane_offsets_by_file[self.file_index]
    }

    pub(crate) fn set_current_offsets(&mut self, pane_offsets: PaneOffsets) {
        self.pane_offsets_by_file[self.file_index] = pane_offsets;
    }

    pub(crate) fn reviewed_count(&self) -> usize {
        self.reviewed_count
    }

    pub(crate) fn is_current_file_reviewed(&self) -> bool {
        self.reviewed_by_file[self.file_index]
    }

    pub(crate) fn toggle_current_file_reviewed(&mut self) -> bool {
        let reviewed = &mut self.reviewed_by_file[self.file_index];
        if *reviewed {
            *reviewed = false;
            self.reviewed_count = self.reviewed_count.saturating_sub(1);
        } else {
            *reviewed = true;
            self.reviewed_count = self.reviewed_count.saturating_add(1);
        }

        *reviewed
    }

    pub(crate) fn search_status_text(&self) -> String {
        if self.search_input_mode {
            return format!("search: /{}", self.search_input);
        }

        if self.search_query.is_empty() {
            return "search: /".to_string();
        }

        if self.search_match_line_indexes.is_empty() {
            return format!("search: /{} (no matches)", self.search_query);
        }

        let current_match = self.search_match_index.unwrap_or(0).saturating_add(1);
        format!(
            "search: /{} ({}/{})",
            self.search_query,
            current_match,
            self.search_match_line_indexes.len()
        )
    }

    fn is_search_input_mode(&self) -> bool {
        self.search_input_mode
    }

    fn refresh_search_matches_for_current_file(&mut self, files: &[DiffFileView]) {
        if self.search_query.is_empty() {
            self.search_match_line_indexes.clear();
            self.search_match_index = None;
            return;
        }

        let current_file = &files[self.file_index];
        self.search_match_line_indexes =
            build_search_match_line_indexes(current_file, &self.search_query);
        self.search_match_index = if self.search_match_line_indexes.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    fn jump_to_search_match(&mut self, files: &[DiffFileView], rows: u16, forward: bool) {
        if self.search_match_line_indexes.is_empty() {
            self.search_match_index = None;
            return;
        }

        let next_match_index = next_match_index(
            self.search_match_line_indexes.len(),
            self.search_match_index,
            forward,
        );

        if let Some(match_index) = next_match_index {
            self.search_match_index = Some(match_index);
            let target_line = self.search_match_line_indexes[match_index];
            let max_scroll = max_scroll_for_current_file(files, self, rows);
            self.scroll_offset = target_line.min(max_scroll);
        }
    }

    fn jump_to_hunk(&mut self, files: &[DiffFileView], rows: u16, forward: bool) {
        let hunk_starts = build_hunk_start_lines(&files[self.file_index]);
        if hunk_starts.is_empty() {
            return;
        }

        let target = if forward {
            hunk_starts
                .iter()
                .find(|&&line| line > self.scroll_offset)
                .or(hunk_starts.first())
        } else {
            hunk_starts
                .iter()
                .rev()
                .find(|&&line| line < self.scroll_offset)
                .or(hunk_starts.last())
        };

        if let Some(&line) = target {
            let max_scroll = max_scroll_for_current_file(files, self, rows);
            self.scroll_offset = line.min(max_scroll);
        }
    }

    fn enter_search_input_mode(&mut self) {
        self.search_input_mode = true;
        self.search_input.clear();
    }

    fn exit_search_input_mode(&mut self) {
        self.search_input_mode = false;
        self.search_input.clear();
    }

    fn apply_search_input(&mut self, files: &[DiffFileView], rows: u16) {
        self.search_query = self.search_input.clone();
        self.search_input_mode = false;
        self.search_input.clear();
        self.refresh_search_matches_for_current_file(files);

        if self.search_match_line_indexes.is_empty() {
            return;
        }

        if let Some(start_index) =
            first_match_index_from_line(&self.search_match_line_indexes, self.scroll_offset, true)
        {
            self.search_match_index = Some(start_index);
            let target_line = self.search_match_line_indexes[start_index];
            let max_scroll = max_scroll_for_current_file(files, self, rows);
            self.scroll_offset = target_line.min(max_scroll);
        }
    }
}

fn max_scroll_for_current_file(files: &[DiffFileView], app: &AppState, rows: u16) -> usize {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let body_line_count = get_body_line_count(rows as usize);
    max_lines.saturating_sub(body_line_count)
}

fn move_file(delta: isize, files: &[DiffFileView], app: &mut AppState) -> bool {
    let max_index = files.len().saturating_sub(1) as isize;
    let next_index = (app.file_index as isize + delta).clamp(0, max_index) as usize;
    if next_index != app.file_index {
        app.file_index = next_index;
        app.scroll_offset = 0;
        return true;
    }

    false
}

fn move_scroll(delta: isize, files: &[DiffFileView], app: &mut AppState, rows: u16) {
    let max_scroll = max_scroll_for_current_file(files, app, rows);
    let next_offset = (app.scroll_offset as isize + delta).clamp(0, max_scroll as isize) as usize;
    app.scroll_offset = next_offset;
}

fn scroll_to_top(app: &mut AppState) {
    app.scroll_offset = 0;
}

fn scroll_to_bottom(files: &[DiffFileView], app: &mut AppState, rows: u16) {
    app.scroll_offset = max_scroll_for_current_file(files, app, rows);
}

fn move_horizontal(
    pane: PaneSide,
    delta: isize,
    files: &[DiffFileView],
    app: &mut AppState,
    columns: u16,
    rows: u16,
) {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let layout = create_frame_layout(columns, rows, max_lines);
    let max_offsets = get_max_pane_offsets(current_file, &layout);
    let current_offsets = &mut app.pane_offsets_by_file[app.file_index];

    match pane {
        PaneSide::Left => {
            current_offsets.left = (current_offsets.left as isize + delta)
                .clamp(0, max_offsets.left as isize) as usize;
        }
        PaneSide::Right => {
            current_offsets.right = (current_offsets.right as isize + delta)
                .clamp(0, max_offsets.right as isize) as usize;
        }
    }
}

fn build_hunk_start_lines(file: &DiffFileView) -> Vec<usize> {
    let mut changed: Vec<usize> = file
        .left_deleted_line_indexes
        .iter()
        .chain(file.right_added_line_indexes.iter())
        .copied()
        .collect();
    changed.sort_unstable();
    changed.dedup();

    let changed_set: std::collections::HashSet<usize> = changed.iter().copied().collect();
    changed
        .into_iter()
        .filter(|&line| line == 0 || !changed_set.contains(&(line - 1)))
        .collect()
}

fn build_search_match_line_indexes(file: &DiffFileView, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return Vec::new();
    }

    let max_lines = file.left_lines.len().max(file.right_lines.len());
    let mut match_indexes = Vec::new();
    for line_index in 0..max_lines {
        let left_matches = file
            .left_lines
            .get(line_index)
            .is_some_and(|line| line.contains(query));
        let right_matches = file
            .right_lines
            .get(line_index)
            .is_some_and(|line| line.contains(query));

        if left_matches || right_matches {
            match_indexes.push(line_index);
        }
    }

    match_indexes
}

fn first_match_index_from_line(
    matches: &[usize],
    line_index: usize,
    forward: bool,
) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }

    if forward {
        matches
            .iter()
            .position(|match_line| *match_line >= line_index)
            .or(Some(0))
    } else {
        matches
            .iter()
            .rposition(|match_line| *match_line <= line_index)
            .or(Some(matches.len().saturating_sub(1)))
    }
}

fn next_match_index(
    match_count: usize,
    current_match_index: Option<usize>,
    forward: bool,
) -> Option<usize> {
    if match_count == 0 {
        return None;
    }

    match current_match_index {
        Some(current_index) => {
            if forward {
                Some((current_index + 1) % match_count)
            } else {
                Some((current_index + match_count - 1) % match_count)
            }
        }
        None => {
            if forward {
                Some(0)
            } else {
                Some(match_count - 1)
            }
        }
    }
}

pub(crate) fn handle_keypress(
    key: KeyEvent,
    files: &[DiffFileView],
    app: &mut AppState,
    rows: u16,
) -> KeypressOutcome {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return KeypressOutcome {
            should_quit: true,
            review_toggled: None,
        };
    }

    if app.is_search_input_mode() {
        match key.code {
            KeyCode::Enter => app.apply_search_input(files, rows),
            KeyCode::Esc => app.exit_search_input_mode(),
            KeyCode::Backspace => {
                let _ = app.search_input.pop();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                app.search_input.push(ch);
            }
            _ => {}
        }

        return KeypressOutcome::default();
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => KeypressOutcome {
            should_quit: true,
            review_toggled: None,
        },
        KeyCode::Left => {
            if move_file(-1, files, app) {
                app.refresh_search_matches_for_current_file(files);
            }
            KeypressOutcome::default()
        }
        KeyCode::Right => {
            if move_file(1, files, app) {
                app.refresh_search_matches_for_current_file(files);
            }
            KeypressOutcome::default()
        }
        KeyCode::Up => {
            move_scroll(-1, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Down => {
            move_scroll(1, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('h') => {
            if move_file(-1, files, app) {
                app.refresh_search_matches_for_current_file(files);
            }
            KeypressOutcome::default()
        }
        KeyCode::Char('l') => {
            if move_file(1, files, app) {
                app.refresh_search_matches_for_current_file(files);
            }
            KeypressOutcome::default()
        }
        KeyCode::Char('k') => {
            move_scroll(-1, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('j') => {
            move_scroll(1, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(-page_size, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(page_size, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::PageUp => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(-page_size, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::PageDown => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(page_size, files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Home => {
            scroll_to_top(app);
            KeypressOutcome::default()
        }
        KeyCode::End => {
            scroll_to_bottom(files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            scroll_to_bottom(files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('G') => {
            scroll_to_bottom(files, app, rows);
            KeypressOutcome::default()
        }
        KeyCode::Char('g') => {
            scroll_to_top(app);
            KeypressOutcome::default()
        }
        KeyCode::Char('/') => {
            app.enter_search_input_mode();
            KeypressOutcome::default()
        }
        KeyCode::Char('n') => {
            app.jump_to_search_match(files, rows, true);
            KeypressOutcome::default()
        }
        KeyCode::Char('N') => {
            app.jump_to_search_match(files, rows, false);
            KeypressOutcome::default()
        }
        KeyCode::Char('}') => {
            app.jump_to_hunk(files, rows, true);
            KeypressOutcome::default()
        }
        KeyCode::Char('{') => {
            app.jump_to_hunk(files, rows, false);
            KeypressOutcome::default()
        }
        KeyCode::Char('r') => {
            let reviewed = app.toggle_current_file_reviewed();
            KeypressOutcome {
                should_quit: false,
                review_toggled: Some((app.file_index, reviewed)),
            }
        }
        _ => KeypressOutcome::default(),
    }
}

pub(crate) fn handle_mouse(
    mouse: MouseEvent,
    files: &[DiffFileView],
    app: &mut AppState,
    columns: u16,
    rows: u16,
) {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let layout = create_frame_layout(columns, rows, max_lines);

    let row = mouse.row as usize;
    if row < layout.body_start_row || row > layout.body_end_row {
        return;
    }

    let column = mouse.column as usize;
    let hovered_pane = get_pane_for_column(column, &layout);

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                if let Some(pane) = hovered_pane {
                    move_horizontal(
                        pane,
                        -(MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize),
                        files,
                        app,
                        columns,
                        rows,
                    );
                }
            } else {
                move_scroll(-(MOUSE_WHEEL_SCROLL_LINES as isize), files, app, rows);
            }
        }
        MouseEventKind::ScrollDown => {
            if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                if let Some(pane) = hovered_pane {
                    move_horizontal(
                        pane,
                        MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize,
                        files,
                        app,
                        columns,
                        rows,
                    );
                }
            } else {
                move_scroll(MOUSE_WHEEL_SCROLL_LINES as isize, files, app, rows);
            }
        }
        MouseEventKind::ScrollLeft => {
            if let Some(pane) = hovered_pane {
                move_horizontal(
                    pane,
                    -(MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize),
                    files,
                    app,
                    columns,
                    rows,
                );
            }
        }
        MouseEventKind::ScrollRight => {
            if let Some(pane) = hovered_pane {
                move_horizontal(
                    pane,
                    MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize,
                    files,
                    app,
                    columns,
                    rows,
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, build_search_match_line_indexes, next_match_index};
    use crate::model::{DiffFileDescriptor, DiffFileView, FileContentSource, PaneOffsets};
    use std::collections::HashSet;

    fn create_test_file(left_lines: &[&str], right_lines: &[&str]) -> DiffFileView {
        DiffFileView {
            descriptor: DiffFileDescriptor {
                raw_status: "M".to_string(),
                display_path: "src/main.rs".to_string(),
                base_path: Some("src/main.rs".to_string()),
                head_path: Some("src/main.rs".to_string()),
                base_source: FileContentSource::Commit,
                head_source: FileContentSource::Commit,
            },
            review_key: "key".to_string(),
            left_lines: left_lines.iter().map(|line| line.to_string()).collect(),
            right_lines: right_lines.iter().map(|line| line.to_string()).collect(),
            left_language: Some("rust".to_string()),
            right_language: Some("rust".to_string()),
            left_deleted_line_indexes: HashSet::new(),
            right_added_line_indexes: HashSet::new(),
            left_max_content_length: 0,
            right_max_content_length: 0,
        }
    }

    #[test]
    fn search_matches_include_left_and_right_panes() {
        let file = create_test_file(
            &["alpha", "left-hit", "gamma"],
            &["one", "two", "right-hit"],
        );

        let left_matches = build_search_match_line_indexes(&file, "left");
        let right_matches = build_search_match_line_indexes(&file, "right");

        assert_eq!(left_matches, vec![1]);
        assert_eq!(right_matches, vec![2]);
    }

    #[test]
    fn next_match_index_wraps_both_directions() {
        assert_eq!(next_match_index(3, Some(2), true), Some(0));
        assert_eq!(next_match_index(3, Some(0), false), Some(2));
        assert_eq!(next_match_index(3, None, true), Some(0));
        assert_eq!(next_match_index(3, None, false), Some(2));
    }

    #[test]
    fn reviewed_toggle_updates_reviewed_count() {
        let mut app = AppState {
            file_index: 1,
            scroll_offset: 0,
            pane_offsets_by_file: vec![PaneOffsets::default(), PaneOffsets::default()],
            reviewed_by_file: vec![false, false],
            reviewed_count: 0,
            search_input_mode: false,
            search_query: String::new(),
            search_input: String::new(),
            search_match_line_indexes: Vec::new(),
            search_match_index: None,
        };

        let first = app.toggle_current_file_reviewed();
        let second = app.toggle_current_file_reviewed();

        assert!(first);
        assert!(!second);
        assert_eq!(app.reviewed_count(), 0);
    }
}
