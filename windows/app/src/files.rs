// Copyright (c) 2026 Claudio Bertini (devBertini)
// SPDX-License-Identifier: GPL-3.0-or-later
//! The app's folder, `%LOCALAPPDATA%\JuiceBar`: `settings.txt` and `history\<key>.txt`, one history file per
//! device (spec/estimator.md, "Storage and cost"). The core produces the lines; this module only moves them.
//!
//! A device key holds `:` (`logitech-hidpp:40B5`), which a Windows file name cannot, so the file name writes it
//! as `+`; the other characters Windows refuses become `_`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use juicebar_core::store::HEADER;
use juicebar_core::types::DeviceKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataDir {
    root: PathBuf,
}

/// The file name of a device's history.
pub fn file_name(key: &DeviceKey) -> String {
    let mut name: String = key
        .0
        .chars()
        .map(|c| match c {
            ':' => '+',
            '<' | '>' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    name.push_str(".txt");
    name
}

/// The key a history file name stands for; `None` for a file that is not a history.
pub fn key_of_file(name: &str) -> Option<DeviceKey> {
    let stem = name.strip_suffix(".txt")?;
    if stem.is_empty() {
        return None;
    }
    Some(DeviceKey(stem.replace('+', ":")))
}

impl DataDir {
    pub fn at(root: impl Into<PathBuf>) -> DataDir {
        DataDir { root: root.into() }
    }

    /// `%LOCALAPPDATA%\JuiceBar`, or `None` when the variable is missing (the app then keeps nothing).
    pub fn from_env() -> Option<DataDir> {
        let base = std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty())?;
        Some(DataDir::at(PathBuf::from(base).join("JuiceBar")))
    }

    #[cfg(test)]
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.txt")
    }

    /// Where the battery calibration keeps its lines, one `<model>.tsv` per model (`calib`).
    pub fn calibration_dir(&self) -> PathBuf {
        self.root.join("calibration")
    }

    fn history_dir(&self) -> PathBuf {
        self.root.join("history")
    }

    pub fn history_path(&self, key: &DeviceKey) -> PathBuf {
        self.history_dir().join(file_name(key))
    }

    pub fn read_settings(&self) -> String {
        fs::read_to_string(self.settings_path()).unwrap_or_default()
    }

    pub fn write_settings(&self, text: &str) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        write_replacing(&self.settings_path(), text)
    }

    /// Every history file: the device it belongs to and its text. A file that is not valid UTF-8 is skipped.
    pub fn read_histories(&self) -> Vec<(DeviceKey, String)> {
        let Ok(entries) = fs::read_dir(self.history_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<(DeviceKey, String)> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let key = key_of_file(e.file_name().to_str()?)?;
                let text = fs::read_to_string(e.path()).ok()?;
                Some((key, text))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Appends lines to a device's history with one write; a new file starts with the header in that same write.
    pub fn append(&self, key: &DeviceKey, lines: &[String]) -> io::Result<()> {
        if lines.is_empty() {
            return Ok(());
        }
        fs::create_dir_all(self.history_dir())?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.history_path(key))?;
        let mut chunk = String::new();
        if file.metadata()?.len() == 0 {
            chunk.push_str(HEADER);
            chunk.push('\n');
        }
        for l in lines {
            chunk.push_str(l);
            chunk.push('\n');
        }
        file.write_all(chunk.as_bytes())
    }

    /// Replaces a device's history with `text` (the compacted form): written beside it and renamed over it.
    pub fn replace(&self, key: &DeviceKey, text: &str) -> io::Result<()> {
        fs::create_dir_all(self.history_dir())?;
        write_replacing(&self.history_path(key), text)
    }

    /// Sets an unreadable history aside (`<name>.bad`) so a fresh one starts and nothing is lost.
    pub fn set_aside(&self, key: &DeviceKey) -> io::Result<()> {
        let path = self.history_path(key);
        fs::rename(&path, path.with_extension("bad"))
    }
}

fn write_replacing(path: &Path, text: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> DataDir {
        let root =
            std::env::temp_dir().join(format!("juicebar-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        DataDir::at(root)
    }

    fn key(s: &str) -> DeviceKey {
        DeviceKey(s.to_string())
    }

    #[test]
    fn the_calibration_folder_is_under_the_data_folder() {
        let d = DataDir::at(PathBuf::from("x").join("JuiceBar"));
        assert_eq!(
            d.calibration_dir(),
            PathBuf::from("x").join("JuiceBar").join("calibration")
        );
    }

    #[test]
    fn a_key_makes_a_windows_file_name_and_back() {
        let k = key("logitech-hidpp:40B5");
        assert_eq!(file_name(&k), "logitech-hidpp+40B5.txt");
        assert_eq!(key_of_file("logitech-hidpp+40B5.txt"), Some(k));
        assert_eq!(file_name(&key("a:b/c?d")), "a+b_c_d.txt");
        assert_eq!(key_of_file("settings.bad"), None);
        assert_eq!(key_of_file(".txt"), None);
    }

    #[test]
    fn a_first_append_writes_the_header_and_later_ones_do_not() {
        let dir = scratch("append");
        let k = key("logitech-hidpp:4086");
        dir.append(&k, &["P 10 1".into(), "R 11 50 d e -".into()])
            .unwrap();
        dir.append(&k, &["P 12 0".into()]).unwrap();
        dir.append(&k, &[]).unwrap();
        let text = fs::read_to_string(dir.history_path(&k)).unwrap();
        assert_eq!(text, "juicebar-history 1\nP 10 1\nR 11 50 d e -\nP 12 0\n");
        let _ = fs::remove_dir_all(dir.root());
    }

    #[test]
    fn replace_swaps_the_file_and_reading_lists_every_history() {
        let dir = scratch("replace");
        let (a, b) = (key("logitech-hidpp:0AB5"), key("logitech-hidpp:4086"));
        dir.append(&a, &["P 1 1".into()]).unwrap();
        dir.append(&b, &["P 2 1".into()]).unwrap();
        dir.replace(&a, "juicebar-history 1\nP 9 0\n").unwrap();
        let all = dir.read_histories();
        assert_eq!(
            all,
            vec![
                (a.clone(), "juicebar-history 1\nP 9 0\n".to_string()),
                (b, "juicebar-history 1\nP 2 1\n".to_string())
            ]
        );
        assert!(!dir.history_path(&a).with_extension("tmp").exists());
        dir.set_aside(&a).unwrap();
        assert_eq!(dir.read_histories().len(), 1);
        let _ = fs::remove_dir_all(dir.root());
    }

    #[test]
    fn settings_are_read_back_and_a_missing_folder_reads_empty() {
        let dir = scratch("settings");
        assert_eq!(dir.read_settings(), "");
        assert!(dir.read_histories().is_empty());
        dir.write_settings("rgb_economy=1\n").unwrap();
        assert_eq!(dir.read_settings(), "rgb_economy=1\n");
        let _ = fs::remove_dir_all(dir.root());
    }
}
