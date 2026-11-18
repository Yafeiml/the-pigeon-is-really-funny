use std::sync::atomic::{AtomicU8, Ordering};
use windows::Win32::Globalization::GetUserDefaultUILanguage;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Language {
    #[default]
    System,
    English,
    Chinese,
}

static PREFERENCE: AtomicU8 = AtomicU8::new(Language::System as u8);

impl Language {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::System),
            "en" => Some(Self::English),
            "zh-CN" => Some(Self::Chinese),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::System => "auto",
            Self::English => "en",
            Self::Chinese => "zh-CN",
        }
    }

    pub fn apply(self) {
        PREFERENCE.store(self as u8, Ordering::Relaxed);
    }
}

fn is_chinese_ui(lang_id: u16) -> bool {
    // A Windows LANGID stores the primary language in its low ten bits.
    lang_id & 0x03ff == 0x04
}

pub fn text<'a>(english: &'a str, chinese: &'a str) -> &'a str {
    let chinese_ui = match PREFERENCE.load(Ordering::Relaxed) {
        value if value == Language::English as u8 => false,
        value if value == Language::Chinese as u8 => true,
        _ => is_chinese_ui(unsafe { GetUserDefaultUILanguage() }),
    };
    if chinese_ui { chinese } else { english }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chinese_display_language_variants_share_the_translation() {
        for chinese in [0x0404, 0x0804, 0x0c04, 0x1004, 0x1404] {
            assert!(is_chinese_ui(chinese));
        }
        for other in [0x0409, 0x0809, 0x040c, 0x0411, 0] {
            assert!(!is_chinese_ui(other));
        }
    }
}
