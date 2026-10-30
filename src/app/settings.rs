/// One row of the settings menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Setting {
    Player,
    Chords,
    ChordsUg,
    AutoZeroCapo,
    ChordsMatch,
    MemberParts,
    MemberPosition,
    Translate,
    TranslateMode,
    TranslationPosition,
    TranslateEngine,
    AiProvider,
    AiModel,
    AiEndpoint,
    AiKeyEnv,
    TranslateLang,
    TranslateSkip,
    TranslateFallback,
    CacheTranslations,
    Minimal,
    Customize,
    GenreLookup,
    Annotations,
    AnnotationsMode,
    AnnotationMaxChars,
    AnnotationScroll,
    GeniusToken,
    BigLyrics,
    Karaoke,
    WindowTitle,
    BigScale,
    LyricSpacing,
    LyricPad,
    Cava,
    ThirdPanel,
    CoverTheme,
    AsciiCover,
    Transparent,
    HideColorWarning,
    Mouse,
    VolumeStep,
    SpotifyOn,
    SpotifyClientId,
    SpotifyLogin,
    SpotifyLogout,
    SpotifyDevice,
    UseSpotifyd,
    KillSpotifyd,
    SpotifyButton,
    SpotifyQueueTab,
}

/// What the settings menu draws, top to bottom: group headers, the Spotify status line, and
/// the rows themselves (`Item` is an index into [`Setting::ALL`]; only those are selectable).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsRow {
    Header(&'static str),
    Status,
    Item(usize),
}

impl Setting {
    /// The group this row starts, if it is the first of one.
    pub fn header(self) -> Option<&'static str> {
        match self {
            Setting::Player => Some("General"),
            Setting::SpotifyOn => Some("Spotify"),
            _ => None,
        }
    }

    /// The menu laid out for drawing (headers and the status line between the rows).
    pub fn rows() -> Vec<SettingsRow> {
        let mut out = Vec::new();
        for (i, s) in Setting::ALL.iter().enumerate() {
            if let Some(h) = s.header() {
                out.push(SettingsRow::Header(h));
                if *s == Setting::SpotifyOn {
                    out.push(SettingsRow::Status);
                }
            }
            out.push(SettingsRow::Item(i));
        }
        out
    }

    /// Edited as text (Enter starts typing).
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Setting::SpotifyClientId | Setting::SpotifyDevice | Setting::GeniusToken | Setting::AiModel | Setting::AiEndpoint | Setting::AiKeyEnv
        )
    }

    pub const ALL: &'static [Setting] = &[
        Setting::Player,
        Setting::Chords,
        Setting::ChordsUg,
        Setting::AutoZeroCapo,
        Setting::ChordsMatch,
        Setting::MemberParts,
        Setting::MemberPosition,
        Setting::Translate,
        Setting::TranslateMode,
        Setting::TranslationPosition,
        Setting::TranslateEngine,
        Setting::AiProvider,
        Setting::AiModel,
        Setting::AiEndpoint,
        Setting::AiKeyEnv,
        Setting::TranslateLang,
        Setting::TranslateSkip,
        Setting::TranslateFallback,
        Setting::CacheTranslations,
        Setting::Minimal,
        Setting::Customize,
        Setting::GenreLookup,
        Setting::Annotations,
        Setting::AnnotationsMode,
        Setting::AnnotationMaxChars,
        Setting::AnnotationScroll,
        Setting::GeniusToken,
        Setting::BigLyrics,
        Setting::Karaoke,
        Setting::WindowTitle,
        Setting::BigScale,
        Setting::LyricSpacing,
        Setting::LyricPad,
        Setting::Cava,
        Setting::ThirdPanel,
        Setting::CoverTheme,
        Setting::AsciiCover,
        Setting::Transparent,
        Setting::HideColorWarning,
        Setting::Mouse,
        Setting::VolumeStep,
        Setting::SpotifyOn,
        Setting::SpotifyClientId,
        Setting::SpotifyLogin,
        Setting::SpotifyLogout,
        Setting::SpotifyDevice,
        Setting::UseSpotifyd,
        Setting::KillSpotifyd,
        Setting::SpotifyButton,
        Setting::SpotifyQueueTab,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Setting::Player => "Player source",
            Setting::Chords => "Chords",
            Setting::ChordsUg => "Ultimate Guitar (next track)",
            Setting::AutoZeroCapo => "Capo 0 for every sheet",
            Setting::ChordsMatch => "Chord match threshold (%)",
            Setting::MemberParts => "Member parts (experimental)",
            Setting::MemberPosition => "Member parts shown",
            Setting::Translate => "Translate lyrics",
            Setting::TranslateMode => "Translation shown as",
            Setting::TranslationPosition => "Translation placed",
            Setting::TranslateEngine => "Translation engine",
            Setting::AiProvider => "AI provider",
            Setting::AiModel => "AI model",
            Setting::AiEndpoint => "AI endpoint",
            Setting::AiKeyEnv => "AI key variable",
            Setting::TranslateLang => "Translate into",
            Setting::TranslateSkip => "Never translate",
            Setting::TranslateFallback => "LibreTranslate fallback",
            Setting::CacheTranslations => "Keep translations on disk",
            Setting::Minimal => "Minimal view",
            Setting::Customize => "Customize layout",
            Setting::GenreLookup => "Genre lookup (MusicBrainz)",
            Setting::Annotations => "Annotations panel (experimental)",
            Setting::AnnotationsMode => "Annotations from",
            Setting::AnnotationMaxChars => "Annotation length",
            Setting::AnnotationScroll => "Long annotations",
            Setting::GeniusToken => "Genius token",
            Setting::BigLyrics => "Big active line",
            Setting::Karaoke => "Word-by-word highlight",
            Setting::WindowTitle => "Terminal window title",
            Setting::BigScale => "Big line size",
            Setting::LyricSpacing => "Lyric spacing",
            Setting::LyricPad => "Lyric side padding",
            Setting::Cava => "Cava visualiser",
            Setting::ThirdPanel => "Cava as third column",
            Setting::CoverTheme => "Accent from cover",
            Setting::AsciiCover => "ASCII cover",
            Setting::Transparent => "Transparent background",
            Setting::HideColorWarning => "Hide colour-file note",
            Setting::Mouse => "Mouse support",
            Setting::VolumeStep => "Volume step %",
            Setting::SpotifyOn => "Spotify features",
            Setting::SpotifyClientId => "Client ID",
            Setting::SpotifyLogin => "Log in",
            Setting::SpotifyLogout => "Log out",
            Setting::SpotifyDevice => "spotifyd device name",
            Setting::UseSpotifyd => "Use spotifyd",
            Setting::KillSpotifyd => "Stop spotifyd on exit",
            Setting::SpotifyButton => "Library button (top left)",
            Setting::SpotifyQueueTab => "Queue tab in the library",
        }
    }

    /// The config-file keys this row owns.
    pub fn config_keys(self) -> &'static [&'static str] {
        match self {
            Setting::Player => &["player"],
            Setting::Chords => &["chords"],
            Setting::ChordsUg => &["chords_ug"],
            Setting::AutoZeroCapo => &["auto_zero_capo"],
            Setting::ChordsMatch => &["chords_match"],
            Setting::MemberParts => &["member_parts"],
            Setting::MemberPosition => &["member_position"],
            Setting::TranslationPosition => &["translation_position"],
            Setting::Translate | Setting::TranslateMode => &["translate", "translate_mode"],
            Setting::TranslateEngine => &["translate_engine"],
            Setting::AiProvider => &["ai.provider"],
            Setting::AiModel => &["ai.model"],
            Setting::AiEndpoint => &["ai.endpoint"],
            Setting::AiKeyEnv => &["ai.api_key_env"],
            Setting::TranslateLang => &["translate_lang"],
            Setting::TranslateSkip => &["translate_skip"],
            Setting::TranslateFallback => &["translate_fallback"],
            Setting::CacheTranslations => &["cache_translations"],
            Setting::Minimal => &["minimal_view"],
            Setting::Customize => &[],
            Setting::GenreLookup => &["genre_lookup"],
            Setting::Annotations => &["annotations"],
            Setting::AnnotationsMode => &["annotations_mode"],
            Setting::AnnotationMaxChars => &["annotation_max_chars"],
            Setting::AnnotationScroll => &["annotation_scroll"],
            Setting::GeniusToken => &["genius_token"],
            Setting::BigLyrics => &["big_lyrics"],
            Setting::Karaoke => &["karaoke"],
            Setting::WindowTitle => &["window_title"],
            Setting::BigScale => &["big_scale"],
            Setting::LyricSpacing => &["lyric_spacing"],
            Setting::LyricPad => &["lyric_pad"],
            Setting::Cava => &["cava", "thirdpanel"],
            Setting::ThirdPanel => &["thirdpanel", "cava"],
            Setting::CoverTheme => &["cover_theme"],
            Setting::AsciiCover => &["ascii_cover"],
            Setting::Transparent => &["transparent"],
            Setting::HideColorWarning => &["hide_color_warning"],
            Setting::Mouse => &["mouse"],
            Setting::VolumeStep => &["volume_step"],
            Setting::SpotifyOn => &["spotify"],
            Setting::SpotifyClientId => &["spotify_client_id"],
            Setting::SpotifyLogin | Setting::SpotifyLogout => &[],
            Setting::SpotifyDevice => &["spotify_device"],
            Setting::UseSpotifyd => &["use_spotifyd"],
            Setting::KillSpotifyd => &["kill_spotifyd_on_exit"],
            Setting::SpotifyButton => &["spotify_button"],
            Setting::SpotifyQueueTab => &["spotify_queue_tab"],
        }
    }

    /// Takes effect only after a restart.
    pub fn needs_restart(self) -> bool {
        matches!(
            self,
            Setting::CoverTheme | Setting::AsciiCover | Setting::Transparent | Setting::Mouse
        )
    }
}

/// The values of `window_title`, in the order the settings menu steps through them.
pub(super) const WINDOW_TITLES: [&str; 3] = ["off", "title", "artist-title"];

pub const LANGS: &[&str] = &["en", "sv", "de", "fr", "es", "it", "pt", "nl", "ja", "ko", "zh-CN", "ru", "ar"];
