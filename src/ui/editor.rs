//todo: ask if you trust the project to not jsut eval some fuckass js

use std::path::PathBuf;

use eframe::egui;

use crate::scripting::{V8Context, init_v8_context};

pub struct CatroEditorApp {
    project_path: PathBuf,
    project_name: String,
    v8_context: V8Context,
}

impl CatroEditorApp {
    pub fn new(project_path: PathBuf) -> Self {
        //todo: better handle error!
        let mut context = match init_v8_context() {
            Ok(context) => context,
            Err(error) => panic!("Failed to init V8 scripting context: {}", error),
        };

        //todo: handle error
        context
            .run_file(project_path.join("init.js"))
            .expect("eval deez");

        let project_name = context
            .get::<String>("config.name")
            .unwrap_or_else(|_| String::from("untitled"));

        Self {
            project_path,
            project_name,
            v8_context: context,
        }
    }
}

impl eframe::App for CatroEditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(&self.project_name);
            ui.label(self.project_path.display().to_string());
            ui.separator();
        });
    }
}
