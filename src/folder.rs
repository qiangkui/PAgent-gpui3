//! Native file picker and "open folder" helper.
//!
//! gpui has no file dialogs, and the window must stay responsive while one is
//! open, so the picker runs on its own thread: `System.Windows.Forms.
//! OpenFileDialog` through PowerShell in STA mode. The chosen paths come back
//! on stdout; an empty answer means the user cancelled.

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

/// Ask the user for files matching one dialog filter. Blocking: call it from a
/// worker thread.
///
/// `filter` uses the Windows dialog syntax (`Text|*.txt;*.md|All files|*.*`)
/// and must stay ASCII, like the folder script. Returns the chosen files, or
/// `None` when the dialog was cancelled or could not be shown.
pub fn pick_files(filter: &str) -> Option<Vec<std::path::PathBuf>> {
    crate::foreground::allow_child_window_foreground();
    let script = FILE_SCRIPT.replace("{filter}", filter);
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
    answer_files(&String::from_utf8_lossy(&output.stdout))
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
}
