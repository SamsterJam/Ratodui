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
    widgets::{Paragraph, Wrap},
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
                        // Removed the break condition here
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

    // If no todos were loaded, initialize with a new todo
    if todos.is_empty() {
        todos.push(Todo {
            name: String::from("New Todo"),
            progress: 0,
        });
    }

    // Variables for mouse interaction
    let mut dragging = false;
    let mut drag_index = None;

    // Variables for editing todo names
    let mut editing_index: Option<usize> = None;
    let mut input_buffer = String::new();
    let mut just_started_editing = false; // Flag to indicate if we just entered edit mode

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
            Event::Input(event) => {
                if let Some(i) = editing_index {
                    // We are in edit mode
                    match event {
                        CEvent::Key(key_event) => {
                            match key_event.code {
                                KeyCode::Char(c) => {
                                    input_buffer.push(c);
                                }
                                KeyCode::Backspace => {
                                    input_buffer.pop();
                                }
                                KeyCode::Enter | KeyCode::Esc => {
                                    // Update the todo's name and exit edit mode
                                    todos[i].name = input_buffer.clone();
                                    input_buffer.clear();
                                    editing_index = None;
                                    // Save the todos after renaming
                                    save_todos(&todos);
                                }
                                _ => {
                                    // Any other key press exits edit mode and saves the name
                                    todos[i].name = input_buffer.clone();
                                    input_buffer.clear();
                                    editing_index = None;
                                    // Save the todos after renaming
                                    save_todos(&todos);

                                    // Now handle the key event as usual
                                    if key_event.code == KeyCode::Char('q') {
                                        break; // Exit the main loop
                                    }
                                    // Handle other key events if needed
                                }
                            }
                        }
                        CEvent::Mouse(mouse_event) => {
                            if just_started_editing {
                                // Ignore the mouse event that initiated edit mode
                                just_started_editing = false;
                            } else {
                                match mouse_event.kind {
                                    MouseEventKind::Moved => {
                                        // Do nothing, stay in edit mode
                                    }
                                    _ => {
                                        // For other mouse events, exit edit mode
                                        todos[i].name = input_buffer.clone();
                                        input_buffer.clear();
                                        editing_index = None;
                                        // Save the todos after renaming
                                        save_todos(&todos);

                                        // Now process the mouse event
                                        process_mouse_event(
                                            mouse_event,
                                            &mut todos,
                                            &mut dragging,
                                            &mut drag_index,
                                            &chunks,
                                            &mut editing_index,
                                            &mut input_buffer,
                                            &mut just_started_editing,
                                        );
                                    }
                                }
                            }
                        }
                        _ => {
                            // Any other event exits edit mode and saves the name
                            todos[i].name = input_buffer.clone();
                            input_buffer.clear();
                            editing_index = None;
                            // Save the todos after renaming
                            save_todos(&todos);
                        }
                    }
                } else {
                    // Not in edit mode
                    match event {
                        CEvent::Key(key_event) => {
                            if key_event.code == KeyCode::Char('q') {
                                break; // Exit the main loop
                            }
                            // Handle other key events if needed
                        }
                        CEvent::Mouse(mouse_event) => {
                            process_mouse_event(
                                mouse_event,
                                &mut todos,
                                &mut dragging,
                                &mut drag_index,
                                &chunks,
                                &mut editing_index,
                                &mut input_buffer,
                                &mut just_started_editing,
                            );
                        }
                        _ => {}
                    }
                }
            }
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
        let mut style = Style::default();
        let title: String;

        if editing_index == Some(i) {
            // Render input buffer with a cursor
            title = format!("{}_", input_buffer); // Add cursor
            style = Style::default().fg(Color::Yellow);
        } else {
            title = todo.name.clone();
        }

        let area = chunks[i];

        // Inside each chunk (line), create a horizontal layout
        let horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(
                [
                    Constraint::Length(30), // Fixed width for todo name
                    Constraint::Min(1),     // Remaining space for progress bar
                ]
                .as_ref(),
            )
            .split(area);

        // The title goes into the first chunk
        let title_paragraph = Paragraph::new(Span::styled(title, style));

        f.render_widget(title_paragraph, horizontal_chunks[0]);

        // Now build and render the progress bar in the second chunk

        let progress_bar_width = horizontal_chunks[1].width;

        let progress_bar = build_progress_bar(todo.progress, progress_bar_width as usize);

        let progress_bar_paragraph = Paragraph::new(Span::raw(progress_bar));

        f.render_widget(progress_bar_paragraph, horizontal_chunks[1]);
    }

    // Render the add button
    let add_button_text = Span::styled("[     +     ]", Style::default().fg(Color::Green));
    let add_button_paragraph = Paragraph::new(add_button_text).wrap(Wrap { trim: false });

    f.render_widget(add_button_paragraph, chunks[todos.len()]);
}

// Function to process mouse events
fn process_mouse_event(
    mouse_event: event::MouseEvent,
    todos: &mut Vec<Todo>,
    dragging: &mut bool,
    drag_index: &mut Option<usize>,
    chunks: &[Rect],
    editing_index: &mut Option<usize>,
    input_buffer: &mut String,
    just_started_editing: &mut bool,
) {
    match mouse_event.kind {
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

                        // Split the line into title and progress bar
                        let horizontal_chunks = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints(
                                [
                                    Constraint::Length(30), // Must match the ui function
                                    Constraint::Min(1),     // Remaining space for progress bar
                                ]
                                .as_ref(),
                            )
                            .split(*chunk);

                        if is_inside(mouse_pos, horizontal_chunks[0]) {
                            // Clicked on the title area - start editing
                            *editing_index = Some(i);
                            *just_started_editing = true; // Indicate that we just entered edit mode
                            if todos[i].name == "New Todo" {
                                *input_buffer = String::new(); // Start with an empty input buffer
                            } else {
                                *input_buffer = todos[i].name.clone(); // Start with the existing name
                            }
                        } else if is_inside(mouse_pos, horizontal_chunks[1]) {
                            // Clicked on the progress bar area
                            // Start dragging to update progress
                            *dragging = true;
                            *drag_index = Some(i);
                            update_progress(&mut todos[i], horizontal_chunks[1], mouse_event.column);
                            // Save the todos after updating progress
                            save_todos(&todos);
                        }

                        break; // We've found the clicked todo, so we can exit the loop
                    }
                }
                // Check if click is on the add button
                if !clicked_on_todo {
                    if let Some(add_button_rect) = chunks.get(todos.len()) {
                        if is_inside(mouse_pos, *add_button_rect) {
                            // Add a new todo
                            todos.push(Todo {
                                name: String::from("New Todo"),
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
            if *editing_index == None && *dragging && button == MouseButton::Left {
                if let Some(i) = *drag_index {
                    let chunk = chunks[i];
                    let horizontal_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints(
                            [
                                Constraint::Length(30), // Must match the ui function
                                Constraint::Min(1),
                            ]
                            .as_ref(),
                        )
                        .split(chunk);

                    update_progress(&mut todos[i], horizontal_chunks[1], mouse_event.column);
                    // Save the todos after updating progress
                    save_todos(&todos);
                }
            }
        }
        MouseEventKind::Up(button) => {
            if button == MouseButton::Left {
                *dragging = false;
                *drag_index = None;
            }
        }
        _ => {}
    }
}

// Function to update the progress of a todo based on mouse x position
fn update_progress(todo: &mut Todo, area: Rect, mouse_x: u16) {
    // Position of the '[' character
    let percent_str = format!(" {}%", todo.progress);
    let extra_chars = 2 + percent_str.len(); // '[' and ']' and percentage

    if area.width <= extra_chars as u16 {
        // Not enough space, do nothing
        return;
    }

    // Calculate the width of the progress bar
    let bar_width = area.width - extra_chars as u16;

    // The progress bar starts after the '[' character
    let progress_bar_start_x = area.x + 1; // Start after '['
    let progress_bar_end_x = progress_bar_start_x + bar_width;

    if mouse_x >= progress_bar_start_x && mouse_x <= progress_bar_end_x {
        let relative_x = mouse_x - progress_bar_start_x;
        let progress = ((relative_x * 100) / bar_width).min(100) as u16;
        if todo.progress != progress {
            todo.progress = progress;
        }
    }
}

// Function to build the ASCII progress bar
fn build_progress_bar(progress: u16, width: usize) -> String {
    // Width is the total width, we need to subtract for brackets and percentage
    let percent_str = format!(" {}%", progress);
    let extra_chars = 2 + percent_str.len(); // '[' and ']' and percentage

    if width <= extra_chars {
        // Not enough space to render progress bar
        return format!("{}%", percent_str);
    }

    let bar_width = width - extra_chars;

    let filled_blocks = (progress as usize * bar_width) / 100;
    let empty_blocks = bar_width - filled_blocks;
    format!(
        "[{}{}]{}",
        "#".repeat(filled_blocks),
        "-".repeat(empty_blocks),
        percent_str
    )
}

// Helper function to compute chunks based on the terminal size and todos
fn compute_chunks(size: Rect, todos: &[Todo]) -> Vec<Rect> {
    let mut constraints: Vec<Constraint> = Vec::new();

    for _ in todos {
        constraints.push(Constraint::Length(1)); // Each todo takes up 1 row
    }

    // Add constraint for the add button
    constraints.push(Constraint::Length(1));

    Layout::default()
        .direction(Direction::Vertical)
        .margin(1) // Reduce margin to save space
        .constraints(constraints)
        .split(size)
        .to_vec() // Convert Rc<[Rect]> to Vec<Rect>
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