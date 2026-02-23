//! Conceptual working example of the menu state machine pattern used in tetro-tui.
//!
//! This is a minimal, self-contained reproduction of the architecture found in:
//!   - src/application/mod.rs      (Application::run, Menu enum, MenuUpdate enum)
//!   - src/application/menus/*.rs  (individual run_menu_* methods)
//!
//! Run with: cargo run --example menu_state_machine
//!
//! ┌──────────────────────────────────────────────────────────────────┐
//! │                    HOW THE PATTERN WORKS                        │
//! │                                                                  │
//! │  Application::run() maintains a Vec<Menu> as a navigation stack. │
//! │  Each loop iteration:                                            │
//! │    1. Peek the top of the stack                                  │
//! │    2. Dispatch to the corresponding run_menu_*() method          │
//! │    3. The method returns a MenuUpdate (Push or Pop)              │
//! │    4. Push adds a new screen; Pop goes back                      │
//! │                                                                  │
//! │  Some menus (Title, PlayGame, GameOver) clear the stack when     │
//! │  pushed — they act as "root" screens that reset navigation.      │
//! │                                                                  │
//! │  The stack empties → the application exits.                      │
//! └──────────────────────────────────────────────────────────────────┘

use std::io::{self, Write};

use crossterm::{
    cursor::MoveTo,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    style::{Print, PrintStyledContent, Stylize},
    terminal::{self, Clear, ClearType},
    ExecutableCommand, QueueableCommand,
};

// ─── Menu enum ──────────────────────────────────────────────────────────────
// Mirrors: src/application/mod.rs:624-649
//
// Each variant represents a distinct screen/state in the application.
// Some variants carry data (like PlayGame carries game state in the real code).

enum Menu {
    Title,
    NewGame,
    PlayGame { mode_name: String },
    Pause { mode_name: String },
    Settings,
    GameOver { score: u32 },
    About,
    Quit,
}

impl std::fmt::Display for Menu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Menu::Title => write!(f, "Title Screen"),
            Menu::NewGame => write!(f, "New Game"),
            Menu::PlayGame { mode_name } => write!(f, "Playing: {mode_name}"),
            Menu::Pause { .. } => write!(f, "Paused"),
            Menu::Settings => write!(f, "Settings"),
            Menu::GameOver { score } => write!(f, "Game Over (score: {score})"),
            Menu::About => write!(f, "About"),
            Menu::Quit => write!(f, "Quit"),
        }
    }
}

// ─── MenuUpdate enum ───────────────────────────────────────────────────────
// Mirrors: src/application/mod.rs:677-681
//
// This is the only "signal" a menu can send back to the main loop.
// Push navigates forward; Pop navigates backward.

enum MenuUpdate {
    Pop,
    Push(Menu),
}

// ─── Application struct ────────────────────────────────────────────────────
// Mirrors: src/application/mod.rs:684-692
//
// Owns the terminal handle and all persistent state.
// Generic over Write so it could be tested with a Vec<u8> instead of Stdout.

struct Application<T: Write> {
    term: T,
}

impl<T: Write> Drop for Application<T> {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = self.term.execute(crossterm::cursor::Show);
        let _ = self.term.execute(terminal::LeaveAlternateScreen);
    }
}

impl<T: Write> Application<T> {
    fn new(term: T) -> Self {
        Application { term }
    }

    // ─── Main loop ─────────────────────────────────────────────────────
    // Mirrors: src/application/mod.rs:897-965
    //
    // This is the heart of the state machine. A Vec<Menu> acts as a
    // navigation stack. Each iteration dispatches to the current menu's
    // handler, then applies the returned MenuUpdate.
    //
    //   menu_stack: [Title]
    //     user picks "New Game" → Push(NewGame)
    //   menu_stack: [Title, NewGame]
    //     user picks "Marathon" → Push(PlayGame{..})
    //     PlayGame clears stack first (it's a "root" screen)
    //   menu_stack: [PlayGame{..}]
    //     user pauses → Push(Pause)
    //   menu_stack: [PlayGame{..}, Pause]
    //     user presses Esc → Pop
    //   menu_stack: [PlayGame{..}]
    //     game ends → Push(GameOver{..})
    //     GameOver clears stack first
    //   menu_stack: [GameOver{..}]

    fn run(&mut self) -> io::Result<()> {
        let mut menu_stack: Vec<Menu> = vec![Menu::Title];

        loop {
            // 1. Peek the top of the stack; exit if empty.
            let Some(menu) = menu_stack.last() else {
                break;
            };

            // 2. Dispatch to the appropriate handler.
            //    Each handler owns its own render + input loop and
            //    returns only when the user makes a navigation decision.
            let menu_update = match menu {
                Menu::Title => self.run_menu_title(),
                Menu::NewGame => self.run_menu_new_game(),
                Menu::PlayGame { mode_name } => {
                    let name = mode_name.clone();
                    self.run_menu_play_game(&name)
                }
                Menu::Pause { mode_name } => {
                    let name = mode_name.clone();
                    self.run_menu_pause(&name)
                }
                Menu::Settings => self.run_menu_settings(),
                Menu::GameOver { score } => {
                    let s = *score;
                    self.run_menu_game_over(s)
                }
                Menu::About => self.run_menu_about(),
                Menu::Quit => break,
            }?;

            // 3. Apply the navigation update.
            match menu_update {
                MenuUpdate::Pop => {
                    // Don't pop the last item — that would exit immediately.
                    // In the real code this keeps Title as a floor.
                    if menu_stack.len() > 1 {
                        menu_stack.pop();
                    }
                }
                MenuUpdate::Push(menu) => {
                    // Some menus are "root" screens that reset the stack.
                    // This prevents unbounded stack growth and provides
                    // clean entry points (you can't "go back" from Title).
                    if matches!(
                        menu,
                        Menu::Title | Menu::PlayGame { .. } | Menu::GameOver { .. }
                    ) {
                        menu_stack.clear();
                    }
                    menu_stack.push(menu);
                }
            }
        }

        Ok(())
    }

    // ─── Generic menu helper ───────────────────────────────────────────
    // Mirrors: src/application/menus/mod.rs:31-166
    //
    // A reusable "template method" that renders a list of choices and
    // handles arrow-key navigation. Individual menus call this with
    // their specific title and list of destination Menu variants.
    //
    // This is the DRY core — Pause, Settings, and other simple menus
    // delegate entirely to this method.

    fn generic_menu(&mut self, title: &str, selection: Vec<Menu>) -> io::Result<MenuUpdate> {
        let mut selected = 0usize;

        loop {
            // ── Render ──
            self.term
                .queue(Clear(ClearType::All))?
                .queue(MoveTo(2, 1))?
                .queue(PrintStyledContent(
                    format!("━━━ {} ━━━", title).bold().underlined(),
                ))?;

            let names: Vec<String> = selection.iter().map(|m| m.to_string()).collect();
            for (i, name) in names.iter().enumerate() {
                self.term.queue(MoveTo(4, 3 + i as u16))?;
                if i == selected {
                    self.term
                        .queue(PrintStyledContent(format!(">> {name} <<").bold()))?;
                } else {
                    self.term.queue(Print(format!("   {name}")))?;
                }
            }

            self.term
                .queue(MoveTo(2, 4 + names.len() as u16))?
                .queue(PrintStyledContent(
                    "  [↑/↓] Navigate  [Enter] Select  [Esc] Back"
                        .italic()
                        .dark_grey(),
                ))?;

            self.term.flush()?;

            // ── Input ──
            match event::read()? {
                // Ctrl+C → force quit
                Event::Key(KeyEvent {
                    code: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press,
                    ..
                }) => break Ok(MenuUpdate::Push(Menu::Quit)),

                // Esc → go back (Pop)
                Event::Key(KeyEvent {
                    code: KeyCode::Esc,
                    kind: KeyEventKind::Press,
                    ..
                }) => break Ok(MenuUpdate::Pop),

                // Enter → select current item (Push)
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    if !selection.is_empty() {
                        let menu = selection.into_iter().nth(selected).unwrap();
                        break Ok(MenuUpdate::Push(menu));
                    }
                }

                // Up arrow
                Event::Key(KeyEvent {
                    code: KeyCode::Up,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => {
                    if !selection.is_empty() {
                        selected = (selected + selection.len() - 1) % selection.len();
                    }
                }

                // Down arrow
                Event::Key(KeyEvent {
                    code: KeyCode::Down,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }) => {
                    if !selection.is_empty() {
                        selected = (selected + 1) % selection.len();
                    }
                }

                _ => {}
            }
        }
    }

    // ─── Individual menu handlers ──────────────────────────────────────
    // Each mirrors a file in src/application/menus/*.rs
    //
    // Simple menus just call generic_menu() with their choices.
    // Complex menus (PlayGame) have their own render+input loop.

    /// Title screen — the root menu.
    /// Mirrors: src/application/menus/title.rs
    fn run_menu_title(&mut self) -> io::Result<MenuUpdate> {
        self.generic_menu(
            "TETRO-TUI",
            vec![
                Menu::NewGame,
                Menu::Settings,
                Menu::About,
                Menu::Quit,
            ],
        )
    }

    /// Game mode selection.
    /// Mirrors: src/application/menus/new_game.rs
    fn run_menu_new_game(&mut self) -> io::Result<MenuUpdate> {
        self.generic_menu(
            "New Game",
            vec![
                Menu::PlayGame {
                    mode_name: "Marathon".into(),
                },
                Menu::PlayGame {
                    mode_name: "40-Lines".into(),
                },
                Menu::PlayGame {
                    mode_name: "Time Trial".into(),
                },
            ],
        )
    }

    /// Live gameplay — this is the "complex" menu with its own loop.
    /// Mirrors: src/application/menus/play_game.rs
    ///
    /// In the real code this spawns an input thread, runs the game engine,
    /// and renders frames at a fixed interval. Here we simulate it.
    fn run_menu_play_game(&mut self, mode_name: &str) -> io::Result<MenuUpdate> {
        let mut score: u32 = 0;
        let mut tick = 0u32;

        loop {
            // ── Simulate game state ──
            tick += 1;
            if tick % 20 == 0 {
                score += 100;
            }

            // ── Render ──
            self.term
                .queue(Clear(ClearType::All))?
                .queue(MoveTo(2, 1))?
                .queue(PrintStyledContent(
                    format!("Playing: {mode_name}").bold(),
                ))?
                .queue(MoveTo(2, 3))?
                .queue(Print(format!("Score: {score}")))?
                .queue(MoveTo(2, 4))?
                .queue(Print(format!("Tick:  {tick}")))?
                .queue(MoveTo(2, 6))?
                .queue(PrintStyledContent(
                    "[Esc] Pause   [Q] End game".italic().dark_grey(),
                ))?;
            self.term.flush()?;

            // ── Input (non-blocking poll) ──
            if event::poll(std::time::Duration::from_millis(100))? {
                match event::read()? {
                    // Ctrl+C → force quit
                    Event::Key(KeyEvent {
                        code: KeyCode::Char('c'),
                        modifiers: KeyModifiers::CONTROL,
                        kind: KeyEventKind::Press,
                        ..
                    }) => break Ok(MenuUpdate::Push(Menu::Quit)),

                    // Esc → pause (push Pause menu on top of PlayGame)
                    Event::Key(KeyEvent {
                        code: KeyCode::Esc,
                        kind: KeyEventKind::Press,
                        ..
                    }) => {
                        break Ok(MenuUpdate::Push(Menu::Pause {
                            mode_name: mode_name.to_string(),
                        }));
                    }

                    // Q → end game (transitions to GameOver, which clears stack)
                    Event::Key(KeyEvent {
                        code: KeyCode::Char('q'),
                        kind: KeyEventKind::Press,
                        ..
                    }) => {
                        break Ok(MenuUpdate::Push(Menu::GameOver { score }));
                    }

                    _ => {}
                }
            }
        }
    }

    /// Pause menu — pushed on top of PlayGame.
    /// Mirrors: src/application/menus/pause.rs
    ///
    /// Pop returns to PlayGame. Push(NewGame) or Push(Title) starts fresh.
    fn run_menu_pause(&mut self, _mode_name: &str) -> io::Result<MenuUpdate> {
        // In the real code, pause.rs is just:
        //   self.generic_menu("Game Paused", selection)
        self.generic_menu(
            "Game Paused",
            vec![
                Menu::NewGame,
                Menu::Settings,
                Menu::About,
                Menu::Quit,
            ],
        )
        // If user presses Esc → Pop → returns to PlayGame (resumes game)
        // If user picks NewGame → Push(NewGame) → starts fresh game flow
    }

    /// Settings — a submenu.
    /// Mirrors: src/application/menus/settings.rs
    fn run_menu_settings(&mut self) -> io::Result<MenuUpdate> {
        self.generic_menu(
            "Settings",
            vec![], // Empty = "nothing here yet" (matches real code behavior)
        )
    }

    /// Game over screen — a "root" screen that clears the stack.
    /// Mirrors: src/application/menus/game_ended.rs
    fn run_menu_game_over(&mut self, score: u32) -> io::Result<MenuUpdate> {
        self.generic_menu(
            &format!("Game Over — Score: {score}"),
            vec![
                Menu::Title,   // Goes back to title (clears stack)
                Menu::NewGame, // Play again
                Menu::Quit,
            ],
        )
    }

    /// About screen.
    /// Mirrors: src/application/menus/about.rs
    fn run_menu_about(&mut self) -> io::Result<MenuUpdate> {
        self.generic_menu(
            "About",
            vec![], // Shows info, Esc to go back
        )
    }
}

// ─── Entry point ───────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    // Terminal setup (mirrors Application::new in mod.rs:733+)
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(terminal::EnterAlternateScreen)?;
    stdout.execute(crossterm::cursor::Hide)?;

    // Run the application — the entire UX is driven by the menu stack loop.
    let mut app = Application::new(stdout);
    app.run()

    // Drop handler restores terminal (mirrors Application::drop in mod.rs:694+)
}
