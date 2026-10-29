//! The window's translation requests: one abortable task per (song, language), the same
//! disk cache, never-translate list and hold-and-retry as the terminal front end, and the
//! lyrics title that says what is going on. The state machine (`Tr`) has no egui or network
//! types in it, so it is unit-tested.
use super::model::Model;
use music_display::cli::{TranslateEngine, TranslateMode};
use music_display::{AppEvent, ai, translate};
use std::time::Duration;
use tokio::task::JoinHandle;

/// Language check kept failing for a song on a never-translate list: asked again this often,
/// this many times.
pub const HOLD_RETRY_SECS: u64 = 20;
pub const HOLD_RETRIES: u32 = 6;
/// Start of the backend name when the AI failed and the standard engine answered instead.
pub const FALLBACK_TAG: &str = "fallback: ";

#[derive(Clone, Debug, PartialEq)]
pub enum TrState {
    Idle,
    Loading,
    /// Why there is no translation (rate limit, never-translate language, held…).
    Failed(String),
}

pub struct Tr {
    pub state: TrState,
    /// The (song, language) being fetched; a held request stays in flight while it retries.
    pub inflight: Option<(u64, String)>,
    pub task: Option<JoinHandle<()>>,
    /// The engine the translations on hand were made with.
    engine: Option<TranslateEngine>,
}

impl Default for Tr {
    fn default() -> Self {
        Tr { state: TrState::Idle, inflight: None, task: None, engine: None }
    }
}

impl Tr {
    /// Forget the request (the song, the lines or the settings changed) and stop its task.
    pub fn reset(&mut self) {
        self.state = TrState::Idle;
        self.inflight = None;
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }

    /// Should a request for `want` start? Not when the translation is there, there is nothing to
    /// translate, the song's language is on the never-translate list, or it is already asked for.
    pub fn wants(&self, have: bool, has_lines: bool, want: &(u64, String)) -> bool {
        if have || !has_lines {
            return false;
        }
        if matches!(&self.state, TrState::Failed(e) if translate::is_skipped(e)) {
            return false;
        }
        self.inflight.as_ref() != Some(want)
    }

    /// A request for `want` is going out as `task`; an older one is stopped.
    pub fn started(&mut self, want: (u64, String), task: JoinHandle<()>) {
        if let Some(t) = self.task.replace(task) {
            t.abort();
        }
        self.inflight = Some(want);
        self.state = TrState::Loading;
    }

    /// An answer for (`key`, `lang`) came while (`cur`) is wanted. `err` is the failure text;
    /// `have` says the wanted translation is on hand now.
    pub fn settle(&mut self, key: u64, lang: &str, cur: &(u64, String), err: Option<&str>, have: bool) {
        let mine = key == cur.0 && lang == cur.1;
        if let (Some(e), true) = (err, mine) {
            self.state = TrState::Failed(e.to_string());
        }
        // a held song keeps its request: the task retries the language check itself
        let held = err.is_some_and(translate::is_held);
        if !held && self.inflight.as_ref().is_some_and(|(k, l)| *k == key && l == lang) {
            self.inflight = None;
        }
        if mine && have {
            self.state = TrState::Idle;
        }
    }

    /// The engine changed since the translations on hand were made: they are stale.
    fn engine_changed(&mut self, now: TranslateEngine) -> bool {
        let changed = self.engine.is_some_and(|e| e != now);
        self.engine = Some(now);
        changed
    }
}

/// What the lyrics title needs to know.
pub struct TitleIn<'a> {
    pub browsing: bool,
    pub mode: TranslateMode,
    /// A translation for these very lines is on hand.
    pub shown: bool,
    pub state: &'a TrState,
    pub backend: &'a str,
    pub ai_model: &'a str,
    pub lang: &'a str,
}

/// The lyrics panel's heading: browsing hint, translation backend, "translating…" or why not.
pub fn title(i: &TitleIn) -> String {
    if i.browsing {
        return "Lyrics · browsing — End to follow".into();
    }
    if i.mode == TranslateMode::Off {
        return "Lyrics".into();
    }
    if i.shown {
        return if i.backend.starts_with("AI ") {
            format!("Lyrics · translated by {} ({})", i.ai_model, i.lang)
        } else if let Some(b) = i.backend.strip_prefix(FALLBACK_TAG) {
            format!("Lyrics · translated ({}, {b})", i.lang)
        } else if !i.backend.is_empty() && i.backend != "google" {
            format!("Lyrics · translated ({}, {})", i.lang, i.backend)
        } else if i.mode == TranslateMode::Both {
            format!("Lyrics · original + translated ({})", i.lang)
        } else {
            format!("Lyrics · translated ({})", i.lang)
        };
    }
    match i.state {
        TrState::Failed(e) => format!("Lyrics · {e}"),
        _ => "Lyrics · translating…".into(),
    }
}

impl Model {
    /// The `translate_skip` codes, normalised (`SV, zh-CN` → `["sv", "zh"]`).
    pub fn skip_langs(&self) -> Vec<String> {
        self.args.translate_skip.split(',').map(translate::base_code).filter(|c| !c.is_empty()).collect()
    }

    /// Everything translated is stale (engine, model or lyrics changed): start over.
    #[allow(dead_code)] // for a settings change that makes the translations stale (model, skip list)
    pub fn reset_translation(&mut self) {
        self.translations.clear();
        self.tr.reset();
        self.ensure_translation();
    }

    /// Start fetching the translation for this song unless we have it (or are asking for it).
    /// Changing the language or the engine drops the old request and asks again.
    pub fn ensure_translation(&mut self) {
        if self.translate_mode == TranslateMode::Off {
            return;
        }
        if self.tr.engine_changed(self.args.translate_engine) {
            self.translations.clear();
            self.tr.reset();
        }
        let have = self.translation().is_some();
        let lines = self.lines().filter(|l| !l.is_empty());
        let want = (self.track_key, self.args.translate_lang.clone());
        if !self.tr.wants(have, lines.is_some(), &want) {
            return;
        }
        let Some(lines) = lines else { return };
        // the one-time question comes before lyrics go to an AI provider (privacy.rs)
        if self.args.translate_engine == TranslateEngine::Ai && self.ai_blocked() {
            return;
        }
        let (c, tx, key, lang) = (self.client.clone(), self.tx.clone(), self.track_key, self.args.translate_lang.clone());
        let skip = self.skip_langs();
        let cfg = translate::Cfg {
            google_base: None,
            fallback: self.args.translate_fallback,
            fallback_url: self.args.translate_fallback_url.clone(),
            fallback_key: self.args.translate_fallback_key.clone(),
            skip: skip.clone(),
        };
        let ai = self.ai_job(&lines);
        // (the tests must not read or fill the real cache folder)
        let keep = !self.args.no_cache_translations && !music_display::util::testing();
        let task = tokio::spawn(async move {
            // the language model first: one request for the whole song; if it cannot deliver,
            // the standard engine takes over and the title says so
            let mut ai_failed: Option<String> = None;
            if let Some((acfg, song)) = ai {
                if keep {
                    if let Some(done) = ai::cached(&acfg, &song).await {
                        let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, ai::label(&acfg))) });
                        return;
                    }
                }
                match ai::translate(&c, &acfg, &song).await {
                    Ok((done, label)) => {
                        if keep {
                            ai::store(&acfg, &song, &done).await;
                        }
                        let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, label)) });
                        return;
                    }
                    Err(why) => ai_failed = Some(why),
                }
            }
            let tag = |backend: &str| match &ai_failed {
                Some(why) => format!("{FALLBACK_TAG}{backend}; AI failed: {why}"),
                None => backend.to_string(),
            };
            if keep && ai_failed.is_none() {
                if let Some((done, backend)) = translate::cached(&lang, &skip, &lines).await {
                    let _ = tx.send(AppEvent::Translated { key, lang, lines: Ok((done, tag(backend))) });
                    return;
                }
            }
            // language not detectable (rate-limited) while the never-translate list is in use:
            // say so once, then ask again every HOLD_RETRY_SECS
            let mut told = false;
            for attempt in 1..=HOLD_RETRIES {
                let mut r = translate::lines(&c, &cfg, &lang, &lines).await;
                if r.as_ref().is_err_and(|e| translate::is_held(e)) {
                    if attempt < HOLD_RETRIES {
                        if !told {
                            told = true;
                            let held = Err(translate::HELD.to_string());
                            let _ = tx.send(AppEvent::Translated { key, lang: lang.clone(), lines: held });
                        }
                        tokio::time::sleep(Duration::from_secs(HOLD_RETRY_SECS)).await;
                        continue;
                    }
                    r = Err(translate::HELD_FINAL.to_string());
                }
                if let (true, Ok((done, backend))) = (keep, &r) {
                    translate::store(&lang, &skip, &lines, done, backend).await;
                }
                let _ = tx.send(AppEvent::Translated { key, lang, lines: r.map(|(l, b)| (l, tag(b))) });
                return;
            }
        });
        self.tr.started(want, task);
    }

    /// An answer from the translation task.
    pub fn on_translated(&mut self, key: u64, lang: String, lines: Result<(Vec<String>, String), String>) {
        let err = lines.as_ref().err().cloned();
        if let Ok((l, backend)) = lines {
            // (made from the lines of this song as they were asked for; a lyrics change since
            // dropped the request, so an answer that still arrives is for the current lines)
            if key == self.track_key {
                self.translations.insert((key, lang.clone()), l);
                self.translate_backend = backend;
            }
        }
        let cur = (self.track_key, self.args.translate_lang.clone());
        let have = self.translations.contains_key(&cur);
        self.tr.settle(key, &lang, &cur, err.as_deref(), have);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn want(k: u64, l: &str) -> (u64, String) {
        (k, l.to_string())
    }

    #[test]
    fn a_request_starts_once_per_song_and_language() {
        let mut t = Tr::default();
        let w = want(1, "en");
        assert!(t.wants(false, true, &w));
        assert!(!t.wants(true, true, &w), "already translated");
        assert!(!t.wants(false, false, &w), "nothing to translate");
        t.inflight = Some(w.clone());
        assert!(!t.wants(false, true, &w), "already asked");
        assert!(t.wants(false, true, &want(1, "sv")), "another language asks again");
        assert!(t.wants(false, true, &want(2, "en")), "another song asks again");
    }

    #[test]
    fn a_skipped_language_settles_until_reset() {
        let mut t = Tr::default();
        let w = want(1, "en");
        t.inflight = Some(w.clone());
        t.settle(1, "en", &w, Some("never translated (sv)"), false);
        assert!(t.inflight.is_none() && matches!(&t.state, TrState::Failed(e) if translate::is_skipped(e)));
        assert!(!t.wants(false, true, &w), "no request loop for a skipped song");
        t.reset();
        assert!(t.wants(false, true, &w));
    }

    #[test]
    fn a_held_request_stays_in_flight_and_clears_when_answered() {
        let mut t = Tr::default();
        let w = want(1, "en");
        t.inflight = Some(w.clone());
        t.settle(1, "en", &w, Some(translate::HELD), false);
        assert_eq!(t.inflight, Some(w.clone()));
        assert_eq!(t.state, TrState::Failed(translate::HELD.into()));
        t.settle(1, "en", &w, None, true);
        assert!(t.inflight.is_none() && t.state == TrState::Idle);
    }

    #[test]
    fn an_answer_for_another_song_or_language_does_not_touch_the_state() {
        let mut t = Tr::default();
        let cur = want(2, "en");
        t.inflight = Some(cur.clone());
        t.state = TrState::Loading;
        t.settle(1, "en", &cur, Some("boom"), false);
        t.settle(2, "sv", &cur, Some("boom"), false);
        assert_eq!(t.state, TrState::Loading);
        assert_eq!(t.inflight, Some(cur));
    }

    #[tokio::test]
    async fn starting_a_new_request_aborts_the_old_task() {
        let mut t = Tr::default();
        let first = tokio::spawn(std::future::pending::<()>());
        let probe = first.abort_handle();
        t.started(want(1, "en"), first);
        assert_eq!(t.state, TrState::Loading);
        t.started(want(1, "sv"), tokio::spawn(std::future::pending::<()>()));
        tokio::task::yield_now().await;
        assert!(probe.is_finished(), "the old task was aborted");
        t.reset();
        assert!(t.task.is_none() && t.inflight.is_none());
    }

    #[test]
    fn the_engine_change_is_seen_once() {
        let mut t = Tr::default();
        assert!(!t.engine_changed(TranslateEngine::Standard), "first look is not a change");
        assert!(!t.engine_changed(TranslateEngine::Standard));
        assert!(t.engine_changed(TranslateEngine::Ai));
        assert!(!t.engine_changed(TranslateEngine::Ai));
    }

    #[test]
    fn the_title_says_what_the_translation_is_doing() {
        let idle = TrState::Idle;
        let mut i = TitleIn { browsing: false, mode: TranslateMode::Translated, shown: false, state: &idle, backend: "google", ai_model: "m", lang: "en" };
        assert_eq!(title(&i), "Lyrics · translating…");
        i.shown = true;
        assert_eq!(title(&i), "Lyrics · translated (en)");
        i.mode = TranslateMode::Both;
        assert_eq!(title(&i), "Lyrics · original + translated (en)");
        i.backend = "AI llama";
        assert_eq!(title(&i), "Lyrics · translated by m (en)");
        i.backend = "fallback: libre; AI failed: x";
        assert_eq!(title(&i), "Lyrics · translated (en, libre; AI failed: x)");
        i.backend = "libre";
        assert_eq!(title(&i), "Lyrics · translated (en, libre)");
        let failed = TrState::Failed("rate limited".into());
        i.shown = false;
        i.state = &failed;
        assert_eq!(title(&i), "Lyrics · rate limited");
        i.mode = TranslateMode::Off;
        assert_eq!(title(&i), "Lyrics");
        i.browsing = true;
        assert!(title(&i).contains("browsing"));
    }
}
