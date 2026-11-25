//! Native file and folder pickers, and the helper that copies a folder in.
//!
//! gpui has no file dialogs, and the window must stay responsive while one is
//! open, so a picker runs on its own thread: `System.Windows.Forms.OpenFileDialog`
//! or `FolderBrowserDialog` through PowerShell in STA mode. The chosen path(s)
//! come back on stdout; an empty answer means the user cancelled.

use std::process::{Command, Stdio};

/// Hide the console window a child process would otherwise flash.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The script the file picker runs. As above, kept ASCII.
const FILE_SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms | Out-Null; \
$dialog = New-Object System.Windows.Forms.OpenFileDialog; \
$dialog.Multiselect = $true; \
$dialog.Filter = '{filter}'; \
$dialog.Title = 'Add files'; \
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { \
[Console]::Out.Write(($dialog.FileNames -join ([char]10))) }";

/// The script the folder picker runs. As above, kept ASCII.
const FOLDER_SCRIPT: &str = "Add-Type -AssemblyName System.Windows.Forms | Out-Null; \
$dialog = New-Object System.Windows.Forms.FolderBrowserDialog; \
$dialog.Description = '{title}'; \
$dialog.ShowNewFolderButton = $false; \
if ($dialog.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { \
[Console]::Out.Write($dialog.SelectedPath) }";

/// Ask the user for files matching one dialog filter. Blocking: call it from a
/// worker thread.
///
/// `filter` uses the Windows dialog syntax (`Text|*.txt;*.md|All files|*.*`)
/// and must stay ASCII, like the folder script. Returns the chosen files, or
/// `None` when the dialog was cancelled or could not be shown.
pub fn pick_files(filter: &str) -> Option<Vec<std::path::PathBuf>> {
    let stdout = run_dialog(&FILE_SCRIPT.replace("{filter}", filter))?;
    answer_files(&stdout)
}

/// Ask the user for one folder. Blocking: call it from a worker thread.
///
/// `title` is the line above the folder tree and must stay ASCII, like the
/// filter: the whole script travels as one command-line argument. Returns the
/// chosen folder, or `None` when the dialog was cancelled or could not be shown.
pub fn pick_folder(title: &str) -> Option<std::path::PathBuf> {
    let stdout = run_dialog(&FOLDER_SCRIPT.replace("{title}", title))?;
    answer_folder(&stdout)
}

/// Show one dialog script and return its stdout.
fn run_dialog(script: &str) -> Option<String> {
    crate::foreground::allow_child_window_foreground();
    let mut command = Command::new("powershell");
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-STA")
        .arg("-Command")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Read the chosen folder out of the picker's stdout.
fn answer_folder(stdout: &str) -> Option<std::path::PathBuf> {
    let path = stdout.trim();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

/// Read the chosen files out of the picker's stdout, one path per line.
fn answer_files(stdout: &str) -> Option<Vec<std::path::PathBuf>> {
    let files: Vec<std::path::PathBuf> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(std::path::PathBuf::from)
        .collect();
    (!files.is_empty()).then_some(files)
}

/// Copy a folder and everything under it into `target`.
///
/// A skill or plugin is copied rather than referenced: the program keeps its
/// own copy, so the folder it was imported from can be moved or deleted. Links
/// are skipped instead of followed, so a copied tree never points back out.
pub fn copy_dir(
    source: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(target)
        .map_err(|error| format!("无法创建 {}：{error}", target.display()))?;
    let entries = std::fs::read_dir(source)
        .map_err(|error| format!("无法读取 {}：{error}", source.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) => return Err(format!("无法读取 {}：{error}", path.display())),
        };
        if kind.is_symlink() {
            continue;
        }
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&path, &destination)?;
        } else if kind.is_file() {
            std::fs::copy(&path, &destination)
                .map_err(|error| format!("无法复制 {}：{error}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_path_per_chosen_file() {
        let files = answer_files("C:\\a\\one.txt\r\nC:\\b\\two.txt\r\n").expect("two files");
        assert_eq!(
            files,
            vec![
                std::path::PathBuf::from("C:\\a\\one.txt"),
                std::path::PathBuf::from("C:\\b\\two.txt")
            ]
        );
        // Nothing chosen, and blank lines from the console, mean cancelled.
        assert!(answer_files("").is_none());
        assert!(answer_files("  \r\n \r\n").is_none());
    }

    #[test]
    fn reads_the_one_chosen_folder() {
        assert_eq!(
            answer_folder("C:\\a\\b\r\n"),
            Some(std::path::PathBuf::from("C:\\a\\b"))
        );
        assert!(answer_folder("").is_none());
        assert!(answer_folder("  \r\n").is_none());
    }

    #[test]
    fn copies_a_folder_with_its_files_and_subfolders() {
        let root = std::env::temp_dir().join(format!("dsh-flow-ui-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("source");
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::write(source.join("skill.json"), "{}").unwrap();
        std::fs::write(source.join("assets").join("logo.txt"), "x").unwrap();

        let target = root.join("target");
        copy_dir(&source, &target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("skill.json")).unwrap(),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("assets").join("logo.txt")).unwrap(),
            "x"
        );
        // The original stays where it was.
        assert!(source.join("skill.json").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
