//! Interactive terminal UI.
//!
//! [`run`] owns the terminal for the lifetime of the session: it switches to
//! the alternate screen with raw mode and mouse capture, drives the event
//! loop, and restores the terminal on exit or panic.

pub mod app;
pub mod view;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui_core::terminal::Terminal;
use ratatui_crossterm::CrosstermBackend;

use crate::tree::ScanEvent;
pub use app::App;

type Term = Terminal<CrosstermBackend<Stdout>>;

/// Run the interactive explorer until the user quits.
///
/// `rx` delivers scan progress and finally the tree; `root` is the directory
/// being scanned (shown while the scan is in flight).
pub fn run(rx: Receiver<ScanEvent>, root: PathBuf) -> io::Result<()> {
    let mut app = App::new(root);
    let mut terminal = init_terminal()?;
    let result = event_loop(&mut terminal, &mut app, rx);
    restore_terminal();
    result
}

fn init_terminal() -> io::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(e);
    }
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn event_loop(terminal: &mut Term, app: &mut App, rx: Receiver<ScanEvent>) -> io::Result<()> {
    let mut rx = Some(rx);
    loop {
        if let Some(receiver) = &rx {
            loop {
                match receiver.try_recv() {
                    Ok(event) => app.apply(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        if !app.is_ready() && !matches!(app.phase, app::Phase::Failed(_)) {
                            app.apply(ScanEvent::Error(
                                "the scanner stopped without producing a result".into(),
                            ));
                        }
                        rx = None;
                        break;
                    }
                }
            }
        }

        terminal.draw(|frame| view::draw(frame, app))?;
        app.tick = app.tick.wrapping_add(1);

        let timeout = if rx.is_some() && !app.is_ready() {
            Duration::from_millis(80)
        } else {
            Duration::from_millis(500)
        };
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                    app.on_key(key);
                }
                Event::Mouse(mouse) => app.on_mouse(mouse),
                _ => {}
            }
        }
        if app.quit {
            return Ok(());
        }
    }
}
