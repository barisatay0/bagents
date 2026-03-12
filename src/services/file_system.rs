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
