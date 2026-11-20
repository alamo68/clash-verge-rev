use std::{
    fs::{self, File},
    io::{Read as _, Seek as _, SeekFrom},
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
struct GameplayVisibility {
    is_in_match: bool,
}

impl GameplayVisibility {
    fn consume(&mut self, line: &str) {
        if line.contains("LoadingScreen.OnScenePreUnload")
            && let Some(mode) = Self::value(line, "nextMode")
            && mode != "GAMEPLAY"
        {
            self.is_in_match = false;
        } else if line.contains("LoadingScreen.OnSceneLoaded")
            && let Some(mode) = Self::value(line, "currMode")
        {
            self.is_in_match = mode == "GAMEPLAY";
        } else if line.contains("Gameplay.Start()") {
            self.is_in_match = false;
        } else if line.contains("MulliganManager.HandleGameStart()") {
            self.is_in_match = true;
        }
    }

    fn value(line: &str, key: &str) -> Option<String> {
        let (_, value) = line.split_once(&format!("{key}="))?;
        Some(
            value
                .chars()
                .take_while(|c| c.is_alphabetic() || *c == '_')
                .collect::<String>()
                .to_uppercase(),
        )
    }
}

pub(super) struct GameSceneMonitor {
    root: PathBuf,
    visibility: GameplayVisibility,
    path: Option<PathBuf>,
    offset: u64,
    pending: Vec<u8>,
    next_scan: Instant,
}

impl GameSceneMonitor {
    pub(super) fn new() -> Self {
        Self {
            root: PathBuf::from("/Applications/Hearthstone/Logs"),
            visibility: GameplayVisibility::default(),
            path: None,
            offset: 0,
            pending: Vec::new(),
            next_scan: Instant::now(),
        }
    }

    pub(super) fn is_in_match(&mut self) -> bool {
        if Instant::now() >= self.next_scan {
            self.next_scan = Instant::now() + Duration::from_secs(2);
            let newest = fs::read_dir(&self.root)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("Hearthstone_"))
                .max_by_key(|entry| entry.file_name())
                .map(|entry| entry.path().join("LoadingScreen.log"));
            if newest != self.path {
                self.path = newest;
                self.reset();
            }
        }
        if self.refresh().is_err() {
            self.reset();
        }
        self.visibility.is_in_match
    }

    fn reset(&mut self) {
        self.offset = 0;
        self.pending.clear();
        self.visibility = GameplayVisibility::default();
    }

    fn refresh(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut file = File::open(path)?;
        if file.metadata()?.len() < self.offset {
            self.reset();
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        self.offset += data.len() as u64;
        self.pending.extend(data);
        let mut consumed = 0;
        for (index, byte) in self.pending.iter().enumerate() {
            if *byte == b'\n' {
                self.visibility
                    .consume(&String::from_utf8_lossy(&self.pending[consumed..index]));
                consumed = index + 1;
            }
        }
        self.pending.drain(..consumed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_scene_and_reconnect_transitions() {
        let mut visibility = GameplayVisibility::default();
        for (line, expected) in [
            ("LoadingScreen.OnSceneLoaded currMode=GAMEPLAY", true),
            ("LoadingScreen.OnScenePreUnload nextMode=GAMEPLAY", true),
            ("Gameplay.Start()", false),
            ("MulliganManager.HandleGameStart()", true),
            ("LoadingScreen.OnScenePreUnload nextMode=HUB", false),
            ("LoadingScreen.OnSceneLoaded currMode=gameplay", true),
            ("LoadingScreen.OnSceneLoaded currMode=COLLECTIONMANAGER", false),
        ] {
            visibility.consume(line);
            assert_eq!(visibility.is_in_match, expected, "{line}");
        }
    }

    #[test]
    fn tails_complete_lines_and_resets_on_truncation_and_session_change() {
        let root = std::env::temp_dir().join(format!("hearthstone-scene-{}", std::process::id()));
        let session = root.join("Hearthstone_2026_11_19");
        fs::create_dir_all(&session).unwrap();
        let log = session.join("LoadingScreen.log");
        fs::write(&log, "LoadingScreen.OnSceneLoaded currMode=GAME").unwrap();
        let mut monitor = GameSceneMonitor::new();
        monitor.root = root.clone();
        assert!(!monitor.is_in_match());
        use std::io::Write as _;
        File::options()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"PLAY\n")
            .unwrap();
        assert!(monitor.is_in_match());
        fs::write(&log, "Gameplay.Start()\n").unwrap();
        assert!(!monitor.is_in_match());
        File::options()
            .append(true)
            .open(&log)
            .unwrap()
            .write_all(b"MulliganManager.HandleGameStart()\n")
            .unwrap();
        assert!(monitor.is_in_match());
        let next = root.join("Hearthstone_2026_11_20");
        fs::create_dir_all(&next).unwrap();
        monitor.next_scan = Instant::now();
        assert!(!monitor.is_in_match());
        fs::write(
            next.join("LoadingScreen.log"),
            "LoadingScreen.OnSceneLoaded currMode=GAMEPLAY\n",
        )
        .unwrap();
        assert!(monitor.is_in_match());
        fs::remove_dir_all(root).unwrap();
    }
}
