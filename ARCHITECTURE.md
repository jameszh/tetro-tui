# Tetro-TUI Architecture Review

## Technologies

| Technology | Version | Purpose |
|---|---|---|
| **Rust** | Edition 2021, MSRV 1.87.0 | Core language |
| **falling-tetromino-engine** | 1.0.0 | External game engine crate (game logic, rules, piece mechanics) |
| **crossterm** | 0.27.0 | Cross-platform terminal I/O (input events, cursor control, colors) |
| **serde / serde_json** | 1.0 | Save file serialization (settings, replays, scores) |
| **clap** | 4.5.9 | CLI argument parsing (`--seed`, `--board`) |
| **chrono** | 0.4.38 | Timestamps on game metadata |
| **dirs** | 6.0.0 | OS-appropriate config directory discovery |
| **rand** | 0.9.2 | RNG (ChaCha12Rng) for piece generation |

No GUI framework is used — the application directly controls the terminal via crossterm, which gives fine-grained control over rendering and input at the cost of implementing its own UI framework.

---

## Project Structure

```
tetro-tui/
├── Cargo.toml
├── src/
│   ├── main.rs                         # Entry point + CLI arg parsing
│   ├── application/
│   │   ├── mod.rs                      # Core Application struct & state machine
│   │   └── menus/                      # Menu implementations
│   │       ├── mod.rs                  # Generic menu UI framework
│   │       ├── title.rs                # Title screen
│   │       ├── new_game.rs             # Game mode selection and settings
│   │       ├── play_game.rs            # Main game loop for live gameplay
│   │       ├── pause.rs                # Pause menu
│   │       ├── replay_game.rs          # Replay playback and controls
│   │       ├── settings.rs             # Settings selection menu
│   │       ├── adjust_graphics.rs      # Graphics customization
│   │       ├── adjust_keybinds.rs      # Keybind configuration
│   │       ├── adjust_gameplay.rs      # Gameplay mechanics tuning
│   │       ├── scores_and_replays.rs   # Game history/replay browser
│   │       ├── game_ended.rs           # Game over/completion screen
│   │       └── about.rs               # About screen
│   ├── game_renderers/                 # Terminal rendering implementations
│   │   ├── mod.rs                      # Renderer trait definition
│   │   ├── diff_print.rs              # Main optimized diff-based renderer
│   │   ├── braille.rs                 # Braille character renderer
│   │   └── legacy_debug.rs            # Debug renderer
│   ├── game_mode_presets/              # Game mode definitions
│   │   ├── mod.rs                      # Standard modes (40-Lines, Marathon, etc.)
│   │   └── game_modifiers/            # Modular game variant system
│   │       ├── mod.rs                  # Modifier reconstruction and composition
│   │       ├── puzzle.rs              # Puzzle mode (24 hand-crafted puzzles)
│   │       ├── cheese.rs             # Cheese mode (gap-filled lines)
│   │       ├── combo_board.rs         # Combo mode (4-wide playfield)
│   │       └── ascent.rs             # Ascent experimental mode
│   ├── keybinds_presets.rs            # Keybind presets (Default, Vim, Guideline)
│   ├── palette_presets.rs             # 10 color palette implementations
│   ├── live_input_handler.rs          # Background input thread handler
│   ├── fmt_helpers.rs                 # Formatting utilities
│   └── assets/                        # Game artwork and resources
└── examples/
    └── menu_state_machine.rs          # Conceptual working example
```

---

## High-Level Architecture

The application follows a **layered architecture** with three distinct layers:

```
┌─────────────────────────────────────────┐
│          Application Layer              │
│  (Menu stack, navigation, settings)     │
├─────────────────────────────────────────┤
│          Game Layer                     │
│  (falling-tetromino-engine + modifiers) │
├─────────────────────────────────────────┤
│          Rendering / I/O Layer          │
│  (Renderer trait, crossterm, input)     │
└─────────────────────────────────────────┘
```

**Entry point** (`src/main.rs`): Parses CLI args via clap, loads/creates save data, constructs `Application<Stdout>`, calls `run()`, and persists state on exit.

**Core struct** (`src/application/mod.rs`): `Application<T: Write>` owns all mutable state — settings, scores, game saves, and the terminal handle. It drives a menu-stack loop that dispatches to specific `run_menu_*()` methods.

---

## Design Patterns

### 1. State Machine with Menu Stack

The central navigation pattern. `Application::run()` maintains a `Vec<Menu>` stack:

```
Application::run()
  ├─ Push Menu::Title
  └─ Loop:
     ├─ Peek current menu
     ├─ Call run_menu_*() → MenuUpdate
     ├─ MenuUpdate::Pop  → pop stack
     └─ MenuUpdate::Push → push new menu (some menus clear stack first)
```

The `Menu` enum (`src/application/menus/mod.rs`) has 13 variants including `Title`, `PlayGame`, `Pause`, `Settings`, `GameOver`, `ReplayGame`, etc. Certain menus (Title, PlayGame, GameOver) clear the stack when pushed, preventing deep nesting.

#### Core types

```rust
// src/application/mod.rs:624-649
enum Menu {
    Title,
    NewGame,
    PlayGame { game, game_input_history, game_meta_data, game_renderer },
    Pause,
    Settings,
    AdjustGraphics,
    AdjustKeybinds,
    AdjustGameplay,
    GameOver(ScoresEntry),
    GameComplete(ScoresEntry),
    ScoresAndReplays,
    ReplayGame { game_restoration_data, game_meta_data, replay_length, game_renderer },
    About,
    Quit,
}

// src/application/mod.rs:677-681
enum MenuUpdate {
    Pop,
    Push(Menu),
}
```

#### Main loop (`src/application/mod.rs:897-965`)

```rust
pub fn run(&mut self) -> io::Result<()> {
    let mut menu_stack = vec![Menu::Title];
    loop {
        let Some(menu) = menu_stack.last_mut() else { break; };
        let menu_update = match menu {
            Menu::Title           => self.run_menu_title(),
            Menu::NewGame         => self.run_menu_new_game(),
            Menu::PlayGame { .. } => self.run_menu_play_game(..),
            Menu::Pause           => self.run_menu_pause(),
            Menu::Settings        => self.run_menu_settings(),
            // ... all other variants
            Menu::Quit            => break,
        }?;
        match menu_update {
            MenuUpdate::Pop => {
                if menu_stack.len() > 1 {
                    menu_stack.pop();
                }
            }
            MenuUpdate::Push(menu) => {
                // "Root" screens reset the navigation stack
                if matches!(menu, Menu::Title | Menu::PlayGame{..} | Menu::GameOver(_) | Menu::GameComplete(_)) {
                    menu_stack.clear();
                }
                menu_stack.push(menu);
            }
        }
    }
    Ok(())
}
```

#### Navigation flow example

```
Action                          Stack after
──────────────────────────────  ──────────────────────────
Start                           [Title]
User picks "New Game"           [Title, NewGame]
User picks "Marathon"           [PlayGame("Marathon")]     ← stack cleared (root screen)
User presses Esc                [PlayGame, Pause]          ← pushed on top
User presses Esc again          [PlayGame]                 ← Pop returns to game
Game ends                       [GameOver(score)]          ← stack cleared (root screen)
User picks "Title"              [Title]                    ← stack cleared (root screen)
User picks "Quit"               []                         ← exit
```

### 2. Strategy Pattern — Renderer Trait

Rendering is abstracted behind a trait (`src/game_renderers/mod.rs`):

```rust
pub trait Renderer: Default {
    fn push_game_feedback_msgs(...);
    fn render(...) -> io::Result<()>;
}
```

Three implementations exist:
- **DiffPrintRenderer** (`diff_print.rs`) — Primary renderer with double-buffering and diff-based updates. Tracks `prev`/`next` screen buffers as `Vec<Vec<(char, Option<Color>)>>` and only writes changed cells. Uses crossterm's `BeginSynchronizedUpdate`/`EndSynchronizedUpdate` for flicker-free output.
- **BrailleRenderer** (`braille.rs`) — Uses Unicode braille characters for a compact display.
- **LegacyDebugRenderer** (`legacy_debug.rs`) — Plain text debug output.

### 3. Builder Pattern

Game construction uses `falling_tetromino_engine::GameBuilder` with method chaining to configure rotation systems, generators, gravity, win conditions, etc. before producing a `Game` instance.

### 4. Modifier / Decorator Pattern

Game variants (Puzzle, Cheese, Combo, Ascent) are implemented as composable modifiers (`src/game_mode_presets/game_modifiers/`). Each modifier implements the engine's `Modifier` trait with a `descriptor` (for serialization) and a `mod_function` (callback hook). Modifiers can be stacked and are reconstructed from saved descriptors for replay.

### 5. Parameterized Helper — Generic Menu

`generic_menu()` in `src/application/menus/mod.rs` is a shared helper that encapsulates the render-poll-update loop for all simple selection menus. Callers pass different data (title + choices) rather than overriding behavior via subclassing.

This is **not** the Template Method pattern (which requires inheritance/trait overrides). It is straightforward **parameterized reuse** — the varying parts are injected as function arguments:

```rust
// src/application/menus/pause.rs — the entire file
fn run_menu_pause(&mut self) -> io::Result<MenuUpdate> {
    let selection = vec![Menu::NewGame, Menu::Settings, Menu::ScoresAndReplays, Menu::About, Menu::Quit];
    self.generic_menu("Game Paused", selection)
}
```

A true Template Method in Rust would use a trait with default methods and overridable steps — something this codebase intentionally avoids in favor of the simpler data-driven approach.

### 6. Observer Pattern — Feedback System

The game engine emits `Feedback` events (line clears, T-spins, combos, etc.) that the renderer consumes via `push_game_feedback_msgs()`. This decouples game logic from visual effects.

### 7. Channel-based Concurrency

Live gameplay uses a **two-thread model** (`src/live_input_handler.rs`):
- **Background thread**: Polls terminal events via `crossterm::event::read()`, translates them to `LiveTermSignal` values.
- **Main thread**: Receives signals via `mpsc::channel()`, feeds button changes to the game engine, and runs the render loop on a fixed frame budget.

This prevents input polling from blocking the render loop.

### Summary of patterns

| Pattern | Location | Purpose |
|---|---|---|
| **State Machine** | `Application::run()` | Menu navigation via stack |
| **Strategy** | Renderer trait | Pluggable rendering implementations |
| **Builder** | `falling_tetromino_engine::GameBuilder` | Game construction with chaining |
| **Modifier** (Decorator-like) | `game_modifiers/*` | Composable game rule variations |
| **Parameterized Helper** | `menus::generic_menu()` | Shared menu rendering loop via data injection |
| **Observer** | Feedback system | Game events → UI messages |
| **Enum Dispatch** | `Menu` enum | Runtime polymorphism without `dyn` trait |
| **Double Buffering** | `DiffPrintRenderer` | Optimized terminal updates |
| **Channel-based IPC** | `play_game` + `mpsc` | Thread-safe input communication |

---

## Data Architecture

### Settings System

Settings are organized into **named slots** — users can create, switch, and lock multiple presets:

- **Graphics**: Glyphset (Unicode/ASCII/Electronika), palette, FPS, effects, shadow piece, blindfold mode
- **Keybinds**: 4 built-in presets (Default, Finesse, Vim, Guideline) + custom slots
- **Gameplay**: Rotation system, DAS/ARR/SDF, spawn delay, preview count, prespawn actions
- **Palettes**: 10 color presets (Monochrome, Fullcolor, Gruvbox, Solarized, etc.)

### Save / Replay System

Persistence uses JSON serialization with granularity levels:
- `NoSavefile` | `RememberSettings` | `RememberSettingsScores` | `RememberSettingsScoresReplays`

Replays store a `CompressedInputHistory` — input events packed into `Vec<u128>` using bit manipulation (5 bits for button change + remaining bits for timestamp). Game state at any point can be deterministically rebuilt by replaying inputs through the engine. An anchor system saves periodic snapshots for fast seeking during replay playback.

Save file location: `{config_dir}/.tetro-tui_1.1.0_savefile.json`

---

## Game Loop Architecture

### Live Gameplay (`run_menu_play_game`)

```
run_menu_play_game()
  ├─ Spawn input handler thread (mpsc channel)
  ├─ Initialize renderer
  ├─ Set frame interval from FPS setting
  └─ Main loop until game.result() != None:
     ├─ Poll input_receiver for new inputs
     ├─ Convert inputs → ButtonChange → game.update()
     ├─ Collect feedback messages from game
     ├─ game_renderer.render() with diff-tracking
     ├─ term.flush() to terminal
     └─ Sleep until next frame deadline
  └─ Transition to GameOver/Complete menu
```

### Replay Playback (`run_menu_replay_game`)

- Deterministic replay via stored input history
- `GameRestorationData::restore(input_index)` replays inputs up to a given point
- Adjustable speed (0.05× – ∞)
- Anchor system: saves periodic game state snapshots for fast seeking

---

## Crossterm Extension Traits

`std::io::Stdout` does not have `execute()` or `queue()` natively. These come from crossterm's blanket-implemented extension traits:

```rust
// crossterm provides:
pub trait ExecutableCommand: Write {
    fn execute(&mut self, command: impl Command) -> io::Result<&mut Self>;
}
pub trait QueueableCommand: Write {
    fn queue(&mut self, command: impl Command) -> io::Result<&mut Self>;
}

// Blanket impl for anything that implements Write:
impl<T: Write> ExecutableCommand for T { ... }
impl<T: Write> QueueableCommand for T { ... }
```

- `execute()` — writes and flushes the command immediately
- `queue()` — writes the command to the internal buffer; requires a manual `flush()` call later (used for batching multiple terminal operations)

---

## Strengths

1. **Clean separation of concerns** — Game logic lives entirely in the external engine crate. The application layer handles UX, and the rendering layer is swappable via traits.
2. **Efficient terminal rendering** — The diff-based renderer with synchronized updates avoids full-screen redraws every frame.
3. **Strong Rust idioms** — Extensive use of enums for dispatch, pattern matching, ownership semantics, and generics (`Application<T: Write>` enables testing with non-terminal writers).
4. **Composable game modes** — The modifier system allows new game variants without modifying core logic. Modifiers serialize/deserialize cleanly for replay support.
5. **Deterministic replays** — Storing compressed inputs rather than game states keeps replays compact while enabling full state reconstruction.
6. **Robust configuration** — The slot-based preset system with locked defaults gives users flexibility without risking loss of built-in presets.

## Areas for Consideration

1. **`application/mod.rs` is large (~967 lines)** — It handles application state, save/load logic, settings management, and menu dispatch. Some of this could be extracted into dedicated modules.
2. **No abstraction over crossterm** — Terminal operations are called directly throughout menu code. A thin wrapper could simplify testing and make it easier to swap terminal backends.
3. **`Menu` enum carries embedded state** — Variants like `PlayGame` and `ReplayGame` hold full game state inline. This works but makes the enum large and structurally heterogeneous.
4. **Game mode presets use `Box<dyn Fn>`** — The type alias `(String, (Stat, bool), Box<dyn Fn(&GameBuilder) -> Game>)` is opaque. A named struct could improve readability.

---

## Working Example

A self-contained, compilable example of the menu state machine pattern is available at:

```
examples/menu_state_machine.rs
```

Run it with:

```bash
cargo run --example menu_state_machine
```

It demonstrates the full `Vec<Menu>` stack loop, `MenuUpdate::Push`/`Pop` signals, root-screen stack clearing, the `generic_menu()` template method, and a simulated gameplay loop with non-blocking input — all in ~450 lines with no dependency on the game engine.
