use std::io::{self, IsTerminal};

use anyhow::{Context, Result, bail};
use crossterm::{
    cursor::{Hide, Show},
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    text::Text,
    widgets::{Clear, Paragraph},
};

use crate::{
    app::{AppState, handle_keypress, handle_mouse},
    model::{DiffFileView, ResolvedComparison},
    render::render_frame,
    review::ReviewStore,
};

fn draw_app<B: Backend>(
    terminal: &mut Terminal<B>,
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    app: &mut AppState,
) -> Result<()> {
    let size = terminal.size()?;
    let render_output = render_frame(
        files,
        comparison,
        app.file_index,
        app.scroll_offset,
        app.current_offsets(),
        app.reviewed_count(),
        app.is_current_file_reviewed(),
        app.search_status_text(),
        size.width,
        size.height,
    );

    app.scroll_offset = app.scroll_offset.min(render_output.max_scroll);
    app.set_current_offsets(render_output.clamped_pane_offsets);

    let text = Text::from(render_output.lines);
    terminal.draw(move |frame| {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(text), area);
    })?;

    Ok(())
}

fn run_event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    review_store: &mut ReviewStore,
) -> Result<()> {
    let initial_reviewed = review_store.reviewed_flags_for_files(files);
    let mut app = AppState::new(files.len(), initial_reviewed);
    draw_app(terminal, files, comparison, &mut app)?;

    loop {
        match event::read().context("failed to read terminal event")? {
            Event::Key(key) => {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }

                let (_, rows) =
                    crossterm::terminal::size().context("failed to read terminal size")?;
                let outcome = handle_keypress(key, files, &mut app, rows);

                if let Some((file_index, reviewed)) = outcome.review_toggled {
                    review_store.set_reviewed(&files[file_index].review_key, reviewed);
                    review_store.persist()?;
                }

                if outcome.should_quit {
                    break;
                }
            }
            Event::Mouse(mouse) => {
                let (columns, rows) =
                    crossterm::terminal::size().context("failed to read terminal size")?;
                handle_mouse(mouse, files, &mut app, columns, rows);
            }
            Event::Resize(_, _) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
        }

        draw_app(terminal, files, comparison, &mut app)?;
    }

    Ok(())
}

pub(crate) fn start_interactive_review(
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    mut review_store: ReviewStore,
) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Interactive TTY is required to run deff");
    }

    enable_raw_mode().context("failed to enable raw mode")?;

    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide) {
        let _ = disable_raw_mode();
        return Err(error).context("failed to initialize terminal UI");
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let mut cleanup_stdout = io::stdout();
            let _ = execute!(
                cleanup_stdout,
                Show,
                DisableMouseCapture,
                LeaveAlternateScreen
            );
            return Err(error).context("failed to build terminal backend");
        }
    };

    let run_result = run_event_loop(&mut terminal, files, comparison, &mut review_store);

    let mut restore_error: Option<anyhow::Error> = None;
    if let Err(error) = disable_raw_mode() {
        restore_error = Some(error.into());
    }
    if let Err(error) = execute!(
        terminal.backend_mut(),
        Show,
        DisableMouseCapture,
        LeaveAlternateScreen
    ) {
        if restore_error.is_none() {
            restore_error = Some(error.into());
        }
    }
    if let Err(error) = terminal.show_cursor() {
        if restore_error.is_none() {
            restore_error = Some(error.into());
        }
    }

    if let Some(error) = restore_error {
        return Err(error).context("failed to restore terminal state");
    }

    run_result
}
