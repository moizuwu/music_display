//! The annotations panel (experimental): what the active lyric line means. Notes come from
//! Genius (free client token), from the AI engine, or from both (genius+ai: Genius first, then
//! one AI request that adds what Genius does not say); they are fetched in a task, tagged with
//! the track, and matched to the lyric lines here.
use super::*;
use crate::annotations::{self, Annotation};
use crate::cli::AnnotationMode;
use crate::layout::PanelId;

pub enum AnnState {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

impl App {
    pub fn annotations_shown(&self) -> bool {
        self.layout.panels.shown(PanelId::Annotations)
    }

    /// The Genius token from the config / flag, else `GENIUS_TOKEN`.
    pub fn genius_token(&self) -> Option<String> {
        annotations::token(&self.args.genius_token)
    }

    /// Index of the lyric line being sung (synced lyrics only).
    pub fn active_line(&self) -> Option<usize> {
        match &self.lyrics {
            LyricsState::Ready(Lyrics::Synced(l)) => {
                let t = self.position() + self.offset;
                l.partition_point(|(ts, _)| *ts <= t).checked_sub(1)
            }
            _ => None,
        }
    }

    /// The notes for lyric line `i`, Genius first: text, the quoted fragment (Genius) and who
    /// wrote it.
    pub fn notes_for(&self, i: usize) -> Vec<Annotation> {
        annotations::notes_for(self.args.annotations_mode, &self.anns, &self.ann_match, &self.ai_notes, i)
    }

    /// Fetch the notes for this track once, while the panel is on and the lyrics are there.
    pub fn ensure_annotations(&mut self) {
        if !self.annotations_shown() || !matches!(self.ann_state, AnnState::Idle) {
            return;
        }
        let Some(lines) = self.lyric_lines().filter(|l| !l.is_empty()) else { return };
        let Some(snap) = self.snap.clone() else { return };
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        if self.args.annotations_mode == AnnotationMode::GeniusAi {
            return self.ensure_genius_ai(lines, snap);
        }
        match self.args.annotations_mode {
            AnnotationMode::Genius => {
                let Some(token) = self.genius_token() else {
                    self.ann_state = AnnState::Failed(annotations::NO_TOKEN.into());
                    return;
                };
                self.ann_state = AnnState::Loading;
                let base = self.annotations_base.clone();
                self.tasks.push(tokio::spawn(async move {
                    let base = base.unwrap_or_else(|| annotations::API.to_string());
                    let result = annotations::fetch_from(&c, &base, &token, &snap.artist, &snap.title).await;
                    let _ = tx.send(AppEvent::Annotations { key, result });
                }));
            }
            AnnotationMode::Ai => {
                let Some(cfg) = self.ai_cfg() else {
                    self.ann_state = AnnState::Failed("AI notes need a model (settings → AI model)".into());
                    return;
                };
                if self.ai_notice_needed() {
                    self.set_note(self.ai_notice());
                    self.ack_ai_notice();
                }
                self.ann_state = AnnState::Loading;
                self.tasks.push(tokio::spawn(async move {
                    let result = annotations::ai_notes(&c, &cfg, &snap.artist, &snap.title, &lines).await;
                    let _ = tx.send(AppEvent::AiNotes { key, result });
                }));
            }
            AnnotationMode::GeniusAi => {}
        }
    }

    /// genius+ai: one task, Genius first (shown as soon as it arrives), then one AI request that
    /// sees the Genius notes. Without a token or a Genius answer the AI still runs; without a
    /// model the Genius notes still show.
    fn ensure_genius_ai(&mut self, lines: Vec<String>, snap: crate::player::Snapshot) {
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        let token = self.genius_token();
        let cfg = self.ai_cfg();
        if cfg.is_some() && self.ai_notice_needed() {
            self.set_note(self.ai_notice());
            self.ack_ai_notice();
        }
        self.ann_state = AnnState::Loading;
        self.ai_state = match cfg {
            Some(_) => AnnState::Loading,
            None => AnnState::Failed("AI extras need a model (settings → AI model)".into()),
        };
        let base = self.annotations_base.clone().unwrap_or_else(|| annotations::API.to_string());
        self.tasks.push(tokio::spawn(async move {
            let genius = match token {
                Some(t) => annotations::fetch_from(&c, &base, &t, &snap.artist, &snap.title).await,
                None => Err(annotations::NO_TOKEN.to_string()),
            };
            let _ = tx.send(AppEvent::Annotations { key, result: genius.clone() });
            let Some(cfg) = cfg else { return };
            let per_line = annotations::genius_per_line(&lines, &genius.unwrap_or_default());
            let result = annotations::ai_extras(&c, &cfg, &snap.artist, &snap.title, &lines, &per_line).await;
            let _ = tx.send(AppEvent::AiNotes { key, result });
        }));
    }

    pub(super) fn reset_annotations(&mut self) {
        self.ann_state = AnnState::Idle;
        self.ai_state = AnnState::Idle;
        self.genius_err = None;
        self.anns.clear();
        self.ann_match.clear();
        self.ai_notes.clear();
        self.ann_scroll = 0;
    }

    /// Match the Genius fragments to the current lyric lines.
    pub fn rematch_annotations(&mut self) {
        self.ann_match = match self.lyric_lines() {
            Some(lines) => annotations::match_lines(&lines, &self.anns),
            None => Vec::new(),
        };
    }

    pub(super) fn on_annotations(&mut self, result: Result<Vec<Annotation>, String>) {
        let both = self.args.annotations_mode == AnnotationMode::GeniusAi;
        match result {
            Ok(a) => {
                self.anns = a;
                self.rematch_annotations();
                self.ann_state = AnnState::Ready;
            }
            // genius+ai: the AI part still comes; the Genius problem is a line in the panel
            Err(e) if both => {
                self.genius_err = Some(e);
                self.ann_state = AnnState::Ready;
            }
            Err(e) => self.ann_state = AnnState::Failed(e),
        }
        self.dirty = true;
    }

    pub(super) fn on_ai_notes(&mut self, result: Result<Vec<Option<String>>, String>) {
        let state = if self.args.annotations_mode == AnnotationMode::GeniusAi { &mut self.ai_state } else { &mut self.ann_state };
        match result {
            Ok(n) => {
                self.ai_notes = n;
                *state = AnnState::Ready;
            }
            Err(e) => *state = AnnState::Failed(e),
        }
        self.dirty = true;
    }

    /// `e` / `E` and the wheel over the panel: scroll a note that does not fit (the panel
    /// clamps it when drawing).
    pub fn scroll_notes(&mut self, by: i32) {
        self.ann_scroll = (self.ann_scroll as i32 + by).max(0) as u16;
        self.dirty = true;
    }

    /// `N`: show / hide the panel.
    pub fn toggle_annotations(&mut self) {
        let on = !self.annotations_shown();
        self.layout.panels.set(PanelId::Annotations, on);
        self.args.annotations = on;
        self.snap_scroll = true;
        if on {
            self.ensure_annotations();
        }
    }

    /// The mode or the token changed: start over for this song.
    pub fn restart_annotations(&mut self) {
        self.reset_annotations();
        self.ensure_annotations();
    }
}

