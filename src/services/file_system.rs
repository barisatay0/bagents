use crate::models::file_modification::FileModification;
use std::fs;
use std::path::Path;

pub fn apply_modifications(modifications: Vec<FileModification>) -> Result<(), String> {
    for modif in modifications {
        let path = Path::new(&modif.file_path);

        // Create directories if they don't exist
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        // Write the new content to the file
        fs::write(path, modif.new_content).map_err(|e| e.to_string())?;
        println!("File updated/created: {}", modif.file_path);
    }
    Ok(())
}
