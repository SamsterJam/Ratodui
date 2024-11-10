// src/main.rs

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use directories::ProjectDirs;
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Gauge},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    io,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Todo {
    name: String,
    progress: u16, // Progress in percentage (0 - 100)
}

enum Event<I> {
    Input(I),
    Tick,
}

fn main() -> Result<(), Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Channel to receive input events
    let (tx, rx) = mpsc::channel();
    let tick_rate = Duration::from_millis(250);
    let tx_clone = tx.clone();
    thread::spawn(move || {
        let mut last_tick = Instant::now();
        loop {
            // Poll for event
            let timeout = tick_rate
                .checked_sub(last_tick.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            if event::poll(timeout).unwrap() {
                match event::read().unwrap() {
                    CEvent::Mouse(mouse_event) => {
                        tx_clone
                            .send(Event::Input(CEvent::Mouse(mouse_event)))
                            .unwrap();
                    }
                    CEvent::Key(key_event) => {
                        tx_clone
                            .send(Event::Input(CEvent::Key(key_event)))
                            .unwrap();

                        if key_event.code == KeyCode::Char('q') {
                            break; // Exit the thread on 'q'
                        }
                    }
                    _ => {}
                }
            }
            if last_tick.elapsed() >= tick_rate {
                tx_clone.send(Event::Tick).unwrap();
                last_tick = Instant::now();
            }
        }
    });

    // Initialize todos
    let mut todos = load_todos();

    // If no todos were loaded, initialize with defaults
    if todos.is_empty() {
        todos = vec![
            Todo {
                name: String::from("TodoName1"),
                progress: 20,
            },
            Todo {
                name: String::from("TodoName2"),
                progress: 32,
            },
        ];
    }

    // Variables for mouse interaction
    let mut dragging = false;
    let mut drag_index = None;

    // Variables for editing todo names
    let mut editing_index: Option<usize> = None;
    let mut input_buffer = String::new();

    // Main loop
    loop {
        // Get the terminal size
        let size = terminal.size()?;
        // Compute the layout chunks
        let chunks = compute_chunks(size, &todos);

        // Rendering
        terminal.draw(|f| {
            ui(f, &todos, editing_index, &input_buffer);
        })?;

        // Event handling
        match rx.recv()? {
            Event::Input(event) => match event {
                CEvent::Key(key_event) => {
                    if let Some(i) = editing_index {
                        // We are editing a todo name
                        match key_event.code {
                            KeyCode::Char(c) => {
                                input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                input_buffer.pop();
                            }
                            KeyCode::Enter => {
                                // Update the todo's name and exit edit mode
                                todos[i].name = input_buffer.clone();
                                input_buffer.clear();
                                editing_index = None;
                                // Save the todos after renaming
                                save_todos(&todos);
                            }
                            KeyCode::Esc => {
                                // Cancel editing
                                input_buffer.clear();
                                editing_index = None;
                            }
                            _ => {}
                        }
                    } else {
                        // Not editing - existing code
                        if key_event.code == KeyCode::Char('q') {
                            break; // Exit the main loop
                        }
                        // Handle other key events if needed
                    }
                }
                CEvent::Mouse(mouse_event) => match mouse_event.kind {
                    MouseEventKind::Down(button) => {
                        if button == MouseButton::Left {
                            // Get the mouse position
                            let mouse_pos = (mouse_event.column, mouse_event.row);
                            let mut clicked_on_todo = false;
                            // Check if click is on any todo item
                            for (i, chunk) in chunks.iter().enumerate() {
                                if i >= todos.len() {
                                    break;
                                }
                                if is_inside(mouse_pos, *chunk) {
                                    clicked_on_todo = true;
                                    if mouse_event.row == chunk.y {
                                        // Clicked on the title line - start editing
                                        editing_index = Some(i);
                                        input_buffer = todos[i].name.clone();
                                    } else if editing_index.is_none() {
                                        // Start dragging to update progress
                                        dragging = true;
                                        drag_index = Some(i);
                                        update_progress(&mut todos[i], *chunk, mouse_event.column);
                                        // Save the todos after updating progress
                                        save_todos(&todos);
                                    }
                                    break;
                                }
                            }
                            // Check if click is on the add button
                            if !clicked_on_todo {
                                if let Some(add_button_rect) = chunks.get(todos.len()) {
                                    if is_inside(mouse_pos, *add_button_rect) {
                                        // Add a new todo
                                        todos.push(Todo {
                                            name: format!("TodoName{}", todos.len() + 1),
                                            progress: 0,
                                        });
                                        // Save the todos after adding a new one
                                        save_todos(&todos);
                                    }
                                }
                            }
                        }
                    }
                    MouseEventKind::Drag(button) => {
                        if editing_index.is_none() && dragging && button == MouseButton::Left {
                            if let Some(i) = drag_index {
                                let chunk = chunks[i];
                                update_progress(&mut todos[i], chunk, mouse_event.column);
                                // Save the todos after updating progress
                                save_todos(&todos);
                            }
                        }
                    }
                    MouseEventKind::Up(button) => {
                        if button == MouseButton::Left {
                            dragging = false;
                            drag_index = None;
                        }
                    }
                    _ => {}
                },
                _ => {}
            },
            Event::Tick => {}
        }
    }

    // Before exiting, save the todos
    save_todos(&todos);

    // Cleanup before exiting
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

// Function to render the UI
fn ui<B: Backend>(
    f: &mut ratatui::Frame<B>,
    todos: &[Todo],
    editing_index: Option<usize>,
    input_buffer: &str,
) {
    let chunks = compute_chunks(f.size(), todos);

    for (i, todo) in todos.iter().enumerate() {
        let mut title = String::new();
        let mut style = Style::default();

        if editing_index == Some(i) {
            // Render input buffer with a cursor
            title = format!("{}_", input_buffer); // Add cursor
            style = Style::default().fg(Color::Yellow);
        } else {
            title = todo.name.clone();
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Span::styled(title, style));

        let gauge = Gauge::default()
            .block(block)
            .gauge_style(Style::default().fg(Color::Blue).bg(Color::Black))
            .percent(todo.progress);

        f.render_widget(gauge, chunks[i]);
    }

    // Render the add button
    let add_button = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            "[    +    ]",
            Style::default().fg(Color::Green),
        ));
    f.render_widget(add_button, chunks[todos.len()]);
}

// Helper function to compute chunks based on the terminal size and todos
fn compute_chunks(size: Rect, todos: &[Todo]) -> Vec<Rect> {
    let mut constraints: Vec<Constraint> = Vec::new();

    for _ in todos {
        constraints.push(Constraint::Length(3)); // Each todo takes up 3 rows
    }

    // Add constraint for the add button
    constraints.push(Constraint::Length(3));

    Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(constraints)
        .split(size)
        .to_vec() // Convert Rc<[Rect]> to Vec<Rect>
}

// Function to update the progress of a todo based on mouse x position
fn update_progress(todo: &mut Todo, area: Rect, mouse_x: u16) {
    // Calculate new progress based on mouse_x position within the area
    let progress = ((mouse_x.saturating_sub(area.x)) * 100 / area.width.max(1)).min(100) as u16;
    if todo.progress != progress {
        todo.progress = progress;
    }
}

// Helper function to check if a point is inside a rectangle
fn is_inside(pos: (u16, u16), area: Rect) -> bool {
    pos.0 >= area.x
        && pos.0 < area.x + area.width
        && pos.1 >= area.y
        && pos.1 < area.y + area.height
}

// Function to load todos from a JSON file
fn load_todos() -> Vec<Todo> {
    let mut todos = Vec::new();

    if let Some(proj_dirs) = ProjectDirs::from("com", "todo", "todo") {
        let data_dir = proj_dirs.data_dir();
        let file_path = data_dir.join("todos.json");

        if let Ok(contents) = fs::read_to_string(&file_path) {
            if let Ok(loaded_todos) = serde_json::from_str::<Vec<Todo>>(&contents) {
                todos = loaded_todos;
            }
        }
    }

    todos
}

// Function to save todos to a JSON file
fn save_todos(todos: &Vec<Todo>) {
    if let Some(proj_dirs) = ProjectDirs::from("com", "todo", "todo") {
        let data_dir = proj_dirs.data_dir();

        // Create directories if they don't exist
        if let Err(e) = fs::create_dir_all(&data_dir) {
            eprintln!("Failed to create data directory: {}", e);
            return;
        }

        let file_path = data_dir.join("todos.json");

        match serde_json::to_string_pretty(&todos) {
            Ok(json) => {
                if let Err(e) = fs::write(&file_path, json) {
                    eprintln!("Failed to write to file: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to serialize todos: {}", e);
            }
        }
    }
}