use crate::models::file_modification::FileModification;
use std::env;
use std::fs;
use std::path::Path;

pub fn apply_modifications(modifications: Vec<FileModification>) -> Result<(), String> {
    let workspace_str = env::var("WORKSPACE_DIR").expect("WORKSPACE_DIR missing in .env");
    let workspace_path = Path::new(&workspace_str);

    for modif in modifications {
        let full_path = workspace_path.join(&modif.file_path);

        // Create directories if they don't exist
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        // Write the new content to the file in the target workspace
        fs::write(&full_path, modif.new_content).map_err(|e| e.to_string())?;
        println!("File updated/created in target repo: {:?}", full_path);
    }
    Ok(())
}

pub fn get_repo_tree() -> String {
    let workspace_str = std::env::var("WORKSPACE_DIR").expect("WORKSPACE_DIR missing in .env");
    let workspace_path = std::path::Path::new(&workspace_str);

    let mut context = String::from("REPOSITORY STRUCTURE (DIRECTORY TREE):\n");
    let mut file_paths = Vec::new();

    fn collect_paths(dir: &std::path::Path, workspace: &std::path::Path, paths: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let file_name = entry.file_name().to_string_lossy().to_string();

                if file_name.starts_with('.')
                    || file_name == "target"
                    || file_name == "node_modules"
                    || file_name == "dist"
                {
                    continue;
                }

                if path.is_dir() {
                    collect_paths(&path, workspace, paths);
                } else {
                    let rel_path = path
                        .strip_prefix(workspace)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    paths.push(rel_path);
                }
            }
        }
    }

    collect_paths(workspace_path, workspace_path, &mut file_paths);
    file_paths.sort();

    for path in &file_paths {
        context.push_str(&format!("  - {}\n", path));
    }

    context
}

pub fn read_specific_files(files: Vec<String>) -> String {
    let workspace_str = std::env::var("WORKSPACE_DIR").expect("WORKSPACE_DIR missing in .env");
    let workspace_path = std::path::Path::new(&workspace_str);

    let mut contents = String::from("REQUESTED FILE CONTENTS:\n");

    for file_path in files {
        let full_path = workspace_path.join(&file_path);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            if content.len() > 15000 {
                contents.push_str(&format!(
                    "\n--- FILE: {} (Content too large, skipped) ---\n",
                    file_path
                ));
            } else {
                contents.push_str(&format!("\n--- FILE: {} ---\n{}\n", file_path, content));
            }
        } else {
            contents.push_str(&format!(
                "\n--- FILE: {} (Could not read file or file does not exist) ---\n",
                file_path
            ));
        }
    }

    contents
}
