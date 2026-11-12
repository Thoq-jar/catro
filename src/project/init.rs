use std::{fs, path::PathBuf};

const INIT_JS: &str = include_str!("init.js");
const HELLO_JS: &str = include_str!("hello.js");

pub struct ProjectGameConfig {
    pub name: String,
    pub window_size: (u32, u32),
    pub resizable: bool,
}

pub fn create_project(directory: PathBuf, config: ProjectGameConfig) {
    let name_literal = match serde_json::to_string(&config.name) {
        Ok(literal) => literal,
        Err(error) => {
            eprintln!("Failed to encode project name: {}", error);
            return;
        }
    };

    let name = &config.name;
    let init = INIT_JS;
    let (width, height) = config.window_size;
    let resizable = config.resizable;
    //todo: generate nicer
    let config = format!(
        "globalThis.config = {{\n    name: {name_literal},\n    windowSize: {{ width: {width}, height: {height} }},\n    resizable: {resizable},\n}};\n"
    );

    let project_dir = directory.join(name);

    if !project_dir.exists() {
        if let Err(error) = fs::create_dir_all(&project_dir) {
            eprintln!(
                "Failed to create project directory {}: {}",
                project_dir.display(),
                error
            );
            return;
        }
    }

    let init_path = project_dir.join("init.js");
    let config_path = project_dir.join("config.js");

    if let Err(error) = fs::write(&init_path, init) {
        eprintln!("Failed to write {}: {}", init_path.display(), error);
        return;
    }

    if let Err(error) = fs::write(&config_path, config) {
        eprintln!("Failed to write {}: {}", config_path.display(), error);
        return;
    }

    let scripts_dir = project_dir.join("scripts");
    let assets_dir = project_dir.join("assets");

    if let Err(error) = fs::create_dir_all(&scripts_dir) {
        eprintln!("Failed to create {}: {}", scripts_dir.display(), error);
        return;
    }

    if let Err(error) = fs::create_dir_all(&assets_dir) {
        eprintln!("Failed to create {}: {}", assets_dir.display(), error);
        return;
    }

    let hello_path = scripts_dir.join("hello.js");
    if let Err(error) = fs::write(&hello_path, HELLO_JS) {
        eprintln!("Failed to write {}: {}", hello_path.display(), error);
        return;
    }

    println!("Project created sucessfully!");
}
