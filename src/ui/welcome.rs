use std::path::PathBuf;

use eframe::egui;

use crate::{
    project::{self, ProjectGameConfig},
    scripting::{V8Context, init_v8_context},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Name,
    Size,
    Resizable,
}

pub struct CatroApp {
    #[allow(dead_code)]
    v8_context: V8Context,
    step: Step,
    project_name: String,
    project_directory: String,
    width: u32,
    height: u32,
    resizable: bool,
}

impl Default for CatroApp {
    fn default() -> Self {
        //todo: better handle error!
        let context = match init_v8_context() {
            Ok(context) => context,
            Err(error) => panic!("Failed to init V8 scripting context: {}", error),
        };

        Self {
            v8_context: context,
            step: Step::Name,
            project_directory: String::new(),
            project_name: String::new(),
            width: 800,
            height: 600,
            resizable: true,
        }
    }
}

/// Called once the user finishes the new project screens.
pub fn create_project(directory: &str, name: &str, size: (u32, u32), resizable: bool) {
    project::create_project(
        PathBuf::from(directory),
        ProjectGameConfig {
            name: name.to_string(),
            window_size: size,
            resizable,
        },
    );
}

impl eframe::App for CatroApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("New project"); //todo: maybe show temporal-based greeting?
            ui.separator();

            match self.step {
                Step::Name => {
                    ui.horizontal(|ui| {
                        let label = ui.label("Project directory: ");
                        ui.text_edit_singleline(&mut self.project_directory)
                            .labelled_by(label.id);
                    });
                    ui.horizontal(|ui| {
                        let label = ui.label("Project name: ");
                        ui.text_edit_singleline(&mut self.project_name)
                            .labelled_by(label.id);
                    });
                }
                Step::Size => {
                    ui.horizontal(|ui| {
                        ui.label("Width: ");
                        ui.add(egui::DragValue::new(&mut self.width).range(1..=16384));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Height: ");
                        ui.add(egui::DragValue::new(&mut self.height).range(1..=16384));
                    });
                }
                Step::Resizable => {
                    ui.checkbox(&mut self.resizable, "Resizable window");
                }
            }

            ui.separator();
            ui.horizontal(|ui| {
                let can_go_back = self.step != Step::Name;
                if ui
                    .add_enabled(can_go_back, egui::Button::new("Back"))
                    .clicked()
                {
                    self.step = match self.step {
                        Step::Resizable => Step::Size,
                        _ => Step::Name,
                    };
                }

                match self.step {
                    Step::Name => {
                        let valid = !self.project_name.trim().is_empty();
                        if ui.add_enabled(valid, egui::Button::new("Next")).clicked() {
                            self.step = Step::Size;
                        }
                    }
                    Step::Size => {
                        if ui.button("Next").clicked() {
                            self.step = Step::Resizable;
                        }
                    }
                    Step::Resizable => {
                        if ui.button("Create").clicked() {
                            create_project(
                                self.project_directory.trim(),
                                self.project_name.trim(),
                                (self.width, self.height),
                                self.resizable,
                            );
                        }
                    }
                }
            });
        });
    }
}
