#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod helpers;
mod project;
mod scripting;
mod ui;

use std::{env, path::PathBuf};

use crate::ui::{editor::CatroEditorApp, welcome::CatroApp};

fn main() -> eframe::Result {
    // let (mut rl, thread) = raylib::init().size(640, 480).title("Hello, World").build();

    // Path to a project to open as optional argument.
    let project_path = env::args().nth(1).map(PathBuf::from);

    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Catro",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);

            match project_path {
                Some(path) => Ok(Box::new(CatroEditorApp::new(path)) as Box<dyn eframe::App>),
                None => Ok(Box::<CatroApp>::default()),
            }
        }),
    )
    // while !rl.window_should_close() {
    //     let mut d = rl.begin_drawing(&thread);

    //     d.clear_background(Color::WHITE);
    //     d.draw_text("Hello, world!", 12, 12, 20, Color::BLACK);
    // }
}
