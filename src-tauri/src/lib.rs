mod app;
mod commands;
mod domain;
mod infrastructure;
mod services;
mod state;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = app::run() {
        panic!("failed to start wabity: {error:#}");
    }
}
