use super::{SIZE_MAX, SIZE_MIN};
use crate::{i18n::Language, motion::Mode};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Settings {
    pub language: Language,
    pub size_percent: u32,
    pub mode: Mode,
    pub speed: f32,
    pub fps: Option<u32>,
    pub paused: bool,
    pub click_through: bool,
    pub fullscreen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::System,
            size_percent: SIZE_MAX,
            mode: Mode::Auto,
            speed: 1.0,
            fps: None,
            paused: false,
            click_through: false,
            fullscreen: true,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        let base = PathBuf::from(env::var_os("LOCALAPPDATA")?);
        base.is_absolute()
            .then(|| base.join("DesktopPigeon").join("settings.ini"))
    }

    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(contents) = fs::read(path) else {
            return settings;
        };
        for line in String::from_utf8_lossy(&contents)
            .trim_start_matches('\u{feff}')
            .lines()
        {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match (key.trim(), value) {
                ("language", _) => {
                    if let Some(language) = Language::parse(value) {
                        settings.language = language;
                    }
                }
                ("size_percent", _) => {
                    if let Ok(percent @ SIZE_MIN..=SIZE_MAX) = value.parse() {
                        settings.size_percent = percent;
                    }
                }
                ("mode", "auto") => settings.mode = Mode::Auto,
                ("mode", "rest") => settings.mode = Mode::Rest,
                ("speed", _) => {
                    if let Ok(speed @ (1.0 | 1.5 | 2.0)) = value.parse() {
                        settings.speed = speed;
                    }
                }
                ("fps", "auto") => settings.fps = None,
                ("fps", _) => {
                    if let Ok(fps @ (60 | 120 | 144)) = value.parse() {
                        settings.fps = Some(fps);
                    }
                }
                ("paused", _) => settings.paused = value.parse().unwrap_or(settings.paused),
                ("click_through", _) => {
                    settings.click_through = value.parse().unwrap_or(settings.click_through)
                }
                ("fullscreen", _) => {
                    settings.fullscreen = value.parse().unwrap_or(settings.fullscreen)
                }
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let mode = if self.mode == Mode::Rest {
            "rest"
        } else {
            "auto"
        };
        let fps = self
            .fps
            .map_or_else(|| "auto".into(), |fps| fps.to_string());
        let contents = format!(
            "size_percent={}\nmode={mode}\nspeed={}\nfps={fps}\npaused={}\nclick_through={}\nfullscreen={}\nlanguage={}\n",
            self.size_percent,
            self.speed,
            self.paused,
            self.click_through,
            self.fullscreen,
            self.language.code(),
        );
        let temporary = path.with_extension("tmp");
        fs::write(&temporary, contents)?;
        fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_all_choices_and_replaces_previous_settings() {
        let directory =
            env::temp_dir().join(format!("pigeon-prefs-roundtrip-{}", std::process::id()));
        let path = directory.join("settings.ini");
        assert_eq!(Settings::load(&path), Settings::default());
        let settings = Settings {
            language: Language::English,
            size_percent: 73,
            mode: Mode::Rest,
            speed: 1.5,
            fps: Some(144),
            paused: true,
            click_through: true,
            fullscreen: false,
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        Settings::default().save(&path).unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn malformed_values_only_affect_their_own_setting() {
        let path = env::temp_dir().join(format!("pigeon-prefs-invalid-{}.ini", std::process::id()));
        fs::write(&path, b"\xef\xbb\xbf size_percent=49\nmode=walk\nspeed=NaN\nfps=0\nclick_through=5\nfullscreen=\xff\npaused = true\r\nlanguage=unknown\nunknown=1\n").unwrap();
        assert_eq!(
            Settings::load(&path),
            Settings {
                paused: true,
                ..Settings::default()
            }
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn existing_settings_without_language_follow_windows() {
        let path = env::temp_dir().join(format!("pigeon-prefs-legacy-{}.ini", std::process::id()));
        fs::write(&path, "mode=rest\nsize_percent=67\n").unwrap();
        let old = Settings::load(&path);
        assert_eq!(old.language, Language::System);
        assert_eq!(old.mode, Mode::Rest);
        assert_eq!(old.size_percent, 67);
        fs::remove_file(path).unwrap();
    }
}
